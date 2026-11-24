//! Windows service mode: run the Elo host as LocalSystem so it starts at boot,
//! before login, and (running as SYSTEM) can follow the secure desktop — the
//! requirement for access while the screen is locked.
//!
//! The service itself lives in session 0, which has no access to the user's
//! display. It therefore acts as a supervisor: it launches `elo.exe
//! --session-worker` as SYSTEM inside the active console session (so the
//! worker can capture the screen and follow the secure desktop) and relaunches
//! it when it exits or the console session changes. If the worker cannot be
//! started at all, the service falls back to hosting in-process.
//!
//! State (identity, unattended password, ...) lives in a machine-wide dir
//! (`C:\ProgramData\elo`), seeded from the user's config at install time.
#![cfg(windows)]

use std::ffi::OsString;
use std::time::Duration;

use anyhow::Result;
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

const SERVICE_NAME: &str = "EloHost";
const SERVICE_DISPLAY: &str = "Elo Remote Host";

/// Is the Elo service registered?
pub fn is_installed() -> bool {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .and_then(|m| m.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS))
        .is_ok()
}

/// Register and start the service (must run elevated). Seeds the machine dir
/// with the user's identity + password first.
pub fn install() -> Result<()> {
    crate::config::seed_service_dir();
    let dir = crate::config::service_dir();
    // Let the interactive user's GUI update the machine config (Users = S-1-5-32-545).
    run_quiet("icacls", &[&dir.to_string_lossy(), "/grant", "*S-1-5-32-545:(OI)(CI)M"]);
    // Allow services to generate Ctrl+Alt+Del via SendSAS (3 = services and
    // ease-of-access apps).
    run_quiet(
        "reg",
        &[
            "add",
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
            "/v",
            "SoftwareSASGeneration",
            "/t",
            "REG_DWORD",
            "/d",
            "3",
            "/f",
        ],
    );
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments: vec![OsString::from("--service")],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    let service =
        manager.create_service(&info, ServiceAccess::START | ServiceAccess::CHANGE_CONFIG)?;
    let _ = service.set_description("Elo: acesso remoto não supervisionado (host).");
    // Restart the service 5 s after a crash; reset the failure count after 60 s.
    run_quiet("sc", &["failure", SERVICE_NAME, "reset=", "60", "actions=", "restart/5000"]);
    service.start::<OsString>(&[])?;
    Ok(())
}

/// Stop and delete the service (must run elevated).
pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service =
        manager.open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::DELETE)?;
    let _ = service.stop();
    service.delete()?;
    Ok(())
}

/// Re-launch this exe elevated (UAC) with `arg` (e.g. `--install-service`).
pub fn elevate(arg: &str) -> Result<()> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let exe = std::env::current_exe()?;
    let verb = HSTRING::from("runas");
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(arg);
    // SAFETY: ShellExecuteW with owned, null-terminated HSTRING buffers kept
    // alive for the call. We ignore the returned instance handle.
    unsafe {
        ShellExecuteW(
            HWND::default(),
            &verb,
            &file,
            &params,
            windows::core::PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
    Ok(())
}

/// Run a console tool without flashing a window; best effort, result ignored.
fn run_quiet(program: &str, args: &[&str]) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

// ----------------------------------------------------------------------------
// Service entry point + session-worker supervisor (runs as LocalSystem).
// ----------------------------------------------------------------------------

windows_service::define_windows_service!(ffi_service_main, service_main);

/// Called by `main` when started with `--service`: hands control to the SCM.
pub fn run_dispatcher() -> Result<()> {
    windows_service::service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| anyhow::anyhow!("service dispatcher: {e}"))?;
    Ok(())
}

fn service_main(_args: Vec<OsString>) {
    let _ = run_service();
}

/// How often the supervisor checks the console session and the worker.
const SUPERVISE_INTERVAL: Duration = Duration::from_secs(2);
/// Consecutive launch failures before falling back to in-process hosting.
const MAX_LAUNCH_FAILURES: u32 = 3;
/// `WTSGetActiveConsoleSessionId` result when no session is attached.
const NO_SESSION: u32 = 0xFFFF_FFFF;

fn run_service() -> windows_service::Result<()> {
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

    let (tx, rx) = std::sync::mpsc::channel();
    let handler = move |control| -> ServiceControlHandlerResult {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, handler)?;

    let running = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    status_handle.set_service_status(running.clone())?;

    // Ctrl+Alt+Del must be sent by the service process itself; the session
    // worker requests it through a named event.
    elo_client_core::platform::spawn_sas_listener();

    supervise(&rx);

    status_handle.set_service_status(ServiceStatus {
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        ..running
    })?;
    Ok(())
}

/// Keep a session worker alive in the active console session until a stop
/// request arrives on `stop`.
fn supervise(stop: &std::sync::mpsc::Receiver<()>) {
    use std::sync::mpsc::RecvTimeoutError;
    use windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;

    let mut worker: Option<Worker> = None;
    let mut failures = 0u32;
    loop {
        // SAFETY: no preconditions; returns 0xFFFFFFFF when no session.
        let session = unsafe { WTSGetActiveConsoleSessionId() };
        if session != NO_SESSION {
            let needs_launch = match &worker {
                None => true,
                Some(w) => w.session != session || !w.is_alive(),
            };
            if needs_launch {
                // Drop (terminates) the stale worker before starting a new one.
                worker = None;
                match Worker::launch(session) {
                    Ok(w) => {
                        worker = Some(w);
                        failures = 0;
                    }
                    Err(_) => {
                        failures += 1;
                        if failures >= MAX_LAUNCH_FAILURES {
                            run_in_process(stop);
                            return;
                        }
                    }
                }
            }
        }
        match stop.recv_timeout(SUPERVISE_INTERVAL) {
            Err(RecvTimeoutError::Timeout) => continue,
            // Stop/Shutdown (or the handler vanished): `worker` drops here and
            // terminates the child.
            _ => return,
        }
    }
}

/// Fallback: host inside the service process (session 0) until stopped.
fn run_in_process(stop: &std::sync::mpsc::Receiver<()>) {
    let Ok(rt) = tokio::runtime::Runtime::new() else {
        let _ = stop.recv();
        return;
    };
    let task = rt.spawn(crate::worker::run_headless(crate::config::service_dir()));
    let _ = stop.recv();
    task.abort();
    rt.shutdown_timeout(Duration::from_secs(2));
}

/// A `--session-worker` child process running as SYSTEM in a user session.
/// Dropping it terminates the process.
struct Worker {
    process: windows::Win32::Foundation::HANDLE,
    session: u32,
}

impl Worker {
    /// Launch `"<this exe>" --session-worker` as SYSTEM in `session`, on the
    /// interactive desktop `winsta0\default`.
    fn launch(session: u32) -> Result<Self> {
        use windows::core::{HSTRING, PCWSTR, PWSTR};
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::Security::{
            DuplicateTokenEx, SecurityImpersonation, SetTokenInformation, TokenPrimary,
            TokenSessionId, TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID,
            TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY,
        };
        use windows::Win32::System::Threading::{
            CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, CREATE_NO_WINDOW,
            PROCESS_INFORMATION, STARTUPINFOW,
        };
        const MAXIMUM_ALLOWED: TOKEN_ACCESS_MASK = TOKEN_ACCESS_MASK(0x0200_0000);

        /// Closes a Win32 handle on drop.
        struct Owned(HANDLE);
        impl Drop for Owned {
            fn drop(&mut self) {
                if !self.0.is_invalid() {
                    // SAFETY: we own this handle and close it exactly once.
                    unsafe {
                        let _ = CloseHandle(self.0);
                    }
                }
            }
        }

        // SAFETY: OpenProcessToken on our own pseudo-handle; the out-param
        // receives an owned token handle.
        let own_token = unsafe {
            let mut h = HANDLE::default();
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_DUPLICATE
                    | TOKEN_QUERY
                    | TOKEN_ASSIGN_PRIMARY
                    | TOKEN_ADJUST_DEFAULT
                    | TOKEN_ADJUST_SESSIONID,
                &mut h,
            )?;
            Owned(h)
        };
        // SAFETY: duplicate into a primary token we own.
        let token = unsafe {
            let mut h = HANDLE::default();
            DuplicateTokenEx(own_token.0, MAXIMUM_ALLOWED, None, SecurityImpersonation, TokenPrimary, &mut h)?;
            Owned(h)
        };
        // SAFETY: TokenSessionId takes a DWORD; `session` outlives the call.
        // Requires SeTcbPrivilege, which LocalSystem holds.
        unsafe {
            SetTokenInformation(
                token.0,
                TokenSessionId,
                &session as *const u32 as *const core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            )?;
        }

        let exe = std::env::current_exe()?;
        let app = HSTRING::from(exe.as_os_str());
        // CreateProcessW may write into the command line, so it must be a
        // mutable, NUL-terminated buffer.
        let mut cmdline: Vec<u16> = format!("\"{}\" --session-worker", exe.display())
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut desktop: Vec<u16> = "winsta0\\default".encode_utf16().chain(std::iter::once(0)).collect();
        let si = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(desktop.as_mut_ptr()),
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();
        // SAFETY: all string buffers (`app`, `cmdline`, `desktop`) and the
        // structs outlive the call; on success we own both returned handles.
        unsafe {
            CreateProcessAsUserW(
                token.0,
                &app,
                PWSTR(cmdline.as_mut_ptr()),
                None,
                None,
                false,
                CREATE_NO_WINDOW,
                None,
                PCWSTR::null(),
                &si,
                &mut pi,
            )?;
        }
        drop(Owned(pi.hThread));
        Ok(Self { process: pi.hProcess, session })
    }

    /// Is the child still running?
    fn is_alive(&self) -> bool {
        use windows::Win32::Foundation::WAIT_TIMEOUT;
        use windows::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: zero-timeout wait on a process handle we own.
        unsafe { WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::TerminateProcess;
        // SAFETY: we own the process handle; terminating an already-exited
        // process fails harmlessly. The handle is closed exactly once.
        unsafe {
            if self.is_alive() {
                let _ = TerminateProcess(self.process, 0);
            }
            let _ = CloseHandle(self.process);
        }
    }
}
