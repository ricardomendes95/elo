//! Host-side events delivered to the UI thread, whether the host runs in this
//! process or in the Windows service's session worker (via IPC).

use std::path::PathBuf;

use elo_client_core::host::{HostEvent, HostSessionHandle};

use crate::ipc::Msg;

pub enum UiEvent {
    /// Someone without a password asks to come in; reply true to allow.
    Approval { name: String, fp: String, reply: tokio::sync::oneshot::Sender<bool> },
    Connected { name: String, fp: String, ctl: HostCtl },
    Disconnected { name: String },
    AuthFailed { name: String, reason: String },
    /// Operator pointer (view-only), relative to `area` (virtual-desktop rect).
    Pointer { x: i32, y: i32, area: (i32, i32, u32, u32) },
    Chat { name: String, text: String },
    File { path: PathBuf },
}

/// Controls a live host session (in-process or in the service worker).
#[derive(Clone)]
pub enum HostCtl {
    Local(HostSessionHandle),
    Remote(tokio::sync::mpsc::Sender<Msg>),
}

impl HostCtl {
    pub fn chat(&self, rt: &tokio::runtime::Runtime, text: String) {
        match self {
            HostCtl::Local(h) => {
                let h = h.clone();
                rt.spawn(async move { h.send_chat(&text).await });
            }
            HostCtl::Remote(tx) => {
                let _ = tx.try_send(Msg::Chat { name: String::new(), text });
            }
        }
    }

    pub fn kick(&self) {
        match self {
            HostCtl::Local(h) => h.disconnect(),
            HostCtl::Remote(tx) => {
                let _ = tx.try_send(Msg::Kick);
            }
        }
    }
}

/// Map an in-process host event to a UI event.
pub fn from_host_event(ev: HostEvent) -> UiEvent {
    match ev {
        HostEvent::Connected { peer, session } => UiEvent::Connected { name: peer.name, fp: peer.fingerprint, ctl: HostCtl::Local(session) },
        HostEvent::Disconnected { peer } => UiEvent::Disconnected { name: peer.name },
        HostEvent::AuthFailed { peer, reason } => UiEvent::AuthFailed { name: peer.name, reason },
        HostEvent::Pointer { x, y, area } => UiEvent::Pointer { x, y, area },
        HostEvent::Chat { peer, text } => UiEvent::Chat { name: peer.name, text },
        HostEvent::FileReceived { path } => UiEvent::File { path },
    }
}
