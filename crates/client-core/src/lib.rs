//! Platform-independent client logic: the screen codec, file transfer, and the
//! host/controller session loops. The loops are generic over [`Capture`] and
//! [`InputSink`] so they run against real Windows backends in production and
//! against in-memory mocks in tests — which is how the whole stack is
//! exercised end-to-end on Linux.

use anyhow::Result;
use elo_protocol::{InputEvent, MonitorInfo};

pub mod audio;
pub mod clipboard;
pub mod codec;
#[cfg(feature = "h264")]
pub mod codec_h264;
#[cfg(not(feature = "h264"))]
#[path = "codec_h264_stub.rs"]
pub mod codec_h264;
pub mod controller;
pub mod discovery;
pub mod host;

pub use host::HostLink;
#[cfg(windows)]
pub mod platform;
pub mod rendezvous;
pub mod shortid;
pub mod totp;
pub mod transfer;

#[cfg(test)]
mod e2e_tests;

/// A source of screen frames. Pixels are BGRA, row-major, `width*height*4`
/// bytes — the layout DXGI Desktop Duplication hands back on Windows.
#[async_trait::async_trait]
pub trait Capture: Send {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    /// Grab the current framebuffer. Returns `width()*height()*4` bytes.
    async fn frame(&mut self) -> Result<Vec<u8>>;
    /// The displays this backend can capture (empty = unknown / single).
    fn monitors(&self) -> Vec<MonitorInfo> {
        Vec::new()
    }
    /// Switch capture to monitor `index` (from [`Capture::monitors`]). After a
    /// successful switch `width()`/`height()` report the new monitor's size.
    async fn select_monitor(&mut self, _index: u32) -> Result<()> {
        anyhow::bail!("this capture backend has a single display")
    }
}

/// A sink that injects input events into the local machine (Windows SendInput
/// in production; a recorder in tests).
pub trait InputSink: Send {
    fn inject(&mut self, ev: InputEvent) -> Result<()>;
    /// The captured monitor's rectangle in the virtual desktop. Input
    /// coordinates are relative to this area.
    fn set_area(&mut self, _x: i32, _y: i32, _width: u32, _height: u32) {}
    /// Send the secure attention sequence (Ctrl+Alt+Del). Needs a privileged
    /// host (Windows service).
    fn send_sas(&mut self) -> Result<()> {
        anyhow::bail!("Ctrl+Alt+Del is not available on this host")
    }
}

// ----------------------------------------------------------------------------
// Test doubles, also usable for non-Windows builds / demos.
// ----------------------------------------------------------------------------

/// A capture backend that paints a deterministic, slowly-changing pattern.
/// Lets the full pipeline run without a real screen.
pub struct MockCapture {
    width: u32,
    height: u32,
    tick: u64,
}

impl MockCapture {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, tick: 0 }
    }
    /// Render the current synthetic framebuffer without consuming it.
    pub fn render(&self) -> Vec<u8> {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut buf = vec![0u8; w * h * 4];
        let t = self.tick as usize;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                buf[i] = ((x + t) & 0xff) as u8; // B
                buf[i + 1] = ((y + t) & 0xff) as u8; // G
                buf[i + 2] = ((x ^ y) & 0xff) as u8; // R
                buf[i + 3] = 0xff; // A
            }
        }
        buf
    }
}

#[async_trait::async_trait]
impl Capture for MockCapture {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    async fn frame(&mut self) -> Result<Vec<u8>> {
        let buf = self.render();
        self.tick = self.tick.wrapping_add(1);
        Ok(buf)
    }
}

/// An input sink that records everything injected, for assertions in tests.
#[derive(Default)]
pub struct RecordingInput {
    pub events: std::sync::Arc<std::sync::Mutex<Vec<InputEvent>>>,
}

impl InputSink for RecordingInput {
    fn inject(&mut self, ev: InputEvent) -> Result<()> {
        self.events.lock().unwrap().push(ev);
        Ok(())
    }
}
