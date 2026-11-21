//! Windows backends for Elo: screen capture via DXGI Desktop Duplication and
//! input injection via `SendInput`.
//!
//! Design notes:
//! * All D3D11/DXGI objects live on one dedicated OS thread owned by
//!   [`DxgiCapture`]. The async side sends requests over a std channel and
//!   awaits a tokio oneshot reply, so no COM object ever crosses threads and
//!   no tokio worker blocks on DXGI.
//! * Both capture and input follow the *input desktop* (Default <-> Winlogon)
//!   via [`ensure_input_desktop`], so a SYSTEM process in the console session
//!   keeps working on the lock screen and UAC prompts.
//! * Every Win32 call is `unsafe`; each unsafe block is small and documented.

use std::cell::RefCell;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::{Capture, InputSink};
use elo_protocol::{InputEvent, MonitorInfo, MouseButton};

use windows::core::{s, w, Interface};
use windows::Win32::Foundation::{FreeLibrary, BOOL, HMODULE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop,
    DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, HDESK, UOI_NAME,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, MONITORINFOF_PRIMARY, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// One mouse-wheel notch, per Win32 (`WHEEL_DELTA`).
const WHEEL_DELTA: i32 = 120;

/// `GENERIC_ALL` access for `OpenInputDesktop`.
const DESKTOP_GENERIC_ALL: DESKTOP_ACCESS_FLAGS = DESKTOP_ACCESS_FLAGS(0x1000_0000);

// ===========================================================================
// Input desktop tracking
// ===========================================================================

thread_local! {
    /// The desktop this thread is currently attached to (handle + name), as
    /// set by [`ensure_input_desktop`]. Per thread because `SetThreadDesktop`
    /// is per thread.
    static CURRENT_DESKTOP: RefCell<Option<(HDESK, String)>> = const { RefCell::new(None) };
}

/// Read a desktop's name (e.g. `Default`, `Winlogon`).
fn desktop_name(desk: HDESK) -> Option<String> {
    let mut buf = [0u16; 256];
    let mut needed = 0u32;
    // SAFETY: `buf` is a writable buffer of the size we pass (in bytes); the
    // handle is a live desktop handle owned by the caller.
    unsafe {
        GetUserObjectInformationW(
            windows::Win32::Foundation::HANDLE(desk.0),
            UOI_NAME,
            Some(buf.as_mut_ptr().cast()),
            (buf.len() * 2) as u32,
            Some(&mut needed),
        )
        .ok()?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Attach the calling thread to the current *input* desktop if it changed.
///
/// Returns `true` when the thread switched desktops (e.g. Default <->
/// Winlogon on lock screen / UAC), so callers can rebuild desktop-bound state.
/// Any failure (insufficient privilege, thread owns windows/hooks) is ignored
/// and reported as `false`.
pub fn ensure_input_desktop() -> bool {
    // SAFETY: OpenInputDesktop returns an owned handle or an error.
    let new = match unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_GENERIC_ALL) } {
        Ok(h) => h,
        Err(_) => return false,
    };
    let Some(name) = desktop_name(new) else {
        // SAFETY: `new` is our own handle, not attached to any thread.
        unsafe {
            let _ = CloseDesktop(new);
        }
        return false;
    };
    CURRENT_DESKTOP.with(|cur| {
        let mut cur = cur.borrow_mut();
        let same = matches!(&*cur, Some((_, n)) if *n == name);
        if same {
            // SAFETY: `new` is our own, unused handle.
            unsafe {
                let _ = CloseDesktop(new);
            }
            return false;
        }
        // SAFETY: switch this thread to `new`; fails if the thread owns
        // windows or hooks, in which case we keep the old desktop.
        if unsafe { SetThreadDesktop(new) }.is_err() {
            unsafe {
                let _ = CloseDesktop(new);
            }
            return false;
        }
        if let Some((old, _)) = cur.take() {
            // SAFETY: `old` is no longer this thread's desktop, so closing our
            // handle to it is allowed.
            unsafe {
                let _ = CloseDesktop(old);
            }
        }
        *cur = Some((new, name));
        true
    })
}

// ===========================================================================
// Output enumeration
// ===========================================================================

/// One DXGI output plus the adapter it hangs off.
struct OutputEntry {
    adapter: IDXGIAdapter1,
    output: IDXGIOutput1,
    info: MonitorInfo,
}

/// Enumerate every desktop-attached output of every adapter, in a stable
/// (adapter, output) order. `current` is left false.
fn enumerate_outputs() -> Result<Vec<OutputEntry>> {
    // SAFETY: CreateDXGIFactory1 returns an owned COM interface or an error.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.context("CreateDXGIFactory1 failed")?;
    let mut list = Vec::new();
    let mut a = 0u32;
    // SAFETY: EnumAdapters1 errors (DXGI_ERROR_NOT_FOUND) past the last one.
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(a) } {
        a += 1;
        let mut o = 0u32;
        // SAFETY: EnumOutputs errors past the last output.
        while let Ok(output) = unsafe { adapter.EnumOutputs(o) } {
            o += 1;
            // SAFETY: GetDesc fills a POD struct.
            let Ok(desc) = (unsafe { output.GetDesc() }) else { continue };
            if !desc.AttachedToDesktop.as_bool() {
                continue;
            }
            let Ok(output1) = output.cast::<IDXGIOutput1>() else { continue };
            let r = desc.DesktopCoordinates;
            let nlen = desc.DeviceName.iter().position(|&c| c == 0).unwrap_or(desc.DeviceName.len());
            let name = String::from_utf16_lossy(&desc.DeviceName[..nlen]);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            // SAFETY: `mi` is a properly sized MONITORINFO; the HMONITOR comes
            // from DXGI. On failure we fall back to the origin heuristic.
            let primary = if unsafe { GetMonitorInfoW(desc.Monitor, &mut mi) }.as_bool() {
                mi.dwFlags & MONITORINFOF_PRIMARY != 0
            } else {
                r.left == 0 && r.top == 0
            };
            list.push(OutputEntry {
                adapter: adapter.clone(),
                output: output1,
                info: MonitorInfo {
                    index: list.len() as u32,
                    name,
                    x: r.left,
                    y: r.top,
                    width: (r.right - r.left).max(0) as u32,
                    height: (r.bottom - r.top).max(0) as u32,
                    primary,
                    current: false,
                },
            });
        }
    }
    if list.is_empty() {
        bail!("no desktop-attached display outputs found");
    }
    Ok(list)
}

// ===========================================================================
// Capture thread state
// ===========================================================================

/// Geometry reported to the async side after each (re)open.
#[derive(Clone)]
struct Geometry {
    width: u32,
    height: u32,
    monitors: Vec<MonitorInfo>,
}

/// An open duplication of one output.
struct Duplication {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dupl: IDXGIOutputDuplication,
    staging: Option<ID3D11Texture2D>,
    frame_held: bool,
}

impl Drop for Duplication {
    fn drop(&mut self) {
        if self.frame_held {
            // SAFETY: balance a frame still held by the duplication.
            unsafe {
                let _ = self.dupl.ReleaseFrame();
            }
        }
    }
}

/// Everything the capture thread owns. Never leaves that thread.
struct Grabber {
    /// Device name of the monitor we capture (stable across re-enumeration).
    current_name: String,
    dup: Option<Duplication>,
    width: u32,
    height: u32,
    monitors: Vec<MonitorInfo>,
    /// Last successfully captured frame (compact BGRA, `width*height*4`).
    last: Vec<u8>,
    /// Set when geometry changed and has not been reported yet.
    geometry_dirty: bool,
}

impl Grabber {
    /// Open monitor `index`, or (if `None`) the monitor named
    /// `current_name`, falling back to the primary one.
    fn open(&mut self, index: Option<u32>) -> Result<()> {
        // Bind to the input desktop first so the duplication is created on it.
        ensure_input_desktop();
        self.dup = None;
        let outputs = enumerate_outputs()?;
        let pos = match index {
            Some(i) => {
                let i = i as usize;
                if i >= outputs.len() {
                    bail!("monitor {i} does not exist ({} available)", outputs.len());
                }
                i
            }
            None => outputs
                .iter()
                .position(|o| !self.current_name.is_empty() && o.info.name == self.current_name)
                .or_else(|| outputs.iter().position(|o| o.info.primary))
                .unwrap_or(0),
        };
        let entry = &outputs[pos];

        // SAFETY: device creation on a specific adapter requires
        // D3D_DRIVER_TYPE_UNKNOWN. Out-params are owned Option slots.
        let (device, context) = unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                &entry.adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("D3D11CreateDevice failed")?;
            (
                device.context("D3D11CreateDevice returned no device")?,
                context.context("D3D11CreateDevice returned no context")?,
            )
        };
        // SAFETY: DuplicateOutput with a device created on the output's adapter.
        let dupl = unsafe { entry.output.DuplicateOutput(&device) }.context("DuplicateOutput failed")?;
        // SAFETY: GetDesc on a live duplication fills a POD struct.
        let mode = unsafe { dupl.GetDesc() }.ModeDesc;
        let (width, height) = if mode.Width > 0 && mode.Height > 0 {
            (mode.Width, mode.Height)
        } else {
            (entry.info.width, entry.info.height)
        };
        if width == 0 || height == 0 {
            bail!("output {} reported zero size", entry.info.name);
        }

        self.current_name = entry.info.name.clone();
        self.monitors = outputs
            .iter()
            .map(|o| MonitorInfo { current: o.info.index as usize == pos, ..o.info.clone() })
            .collect();
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.last = vec![0u8; width as usize * height as usize * 4];
        }
        self.dup = Some(Duplication { device, context, dupl, staging: None, frame_held: false });
        self.geometry_dirty = true;
        Ok(())
    }

    fn geometry(&self) -> Geometry {
        Geometry { width: self.width, height: self.height, monitors: self.monitors.clone() }
    }

    /// Produce one frame. After the initial open this never fails: transient
    /// problems (desktop switch, access lost, secure desktop without
    /// privilege) yield the last good frame and a reopen attempt next time.
    fn frame(&mut self) -> Vec<u8> {
        if ensure_input_desktop() {
            // Default <-> Winlogon switch: the duplication is bound to the old
            // desktop. Recreate it on the new one.
            self.dup = None;
        }
        if self.dup.is_none() && self.open(None).is_err() {
            // Avoid a hot loop while the output is unavailable.
            std::thread::sleep(Duration::from_millis(50));
            return self.last.clone();
        }
        match self.grab() {
            Ok(_) => {}
            Err(_) => {
                // Access lost or device error: rebuild on the next request.
                self.dup = None;
            }
        }
        self.last.clone()
    }

    /// Acquire + copy + map one frame into `self.last`. `Ok(false)` is a
    /// benign timeout (no screen change).
    fn grab(&mut self) -> Result<bool> {
        let (width, height) = (self.width as usize, self.height as usize);
        let dup = self.dup.as_mut().context("no duplication")?;
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;

        // SAFETY: AcquireNextFrame fills `info`/`resource`; every success must
        // be balanced by ReleaseFrame (done below / in Drop).
        if let Err(e) = unsafe { dup.dupl.AcquireNextFrame(16, &mut info, &mut resource) } {
            if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                return Ok(false);
            }
            if e.code() == DXGI_ERROR_ACCESS_LOST {
                bail!("duplication access lost");
            }
            return Err(e).context("AcquireNextFrame failed");
        }
        dup.frame_held = true;

        let last = &mut self.last;
        let result = (|| -> Result<()> {
            let resource = resource.context("AcquireNextFrame returned no resource")?;
            let tex: ID3D11Texture2D = resource.cast().context("desktop resource is not a texture")?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: GetDesc fills a POD struct.
            unsafe { tex.GetDesc(&mut desc) };

            // (Re)create the staging texture when missing or the size changed.
            let reuse = dup.staging.as_ref().is_some_and(|s| {
                let mut sd = D3D11_TEXTURE2D_DESC::default();
                // SAFETY: GetDesc fills a POD struct.
                unsafe { s.GetDesc(&mut sd) };
                sd.Width == desc.Width && sd.Height == desc.Height && sd.Format == desc.Format
            });
            if !reuse {
                let mut sdesc = desc;
                sdesc.Usage = D3D11_USAGE_STAGING;
                sdesc.BindFlags = 0;
                sdesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                sdesc.MiscFlags = 0;
                let mut staging: Option<ID3D11Texture2D> = None;
                // SAFETY: CreateTexture2D validates the descriptor.
                unsafe { dup.device.CreateTexture2D(&sdesc, None, Some(&mut staging)) }
                    .context("CreateTexture2D (staging) failed")?;
                dup.staging = Some(staging.context("CreateTexture2D returned no texture")?);
            }
            let staging = dup.staging.as_ref().context("no staging texture")?;

            // Copy only what both the texture and our buffer can hold (they
            // differ only transiently, e.g. on a rotated output).
            let rows = height.min(desc.Height as usize);
            let row_bytes = width.min(desc.Width as usize) * 4;
            let dst_stride = width * 4;

            // SAFETY: CopyResource + Map/Unmap on our own staging texture. The
            // mapped pointer is valid only between Map and Unmap, which
            // bracket the bounded row copy below.
            unsafe {
                dup.context.CopyResource(staging, &tex);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                dup.context
                    .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .context("Map(staging) failed")?;
                let pitch = mapped.RowPitch as usize;
                let src = mapped.pData as *const u8;
                for y in 0..rows {
                    let row = std::slice::from_raw_parts(src.add(y * pitch), row_bytes);
                    last[y * dst_stride..y * dst_stride + row_bytes].copy_from_slice(row);
                }
                dup.context.Unmap(staging, 0);
            }
            Ok(())
        })();

        // SAFETY: release the frame we acquired, regardless of copy success.
        unsafe {
            let _ = dup.dupl.ReleaseFrame();
        }
        dup.frame_held = false;
        result?;
        Ok(true)
    }
}

/// Requests from the async side to the capture thread.
enum Request {
    Frame(oneshot::Sender<(Vec<u8>, Option<Geometry>)>),
    Select(u32, oneshot::Sender<Result<Geometry>>),
}

/// Body of the capture thread: open the primary output, report the result,
/// then serve requests until the owning [`DxgiCapture`] is dropped.
fn capture_thread(init: std_mpsc::Sender<Result<Geometry>>, rx: std_mpsc::Receiver<Request>) {
    let mut g = Grabber {
        current_name: String::new(),
        dup: None,
        width: 0,
        height: 0,
        monitors: Vec::new(),
        last: Vec::new(),
        geometry_dirty: false,
    };
    match g.open(None) {
        Ok(()) => {
            g.geometry_dirty = false;
            let _ = init.send(Ok(g.geometry()));
        }
        Err(e) => {
            let _ = init.send(Err(e));
            return;
        }
    }
    drop(init);

    while let Ok(req) = rx.recv() {
        match req {
            Request::Frame(reply) => {
                let buf = g.frame();
                let geom = std::mem::take(&mut g.geometry_dirty).then(|| g.geometry());
                let _ = reply.send((buf, geom));
            }
            Request::Select(index, reply) => {
                let res = g.open(Some(index)).map(|_| {
                    g.geometry_dirty = false;
                    g.geometry()
                });
                let _ = reply.send(res);
            }
        }
    }
}

// ===========================================================================
// Screen capture (async facade)
// ===========================================================================

/// Captures one monitor with DXGI Desktop Duplication on a dedicated thread.
/// Defaults to the primary monitor; see [`Capture::select_monitor`].
pub struct DxgiCapture {
    tx: std_mpsc::Sender<Request>,
    width: u32,
    height: u32,
    monitors: Vec<MonitorInfo>,
}

impl DxgiCapture {
    /// Start the capture thread and open the primary output.
    pub fn new() -> Result<Self> {
        let (tx, rx) = std_mpsc::channel();
        let (init_tx, init_rx) = std_mpsc::channel();
        std::thread::Builder::new()
            .name("elo-dxgi-capture".into())
            .spawn(move || capture_thread(init_tx, rx))
            .context("spawning capture thread failed")?;
        let geom = init_rx
            .recv()
            .map_err(|_| anyhow!("capture thread exited during startup"))??;
        Ok(Self { tx, width: geom.width, height: geom.height, monitors: geom.monitors })
    }

    fn apply(&mut self, g: Geometry) {
        self.width = g.width;
        self.height = g.height;
        self.monitors = g.monitors;
    }
}

#[async_trait]
impl Capture for DxgiCapture {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    async fn frame(&mut self) -> Result<Vec<u8>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Request::Frame(reply))
            .map_err(|_| anyhow!("capture thread is gone"))?;
        let (buf, geom) = rx.await.map_err(|_| anyhow!("capture thread dropped the request"))?;
        if let Some(g) = geom {
            // Resolution / desktop change: report the new size with this frame.
            self.apply(g);
        }
        Ok(buf)
    }
    fn monitors(&self) -> Vec<MonitorInfo> {
        self.monitors.clone()
    }
    async fn select_monitor(&mut self, index: u32) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Request::Select(index, reply))
            .map_err(|_| anyhow!("capture thread is gone"))?;
        let geom = rx.await.map_err(|_| anyhow!("capture thread dropped the request"))??;
        self.apply(geom);
        Ok(())
    }
}

// Dropping `DxgiCapture` drops `tx`; the capture thread's `recv()` then fails
// and the thread exits, releasing all D3D/DXGI objects on that same thread.

// ===========================================================================
// Input injection
// ===========================================================================

/// Injects remote input events into the local session via `SendInput`.
/// Mouse coordinates are relative to the captured monitor (`set_area`) and
/// are mapped onto the whole virtual desktop.
pub struct SendInputSink {
    area_x: i32,
    area_y: i32,
    area_w: u32,
    area_h: u32,
}

impl SendInputSink {
    pub fn new(width: u32, height: u32) -> Self {
        Self { area_x: 0, area_y: 0, area_w: width.max(1), area_h: height.max(1) }
    }

    /// Send a single prepared `INPUT` on the current input desktop.
    fn send(&self, input: INPUT) -> Result<()> {
        // Follow the secure desktop (lock screen / UAC) when privileged.
        ensure_input_desktop();
        // SAFETY: `input` is a fully-initialized INPUT; cbsize matches. The
        // array lives for the duration of the call.
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if sent == 1 {
            Ok(())
        } else {
            bail!("SendInput injected {sent}/1 events (blocked by UIPI?)");
        }
    }

    fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    /// Map a monitor-relative point to SendInput's 0..=65535 virtual-desktop
    /// coordinates.
    fn to_virtual(&self, x: i32, y: i32) -> (i32, i32) {
        let x = x.clamp(0, self.area_w as i32 - 1);
        let y = y.clamp(0, self.area_h as i32 - 1);
        // SAFETY: GetSystemMetrics has no preconditions.
        let (vx, vy, vw, vh) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        // Fall back to the area itself if the metrics are unavailable.
        let (vx, vy, vw, vh) = if vw > 0 && vh > 0 {
            (vx, vy, vw, vh)
        } else {
            (self.area_x, self.area_y, self.area_w as i32, self.area_h as i32)
        };
        let px = (self.area_x + x - vx) as i64;
        let py = (self.area_y + y - vy) as i64;
        let nx = px * 65535 / (vw as i64 - 1).max(1);
        let ny = py * 65535 / (vh as i64 - 1).max(1);
        (nx.clamp(0, 65535) as i32, ny.clamp(0, 65535) as i32)
    }
}

impl InputSink for SendInputSink {
    fn inject(&mut self, ev: InputEvent) -> Result<()> {
        match ev {
            InputEvent::MouseMove { x, y } => {
                let (nx, ny) = self.to_virtual(x, y);
                let flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
                self.send(Self::mouse(nx, ny, 0, flags))
            }
            InputEvent::MouseButton { button, down } => {
                let flag = match (button, down) {
                    (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
                    (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
                    (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                    (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
                    (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                    (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
                };
                self.send(Self::mouse(0, 0, 0, flag))
            }
            InputEvent::Scroll { dx, dy } => {
                // Vertical first, then horizontal, if present.
                if dy != 0 {
                    self.send(Self::mouse(0, 0, dy * WHEEL_DELTA, MOUSEEVENTF_WHEEL))?;
                }
                if dx != 0 {
                    self.send(Self::mouse(0, 0, dx * WHEEL_DELTA, MOUSEEVENTF_HWHEEL))?;
                }
                Ok(())
            }
            InputEvent::Key { code, down } => {
                let flags = if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP };
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT { wVk: VIRTUAL_KEY(code), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
                    },
                };
                self.send(input)
            }
            InputEvent::Char { code } => {
                // KEYEVENTF_UNICODE types the character itself (UTF-16 units),
                // independent of the host's keyboard layout.
                let Some(ch) = char::from_u32(code) else { return Ok(()) };
                let mut units = [0u16; 2];
                for &u in ch.encode_utf16(&mut units).iter() {
                    for up in [false, true] {
                        let flags = if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE };
                        let input = INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: u, dwFlags: flags, time: 0, dwExtraInfo: 0 },
                            },
                        };
                        self.send(input)?;
                    }
                }
                Ok(())
            }
        }
    }

    fn set_area(&mut self, x: i32, y: i32, width: u32, height: u32) {
        self.area_x = x;
        self.area_y = y;
        self.area_w = width.max(1);
        self.area_h = height.max(1);
    }

    fn send_sas(&mut self) -> Result<()> {
        // SAFETY: load sas.dll and resolve SendSAS by name; the module is
        // freed after the call.
        unsafe {
            let module = LoadLibraryW(w!("sas.dll")).context("sas.dll is not available")?;
            let Some(proc_addr) = GetProcAddress(module, s!("SendSAS")) else {
                let _ = FreeLibrary(module);
                bail!("SendSAS not found in sas.dll");
            };
            // SendSAS: VOID WINAPI SendSAS(BOOL AsUser).
            let send_sas: unsafe extern "system" fn(BOOL) = std::mem::transmute(proc_addr);
            // FALSE = act as a service (requires SYSTEM + SoftwareSASGeneration).
            send_sas(BOOL(0));
            let _ = FreeLibrary(module);
        }
        Ok(())
    }
}
