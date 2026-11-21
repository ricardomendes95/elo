//! Controller (operator) session loop.
//!
//! Ways out:
//!   * [`run_controller`] — reach a host by device id through the relay.
//!   * [`run_controller_direct`] — connect straight to a host `IP:port`.
//!   * [`connect_on`] / [`connect_on_opts`] — over any ready stream (P2P/QUIC…).
//!
//! All converge on `connect_session`, which returns a [`ControllerHandle`]
//! (to push input / chat / files / commands) and a stream of [`ControllerEvent`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Result};
use elo_protocol::{decode_msg, encode_msg, InputEvent, MonitorInfo, Permissions, SessionMsg, TransferId};
use elo_session::{handshake_initiator, Keypair};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::warn;

use crate::audio::AudioPlayer;
use crate::clipboard::ClipboardBridge;
use crate::codec::Decoder;
use crate::codec_h264::H264Decoder;
use crate::rendezvous::{auth_proof, connect_controller};
use crate::transfer::{chunks, IncomingFile};

/// Returned (as the error text) when the host asks for a 2FA code and none was
/// given: the UI should ask the user for the code and reconnect with it.
pub const ERR_2FA_REQUIRED: &str = "2FA_REQUIRED";

pub struct ControllerConfig {
    pub relay: String,
    pub target_id: String,
    pub keypair: Keypair,
    pub password: String,
    pub download_dir: PathBuf,
}

/// Config for a direct (no-relay) connection.
pub struct DirectControllerConfig {
    /// Host address, e.g. `192.168.0.10:9000`.
    pub host: String,
    pub keypair: Keypair,
    pub password: String,
    pub download_dir: PathBuf,
}

/// Extra connection options.
#[derive(Clone, Default)]
pub struct ConnectOptions {
    /// Name shown to the host (approval prompt, banner). Defaults to the OS user.
    pub name: Option<String>,
    /// Authenticator code, if the host has 2FA on.
    pub totp_code: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ControllerEvent {
    Frame { width: u32, height: u32, bgra: Vec<u8> },
    /// The host switched monitors / resolution.
    ScreenChanged { width: u32, height: u32 },
    Monitors(Vec<MonitorInfo>),
    Chat(String),
    FileReceived { path: PathBuf },
    Disconnected,
}

#[derive(Clone)]
pub struct ControllerHandle {
    pub screen: (u32, u32),
    pub permissions: Permissions,
    /// Host's static public key (Noise). Shown as a security fingerprint.
    pub peer_fingerprint: Vec<u8>,
    outbound: mpsc::Sender<Vec<u8>>,
    audio_on: Arc<AtomicBool>,
}

impl ControllerHandle {
    async fn send(&self, m: SessionMsg) -> Result<()> {
        self.outbound.send(encode_msg(&m)).await?;
        Ok(())
    }

    pub async fn send_input(&self, ev: InputEvent) -> Result<()> {
        self.send(SessionMsg::Input(ev)).await
    }

    pub async fn send_chat(&self, text: &str) -> Result<()> {
        self.send(SessionMsg::Chat(text.to_string())).await
    }

    /// Show our pointer on the host's screen (used when we cannot control).
    pub async fn send_pointer(&self, x: i32, y: i32) -> Result<()> {
        self.send(SessionMsg::Pointer { x, y }).await
    }

    pub async fn select_monitor(&self, index: u32) -> Result<()> {
        self.send(SessionMsg::SelectMonitor { index }).await
    }

    /// Ctrl+Alt+Del on the host.
    pub async fn send_sas(&self) -> Result<()> {
        self.send(SessionMsg::SendSas).await
    }

    /// Mute/unmute the host's audio locally.
    pub fn set_audio(&self, on: bool) {
        self.audio_on.store(on, Ordering::Relaxed);
    }
    pub fn audio(&self) -> bool {
        self.audio_on.load(Ordering::Relaxed)
    }

    pub async fn send_file(&self, id: TransferId, name: &str, bytes: &[u8]) -> Result<()> {
        self.send(SessionMsg::FileOffer { id, name: name.to_string(), size: bytes.len() as u64 }).await?;
        for (offset, chunk) in chunks(bytes) {
            self.send(SessionMsg::FileChunk { id, offset, data: chunk.to_vec() }).await?;
        }
        self.send(SessionMsg::FileDone { id }).await
    }
}

/// Connect to a host by device id through the relay.
pub async fn run_controller(cfg: ControllerConfig) -> Result<(ControllerHandle, mpsc::Receiver<ControllerEvent>)> {
    let (stream, peer_pubkey) = connect_controller(&cfg.relay, &cfg.target_id, &cfg.keypair.public).await?;
    connect_session(stream, Some(peer_pubkey), &cfg.keypair, &cfg.password, cfg.download_dir, ConnectOptions::default()).await
}

/// Connect straight to a host address (no relay).
pub async fn run_controller_direct(cfg: DirectControllerConfig) -> Result<(ControllerHandle, mpsc::Receiver<ControllerEvent>)> {
    run_controller_direct_opts(cfg, ConnectOptions::default()).await
}

pub async fn run_controller_direct_opts(
    cfg: DirectControllerConfig,
    opts: ConnectOptions,
) -> Result<(ControllerHandle, mpsc::Receiver<ControllerEvent>)> {
    let stream = tokio::time::timeout(std::time::Duration::from_secs(10), TcpStream::connect(&cfg.host))
        .await
        .map_err(|_| anyhow::anyhow!("tempo esgotado ao conectar em {}", cfg.host))??;
    connect_session(stream, None, &cfg.keypair, &cfg.password, cfg.download_dir, opts).await
}

/// Connect over any ready byte stream (direct TCP, or a P2P/QUIC stream).
pub async fn connect_on<S>(
    stream: S,
    keypair: &Keypair,
    password: &str,
    download_dir: PathBuf,
) -> Result<(ControllerHandle, mpsc::Receiver<ControllerEvent>)>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    connect_session(stream, None, keypair, password, download_dir, ConnectOptions::default()).await
}

/// [`connect_on`] with options (name, 2FA code).
pub async fn connect_on_opts<S>(
    stream: S,
    keypair: &Keypair,
    password: &str,
    download_dir: PathBuf,
    opts: ConnectOptions,
) -> Result<(ControllerHandle, mpsc::Receiver<ControllerEvent>)>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    connect_session(stream, None, keypair, password, download_dir, opts).await
}

fn os_user() -> String {
    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "Elo".into())
}

async fn connect_session<S>(
    stream: S,
    pin: Option<Vec<u8>>,
    keypair: &Keypair,
    password: &str,
    download_dir: PathBuf,
    opts: ConnectOptions,
) -> Result<(ControllerHandle, mpsc::Receiver<ControllerEvent>)>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut enc = handshake_initiator(stream, keypair).await?;
    if let Some(p) = &pin {
        if enc.remote_static() != p.as_slice() {
            bail!("peer static key does not match the key the relay advertised — possible MITM");
        }
    }
    let peer_fingerprint = enc.remote_static().to_vec();

    // ---- introduce ourselves, then authenticate ----
    let name = opts.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(os_user);
    enc.send(&encode_msg(&SessionMsg::Hello { name })).await?;
    let salt = match decode_msg(&enc.recv().await?)? {
        SessionMsg::AuthRequired { salt } => salt,
        m => bail!("expected AuthRequired, got {m:?}"),
    };
    enc.send(&encode_msg(&SessionMsg::AuthSubmit { proof: auth_proof(&salt, password) })).await?;
    loop {
        // May take a while: the host user could be deciding Allow/Deny.
        match decode_msg(&enc.recv().await?)? {
            SessionMsg::AuthOk => break,
            SessionMsg::AuthFailed => {
                if password.is_empty() {
                    bail!("acesso negado pelo host (ou sem aprovação disponível)");
                }
                bail!("senha incorreta, acesso negado, ou muitas tentativas");
            }
            SessionMsg::TwoFaRequired => match &opts.totp_code {
                Some(code) if !code.trim().is_empty() => {
                    enc.send(&encode_msg(&SessionMsg::TwoFaSubmit { code: code.trim().to_string() })).await?;
                }
                _ => bail!("{ERR_2FA_REQUIRED}"),
            },
            m => bail!("expected auth result, got {m:?}"),
        }
    }
    enc.send(&encode_msg(&SessionMsg::Caps {
        h264: crate::codec_h264::available(),
        audio: crate::audio::available(),
    }))
    .await?;

    let permissions = match decode_msg(&enc.recv().await?)? {
        SessionMsg::Permissions(p) => p,
        m => bail!("expected Permissions, got {m:?}"),
    };
    let screen = match decode_msg(&enc.recv().await?)? {
        SessionMsg::ScreenInfo { width, height } => (width, height),
        m => bail!("expected ScreenInfo, got {m:?}"),
    };

    let (tx, mut rx) = enc.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(256);
    let (ev_tx, ev_rx) = mpsc::channel::<ControllerEvent>(64);
    let audio_on = Arc::new(AtomicBool::new(true));

    // Writer: single owner of the encrypted sender.
    let mut sender = tx;
    tokio::spawn(async move {
        while let Some(bytes) = out_rx.recv().await {
            if sender.send(&bytes).await.is_err() {
                break;
            }
        }
    });

    // Keepalive.
    let ka = out_tx.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            tick.tick().await;
            if ka.send(encode_msg(&SessionMsg::Ping)).await.is_err() {
                break;
            }
        }
    });

    // Clipboard (text) both ways, when the host allows it.
    let (clip_tx, mut clip_rx) = mpsc::channel::<String>(8);
    let clipboard = if permissions.clipboard { ClipboardBridge::start(clip_tx) } else { None };
    let clip_out = out_tx.clone();
    tokio::spawn(async move {
        while let Some(t) = clip_rx.recv().await {
            if clip_out.send(encode_msg(&SessionMsg::Clipboard(t))).await.is_err() {
                break;
            }
        }
    });

    // Reader: decode frames and dispatch events.
    let audio_flag = audio_on.clone();
    tokio::spawn(async move {
        let clipboard = clipboard; // owned by the reader for the session's life
        let mut tiles = Decoder::new();
        let mut cur_screen = screen;
        let mut h264: Option<H264Decoder> = None;
        let mut player: Option<AudioPlayer> = None;
        let mut player_failed = false;
        let mut incoming: Option<IncomingFile> = None;
        loop {
            let bytes = match rx.recv().await {
                Ok(b) => b,
                Err(_) => {
                    let _ = ev_tx.send(ControllerEvent::Disconnected).await;
                    break;
                }
            };
            let msg = match decode_msg(&bytes) {
                Ok(m) => m,
                Err(e) => {
                    warn!(error = %e, "undecodable session message");
                    continue;
                }
            };
            match msg {
                SessionMsg::Frame(frame) => match tiles.apply(&frame) {
                    Ok(canvas) => {
                        let bgra = canvas.to_vec();
                        let (width, height) = tiles.dims();
                        let _ = ev_tx.send(ControllerEvent::Frame { width, height, bgra }).await;
                    }
                    Err(e) => warn!(error = %e, "frame decode failed"),
                },
                SessionMsg::FrameH264 { data, .. } => {
                    if h264.is_none() {
                        match H264Decoder::new() {
                            Ok(d) => h264 = Some(d),
                            Err(e) => {
                                warn!(error = %e, "H.264 decoder unavailable");
                                continue;
                            }
                        }
                    }
                    if let Some(d) = &mut h264 {
                        match d.decode(&data) {
                            Ok(Some((width, height, bgra))) => {
                                let (width, height, bgra) = crop_to(width, height, bgra, cur_screen);
                                let _ = ev_tx.send(ControllerEvent::Frame { width, height, bgra }).await;
                            }
                            Ok(None) => {}
                            Err(e) => warn!(error = %e, "H.264 decode failed"),
                        }
                    }
                }
                SessionMsg::ScreenInfo { width, height } => {
                    cur_screen = (width, height);
                    let _ = ev_tx.send(ControllerEvent::ScreenChanged { width, height }).await;
                }
                SessionMsg::Monitors { list } => {
                    let _ = ev_tx.send(ControllerEvent::Monitors(list)).await;
                }
                SessionMsg::Audio { rate, channels, pcm } => {
                    if !audio_flag.load(Ordering::Relaxed) || player_failed {
                        continue;
                    }
                    if player.is_none() {
                        match AudioPlayer::new() {
                            Ok(p) => player = Some(p),
                            Err(e) => {
                                warn!(error = %e, "audio playback unavailable");
                                player_failed = true;
                                continue;
                            }
                        }
                    }
                    if let Some(p) = &mut player {
                        p.push(rate, channels, &pcm);
                    }
                }
                SessionMsg::Clipboard(text) => {
                    if let Some(c) = &clipboard {
                        c.set(text);
                    }
                }
                SessionMsg::Chat(text) => {
                    let _ = ev_tx.send(ControllerEvent::Chat(text)).await;
                }
                SessionMsg::FileOffer { id, name, size } => {
                    let safe = std::path::Path::new(&name)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "download.bin".into());
                    incoming = Some(IncomingFile::new(id, safe, size));
                }
                SessionMsg::FileChunk { id, offset, data } => {
                    if let Some(f) = &mut incoming {
                        if f.id == id && f.push(offset, &data).is_err() {
                            incoming = None;
                        }
                    }
                }
                SessionMsg::FileDone { .. } => {
                    if let Some(f) = incoming.take() {
                        let name = f.name.clone();
                        if let Ok(data) = f.finish() {
                            let path = download_dir.join(&name);
                            if tokio::fs::write(&path, &data).await.is_ok() {
                                let _ = ev_tx.send(ControllerEvent::FileReceived { path }).await;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    });

    Ok((
        ControllerHandle { screen, permissions, peer_fingerprint, outbound: out_tx, audio_on },
        ev_rx,
    ))
}

/// H.264 pads odd sizes to even; crop back to the host's real screen size.
fn crop_to(w: u32, h: u32, bgra: Vec<u8>, screen: (u32, u32)) -> (u32, u32, Vec<u8>) {
    let (sw, sh) = screen;
    if (w, h) == (sw, sh) || sw == 0 || sh == 0 || sw > w || sh > h || w - sw > 1 || h - sh > 1 {
        return (w, h, bgra);
    }
    let mut out = Vec::with_capacity(sw as usize * sh as usize * 4);
    for row in 0..sh as usize {
        let start = row * w as usize * 4;
        out.extend_from_slice(&bgra[start..start + sw as usize * 4]);
    }
    (sw, sh, out)
}

#[cfg(test)]
mod crop_tests {
    #[test]
    fn crops_one_pixel_padding() {
        let (w, h) = (4u32, 4u32);
        let buf: Vec<u8> = (0..w * h * 4).map(|i| i as u8).collect();
        let (cw, ch, out) = super::crop_to(w, h, buf.clone(), (3, 3));
        assert_eq!((cw, ch), (3, 3));
        assert_eq!(out.len(), 3 * 3 * 4);
        assert_eq!(&out[..12], &buf[..12]);
        assert_eq!(&out[12..24], &buf[16..28]);
        // Unrelated sizes are left alone.
        assert_eq!(super::crop_to(w, h, buf, (2, 2)).0, 4);
    }
}
