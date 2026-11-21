//! The host loop shared by the GUI (in-process host) and the Windows service's
//! session worker: accept LAN + P2P connections and serve them one at a time
//! with the current access rules.

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use elo_client_core::host::{ApproveFn, EventFn};
use elo_protocol::Permissions;
use elo_session::Keypair;

use crate::config::Config;

/// Fixed port the direct (LAN) host listens on.
pub const DIRECT_PORT: u16 = 9000;

/// Access rules + permissions in effect for the next connection.
#[derive(Clone)]
pub struct HostSettings {
    pub password: String,
    pub open_access: bool,
    pub allow_requests: bool,
    pub trusted: Vec<String>,
    pub totp_secret: Option<Vec<u8>>,
    pub permissions: Permissions,
    pub show_pointer: bool,
}

impl HostSettings {
    /// `session_password` is the random (or rotating) password shown on screen;
    /// a fixed password, when set and OTP is off, takes its place.
    pub fn from_config(c: &Config, session_password: &str) -> Self {
        let fixed = c.unattended_password.trim();
        let password = if !fixed.is_empty() && !c.otp { fixed.to_string() } else { session_password.to_string() };
        let totp_secret = if c.twofa { elo_client_core::totp::secret_from_base32(&c.totp_secret) } else { None };
        HostSettings {
            password,
            open_access: c.open_access,
            allow_requests: c.allow_requests,
            trusted: c.trusted.iter().map(|t| t.fingerprint.clone()).collect(),
            totp_secret,
            permissions: Permissions {
                control: c.allow_control,
                file_transfer: c.allow_files,
                clipboard: c.allow_clipboard,
                audio: c.allow_audio,
            },
            show_pointer: c.show_pointer,
        }
    }
}

/// Load (or create and persist) a stable Noise host keypair in `dir/host.key`.
pub fn load_or_create_keypair(dir: &Path) -> Keypair {
    let path = dir.join("host.key");
    if let Ok(b) = std::fs::read(&path) {
        if b.len() == 64 {
            return Keypair { private: b[0..32].to_vec(), public: b[32..64].to_vec() };
        }
    }
    let kp = elo_session::generate_keypair().expect("keypair");
    let _ = std::fs::create_dir_all(dir);
    let mut bytes = kp.private.clone();
    bytes.extend_from_slice(&kp.public);
    let _ = std::fs::write(&path, &bytes);
    kp
}

/// Keep `short_id -> node_id` registered in a directory server (reconnects).
pub fn spawn_directory(directory: String, short_id: String, node_id: String) {
    if directory.trim().is_empty() || short_id.len() != 9 {
        return;
    }
    tokio::spawn(async move {
        loop {
            match elo_client_core::rendezvous::register_directory(&directory, &short_id, &node_id).await {
                Ok(mut conn) => {
                    tracing::info!(%directory, "registrado no diretório como {short_id}");
                    use tokio::io::AsyncReadExt;
                    let mut buf = [0u8; 64];
                    while let Ok(n) = conn.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                    }
                    tracing::warn!(%directory, "registro no diretório caiu; reconectando");
                }
                Err(e) => tracing::warn!(%directory, error = %e, "diretório indisponível"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}

pub struct HostRun {
    pub node: elo_p2p::Node,
    pub keypair: Keypair,
    pub download_dir: PathBuf,
    pub settings: Arc<dyn Fn() -> HostSettings + Send + Sync>,
    pub approver: ApproveFn,
    pub on_event: EventFn,
    pub status: Arc<Mutex<String>>,
    pub online_text: String,
    /// Called after each successful session (e.g. rotate a one-time password).
    pub after_session: Arc<dyn Fn() + Send + Sync>,
}

/// Run the host forever.
pub async fn run(h: HostRun) {
    #[cfg(windows)]
    {
        use elo_client_core::host::{serve_host_opts, HostOptions};
        use elo_client_core::platform::{DxgiCapture, SendInputSink};
        use elo_client_core::Capture;

        enum In {
            Lan(tokio::net::TcpStream),
            P2p(elo_p2p::Duplex),
        }

        let listener = tokio::net::TcpListener::bind(("0.0.0.0", DIRECT_PORT)).await;
        if let Err(e) = &listener {
            tracing::warn!(error = %e, "porta LAN {DIRECT_PORT} indisponível");
        }
        let listener = listener.ok();
        *h.status.lock().unwrap() = h.online_text.clone();
        loop {
            let incoming: anyhow::Result<In> = tokio::select! {
                r = async {
                    match &listener {
                        Some(l) => l.accept().await.map(|(s, _)| In::Lan(s)).map_err(anyhow::Error::from),
                        None => std::future::pending().await,
                    }
                } => r,
                r = h.node.accept() => r.map(In::P2p),
            };
            let incoming = match incoming {
                Ok(i) => i,
                Err(e) => {
                    tracing::warn!(error = %e, "erro ao aceitar conexão");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            let s = (h.settings)();
            let opts = HostOptions {
                password: s.password,
                open_access: s.open_access,
                trusted: s.trusted,
                totp_secret: s.totp_secret,
                approver: s.allow_requests.then(|| h.approver.clone()),
                on_event: Some(h.on_event.clone()),
                permissions: s.permissions,
                fps: 30,
                download_dir: h.download_dir.clone(),
                prefer_h264: true,
                show_pointer: s.show_pointer,
            };
            let factory = || -> anyhow::Result<(DxgiCapture, SendInputSink)> {
                let c = DxgiCapture::new()?;
                let i = SendInputSink::new(c.width(), c.height());
                Ok((c, i))
            };
            let r = match incoming {
                In::Lan(st) => serve_host_opts(st, &h.keypair, opts, factory).await,
                In::P2p(d) => serve_host_opts(d, &h.keypair, opts, factory).await,
            };
            match r {
                Ok(()) => (h.after_session)(),
                Err(e) => tracing::debug!(error = %e, "conexão encerrada"),
            }
            *h.status.lock().unwrap() = h.online_text.clone();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (&h.node, &h.keypair, &h.download_dir, &h.settings, &h.approver, &h.on_event, &h.after_session, &h.online_text);
        *h.status.lock().unwrap() = "pronto para conectar (servir a própria tela requer Windows)".into();
        std::future::pending::<()>().await
    }
}
