//! Text clipboard sync for a session. A background thread polls the local
//! clipboard; local changes are sent to the peer, and text received from the
//! peer is applied locally (without echoing it back).

#![cfg_attr(not(feature = "clipboard"), allow(unused_imports, dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as smpsc, Arc};
use std::time::Duration;

use tokio::sync::mpsc;

/// Largest clipboard text we sync (bytes).
const MAX_TEXT: usize = 1024 * 1024;

pub struct ClipboardBridge {
    set_tx: smpsc::Sender<String>,
    stop: Arc<AtomicBool>,
}

impl ClipboardBridge {
    /// Start watching the local clipboard. Local changes are sent to `out`.
    /// Returns `None` when no clipboard is available (e.g. headless).
    ///
    /// Without the `clipboard` feature (Android) this always returns `None`;
    /// the app syncs text through the raw `Clipboard` events instead.
    #[cfg(not(feature = "clipboard"))]
    pub fn start(_out: mpsc::Sender<String>) -> Option<Self> {
        None
    }

    #[cfg(feature = "clipboard")]
    pub fn start(out: mpsc::Sender<String>) -> Option<Self> {
        let (set_tx, set_rx) = smpsc::channel::<String>();
        let (ready_tx, ready_rx) = smpsc::channel::<bool>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::Builder::new()
            .name("elo-clipboard".into())
            .spawn(move || {
                let mut cb = match arboard::Clipboard::new() {
                    Ok(c) => c,
                    Err(_) => {
                        let _ = ready_tx.send(false);
                        return;
                    }
                };
                let _ = ready_tx.send(true);
                // Don't push whatever was already copied before the session.
                let mut last = cb.get_text().unwrap_or_default();
                while !stop2.load(Ordering::Relaxed) {
                    while let Ok(text) = set_rx.try_recv() {
                        if cb.set_text(text.clone()).is_ok() {
                            last = text;
                        }
                    }
                    if let Ok(t) = cb.get_text() {
                        if t != last && !t.is_empty() && t.len() <= MAX_TEXT {
                            last = t.clone();
                            if out.blocking_send(t).is_err() {
                                break;
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            })
            .ok()?;
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(true) => Some(Self { set_tx, stop }),
            _ => None,
        }
    }

    /// Apply text received from the peer to the local clipboard.
    pub fn set(&self, text: String) {
        if text.len() <= MAX_TEXT {
            let _ = self.set_tx.send(text);
        }
    }
}

impl Drop for ClipboardBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
