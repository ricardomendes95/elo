//! Local IPC between the GUI and the Windows service's session worker, so the
//! service (which has no window) can still ask the user to Allow/Deny, show the
//! "connected" banner and the operator pointer, and receive chat/kick.
//!
//! Transport: TCP on 127.0.0.1:47199, one JSON object per line. The GUI listens;
//! the worker connects (and reconnects). The worker must first present the
//! token stored in `C:\ProgramData\elo\ipc.token`.

#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use crate::events::{HostCtl, UiEvent};

pub const PORT: u16 = 47199;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Msg {
    Hello { token: String },
    Approve { id: u64, name: String, fp: String },
    ApproveReply { id: u64, allow: bool },
    Connected { name: String, fp: String },
    Disconnected { name: String },
    AuthFailed { name: String, reason: String },
    Pointer { x: i32, y: i32, ax: i32, ay: i32, aw: u32, ah: u32 },
    Chat { name: String, text: String },
    File { path: String },
    Kick,
}

/// The shared secret (created by the worker; the GUI reads it).
pub fn token(dir: &Path) -> String {
    let path = dir.join("ipc.token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        if t.trim().len() >= 16 {
            return t.trim().to_string();
        }
    }
    let t: String = (0..32).map(|_| format!("{:x}", rand::random::<u8>() & 0xf)).collect();
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&path, &t);
    t
}

async fn write_msg(w: &mut (impl AsyncWriteExt + Unpin), m: &Msg) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(m).unwrap_or_default();
    line.push(b'\n');
    w.write_all(&line).await
}

// ---------------------------------------------------------------------------
// GUI side
// ---------------------------------------------------------------------------

/// Listen for the session worker and turn its messages into UI events.
/// `on_approval(nome)` é chamado (fora da thread da UI) a cada pedido de
/// aprovação, para trazer a janela à frente mesmo escondida na bandeja.
pub fn spawn_gui_server(
    rt: &tokio::runtime::Runtime,
    token_dir: std::path::PathBuf,
    ui: std::sync::mpsc::Sender<UiEvent>,
    on_approval: Arc<dyn Fn(&str) + Send + Sync>,
) {
    rt.spawn(async move {
        let listener = match TcpListener::bind(("127.0.0.1", PORT)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "IPC com o serviço indisponível");
                return;
            }
        };
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let ui = ui.clone();
            let token_dir = token_dir.clone();
            let on_approval = on_approval.clone();
            tokio::spawn(async move {
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                // Authenticate the worker.
                let expected = std::fs::read_to_string(token_dir.join("ipc.token")).unwrap_or_default();
                match lines.next_line().await {
                    Ok(Some(l)) => match serde_json::from_str::<Msg>(&l) {
                        Ok(Msg::Hello { token }) if !expected.trim().is_empty() && token == expected.trim() => {}
                        _ => return,
                    },
                    _ => return,
                }
                tracing::info!("serviço conectado à interface");
                let (tx, mut rx) = mpsc::channel::<Msg>(32);
                tokio::spawn(async move {
                    while let Some(m) = rx.recv().await {
                        if write_msg(&mut w, &m).await.is_err() {
                            break;
                        }
                    }
                });
                while let Ok(Some(l)) = lines.next_line().await {
                    let Ok(m) = serde_json::from_str::<Msg>(&l) else { continue };
                    let mut alert: Option<String> = None;
                    let ev = match m {
                        Msg::Approve { id, name, fp } => {
                            let (otx, orx) = oneshot::channel::<bool>();
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let allow = orx.await.unwrap_or(false);
                                let _ = tx2.send(Msg::ApproveReply { id, allow }).await;
                            });
                            alert = Some(name.clone());
                            UiEvent::Approval { name, fp, reply: otx }
                        }
                        Msg::Connected { name, fp } => UiEvent::Connected { name, fp, ctl: HostCtl::Remote(tx.clone()) },
                        Msg::Disconnected { name } => UiEvent::Disconnected { name },
                        Msg::AuthFailed { name, reason } => UiEvent::AuthFailed { name, reason },
                        Msg::Pointer { x, y, ax, ay, aw, ah } => UiEvent::Pointer { x, y, area: (ax, ay, aw, ah) },
                        Msg::Chat { name, text } => UiEvent::Chat { name, text },
                        Msg::File { path } => UiEvent::File { path: path.into() },
                        _ => continue,
                    };
                    if ui.send(ev).is_err() {
                        break;
                    }
                    if let Some(name) = alert {
                        on_approval(&name);
                    }
                }
                tracing::info!("serviço desconectou da interface");
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Worker side
// ---------------------------------------------------------------------------

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<bool>>>>;

/// The worker's link to the GUI (if the user's GUI is running).
#[derive(Clone)]
pub struct WorkerLink {
    out: Arc<Mutex<Option<mpsc::Sender<Msg>>>>,
    pending: Pending,
    next: Arc<AtomicU64>,
    /// The live session, for chat/kick from the GUI.
    pub session: Arc<Mutex<Option<elo_client_core::host::HostSessionHandle>>>,
}

impl WorkerLink {
    /// Start (re)connecting to the GUI in the background.
    pub fn start(token: String) -> Self {
        let link = WorkerLink {
            out: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next: Arc::new(AtomicU64::new(1)),
            session: Arc::new(Mutex::new(None)),
        };
        let l = link.clone();
        tokio::spawn(async move {
            loop {
                if let Ok(stream) = TcpStream::connect(("127.0.0.1", PORT)).await {
                    let (r, mut w) = stream.into_split();
                    if write_msg(&mut w, &Msg::Hello { token: token.clone() }).await.is_ok() {
                        let (tx, mut rx) = mpsc::channel::<Msg>(32);
                        *l.out.lock().unwrap() = Some(tx);
                        let writer = tokio::spawn(async move {
                            while let Some(m) = rx.recv().await {
                                if write_msg(&mut w, &m).await.is_err() {
                                    break;
                                }
                            }
                        });
                        let mut lines = BufReader::new(r).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            match serde_json::from_str::<Msg>(&line) {
                                Ok(Msg::ApproveReply { id, allow }) => {
                                    if let Some(s) = l.pending.lock().unwrap().remove(&id) {
                                        let _ = s.send(allow);
                                    }
                                }
                                Ok(Msg::Chat { text, .. }) => {
                                    let h = l.session.lock().unwrap().clone();
                                    if let Some(h) = h {
                                        h.send_chat(&text).await;
                                    }
                                }
                                Ok(Msg::Kick) => {
                                    if let Some(h) = l.session.lock().unwrap().as_ref() {
                                        h.disconnect();
                                    }
                                }
                                _ => {}
                            }
                        }
                        writer.abort();
                    }
                    *l.out.lock().unwrap() = None;
                    // Fail any approvals still waiting on a vanished GUI.
                    l.pending.lock().unwrap().clear();
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
        link
    }

    /// Há uma interface (GUI do usuário) conectada a este worker?
    pub fn has_gui(&self) -> bool {
        self.out.lock().unwrap().is_some()
    }

    pub fn send(&self, m: Msg) {
        if let Some(tx) = self.out.lock().unwrap().as_ref() {
            let _ = tx.try_send(m);
        }
    }

    /// Ask the user (through the GUI) to allow `name`. False when no GUI.
    pub async fn approve(&self, name: String, fp: String) -> bool {
        let Some(tx) = self.out.lock().unwrap().clone() else { return false };
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, otx);
        if tx.send(Msg::Approve { id, name, fp }).await.is_err() {
            return false;
        }
        tokio::time::timeout(Duration::from_secs(120), orx).await.ok().and_then(|r| r.ok()).unwrap_or(false)
    }
}
