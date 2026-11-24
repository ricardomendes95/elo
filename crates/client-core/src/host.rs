//! Host (controlled machine) session loop.
//!
//! Ways in:
//!   * [`run_host`] — via the relay (rendezvous by device id; works through NAT).
//!   * [`run_host_direct`] — listen on a TCP port (same LAN / reachable host).
//!   * [`serve_host_opts`] — over any ready stream (direct TCP, P2P/QUIC), with
//!     the full option set (approval, trusted devices, 2FA, events…).
//!
//! Access decision, after the Noise handshake (so the peer's key fingerprint
//! and name are known):
//!   1. trusted (paired) device          → accepted (2FA still asked if on)
//!   2. open access ("unattended")       → accepted (2FA still asked if on)
//!   3. correct password                 → accepted (2FA still asked if on)
//!   4. no password typed + approver     → ask the user Allow/Deny (no 2FA)
//!   5. otherwise                        → rejected (failures are rate-limited)

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use elo_protocol::{decode_msg, encode_msg, MonitorInfo, Permissions, SessionMsg};
use elo_session::{handshake_responder, EncryptedStream, Keypair};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use crate::audio::AudioCapture;
use crate::clipboard::ClipboardBridge;
use crate::codec::Encoder;
use crate::codec_h264::H264Encoder;
use crate::rendezvous::{auth_proof, register_host};
use crate::transfer::{chunks, IncomingFile};
use crate::{totp, Capture, InputSink};

/// Who is connecting.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// Display name the controller sent (`Hello`).
    pub name: String,
    /// Hex of the controller's static Noise key — stable per device.
    pub fingerprint: String,
}

/// Something that happened on the host side, for UI / logs / overlays.
#[derive(Clone)]
pub enum HostEvent {
    Connected { peer: PeerInfo, session: HostSessionHandle },
    Disconnected { peer: PeerInfo },
    AuthFailed { peer: PeerInfo, reason: String },
    /// Operator pointer (view-only sessions). `x`,`y` are relative to `area`
    /// (the captured monitor in virtual-desktop pixels: x, y, width, height).
    Pointer { x: i32, y: i32, area: (i32, i32, u32, u32) },
    Chat { peer: PeerInfo, text: String },
    FileReceived { path: PathBuf },
}

/// Control a live host session from the host's own UI.
#[derive(Clone)]
pub struct HostSessionHandle {
    out: mpsc::Sender<Vec<u8>>,
    cancel: watch::Sender<bool>,
}

impl HostSessionHandle {
    /// Send a chat line to the operator.
    pub async fn send_chat(&self, text: &str) {
        let _ = self.out.send(encode_msg(&SessionMsg::Chat(text.to_string()))).await;
    }
    /// End the session now (kick the operator).
    pub fn disconnect(&self) {
        let _ = self.cancel.send(true);
    }
}

/// Envia conteúdo do host para o controlador conectado na sessão ativa.
///
/// É entregue à UI do host por [`HostOptions::link_out`] quando uma sessão fica
/// ativa e removido (`None`) ao desconectar. Os métodos são "dispare e esqueça":
/// agendam o envio na runtime Tokio da sessão, usando o mesmo canal de saída do
/// loop do host (logo, respeitam o backpressure do caminho de transferência em
/// chunks — `FileOffer`/`FileChunk`/`FileDone`, CHUNK de 60 KiB).
#[derive(Clone)]
pub struct HostLink {
    out: mpsc::Sender<Vec<u8>>,
    rt: tokio::runtime::Handle,
}

impl HostLink {
    /// Envia um arquivo ao controlador conectado (chunked, em segundo plano).
    pub fn send_file(&self, name: String, data: Vec<u8>) {
        let out = self.out.clone();
        self.rt.spawn(async move {
            let id: elo_protocol::TransferId = rand::random();
            if out.send(encode_msg(&SessionMsg::FileOffer { id, name, size: data.len() as u64 })).await.is_err() {
                return;
            }
            for (offset, chunk) in chunks(&data) {
                if out.send(encode_msg(&SessionMsg::FileChunk { id, offset, data: chunk.to_vec() })).await.is_err() {
                    return;
                }
            }
            let _ = out.send(encode_msg(&SessionMsg::FileDone { id })).await;
        });
    }

    /// Envia texto para a área de transferência do controlador (em segundo plano).
    pub fn send_clipboard(&self, text: String) {
        let out = self.out.clone();
        self.rt.spawn(async move {
            let _ = out.send(encode_msg(&SessionMsg::Clipboard(text))).await;
        });
    }
}

/// Async "Allow?" callback: returns true to allow.
pub type ApproveFn = Arc<dyn Fn(PeerInfo) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;
/// Event sink.
pub type EventFn = Arc<dyn Fn(HostEvent) + Send + Sync>;

#[derive(Clone)]
pub struct HostOptions {
    /// Password accepted without approval. Empty = no password path.
    pub password: String,
    /// "Unattended access": anyone who reaches this host is accepted.
    pub open_access: bool,
    /// Fingerprints (hex) of paired devices accepted without password/approval.
    pub trusted: Vec<String>,
    /// When set, password/open/trusted logins must also give a TOTP code.
    pub totp_secret: Option<Vec<u8>>,
    /// Asks the local user when a controller connects without a password.
    pub approver: Option<ApproveFn>,
    pub on_event: Option<EventFn>,
    pub permissions: Permissions,
    pub fps: u32,
    pub download_dir: PathBuf,
    /// Use H.264 when the controller supports it.
    pub prefer_h264: bool,
    /// Report the operator's pointer (view-only sessions) via `on_event`.
    pub show_pointer: bool,
    /// Receptáculo para o [`HostLink`] da sessão ativa. Quando uma sessão de
    /// controlador fica ativa, o host grava `Some(link)`; ao desconectar, `None`.
    /// A UI do host lê daqui para enviar arquivos/clipboard ao controlador.
    pub link_out: Option<std::sync::Arc<std::sync::Mutex<Option<HostLink>>>>,
}

impl HostOptions {
    /// Password-only options (the classic behaviour).
    pub fn with_password(password: &str, permissions: Permissions, fps: u32, download_dir: PathBuf) -> Self {
        Self {
            password: password.to_string(),
            open_access: false,
            trusted: Vec::new(),
            totp_secret: None,
            approver: None,
            on_event: None,
            permissions,
            fps,
            download_dir,
            prefer_h264: true,
            show_pointer: true,
            link_out: None,
        }
    }

    fn emit(&self, ev: HostEvent) {
        if let Some(f) = &self.on_event {
            f(ev);
        }
    }
}

/// Host reachable through the relay by its device id.
pub struct HostConfig {
    pub relay: String,
    pub device_id: String,
    pub keypair: Keypair,
    pub password: String,
    pub permissions: Permissions,
    pub fps: u32,
    pub download_dir: PathBuf,
}

/// Host that listens directly for a controller (no relay).
pub struct DirectHostConfig {
    /// Address to bind, e.g. `0.0.0.0:9000`.
    pub bind: String,
    pub keypair: Keypair,
    pub password: String,
    pub permissions: Permissions,
    pub fps: u32,
    pub download_dir: PathBuf,
}

/// Serve one controller via the relay (key pinned to the relay's advert).
pub async fn run_host<C, I>(cfg: HostConfig, capture: C, input: I) -> Result<()>
where
    C: Capture + 'static,
    I: InputSink + 'static,
{
    let (stream, peer_pubkey) = register_host(&cfg.relay, &cfg.device_id, &cfg.keypair.public).await?;
    let enc = handshake_responder(stream, &cfg.keypair).await?;
    if enc.remote_static() != peer_pubkey.as_slice() {
        bail!("peer static key does not match the key the relay advertised — possible MITM");
    }
    let opts = HostOptions::with_password(&cfg.password, cfg.permissions, cfg.fps, cfg.download_dir);
    serve_host_enc(enc, opts, move || Ok((capture, input))).await
}

/// Accept one direct controller connection on `cfg.bind`, then return.
pub async fn run_host_direct<C, I>(cfg: DirectHostConfig, capture: C, input: I) -> Result<()>
where
    C: Capture + 'static,
    I: InputSink + 'static,
{
    let listener = TcpListener::bind(&cfg.bind).await?;
    info!(bind = %cfg.bind, "direct host listening");
    let (stream, peer) = listener.accept().await?;
    info!(%peer, "direct controller connected");
    let opts = HostOptions::with_password(&cfg.password, cfg.permissions, cfg.fps, cfg.download_dir);
    serve_host_opts(stream, &cfg.keypair, opts, move || Ok((capture, input))).await
}

/// Classic entry point: password only, backend already created.
#[allow(clippy::too_many_arguments)]
pub async fn serve_host_on<S, C, I>(
    stream: S,
    keypair: &Keypair,
    password: &str,
    permissions: Permissions,
    fps: u32,
    download_dir: PathBuf,
    capture: C,
    input: I,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C: Capture + 'static,
    I: InputSink + 'static,
{
    let opts = HostOptions::with_password(password, permissions, fps, download_dir);
    serve_host_opts(stream, keypair, opts, move || Ok((capture, input))).await
}

/// Full entry point. The capture/input backend is created by `make_backend`
/// only after the peer is accepted (no screen capture for probes or rejected
/// peers).
pub async fn serve_host_opts<S, C, I, F>(stream: S, keypair: &Keypair, opts: HostOptions, make_backend: F) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C: Capture + 'static,
    I: InputSink + 'static,
    F: FnOnce() -> Result<(C, I)> + Send,
{
    let enc = tokio::time::timeout(Duration::from_secs(20), handshake_responder(stream, keypair))
        .await
        .map_err(|_| anyhow::anyhow!("handshake timed out"))??;
    serve_host_enc(enc, opts, make_backend).await
}

// ---------------------------------------------------------------------------
// Brute-force protection: after 5 failures within 60 s, refuse for 60 s.
// ---------------------------------------------------------------------------

static FAILURES: StdMutex<Vec<Instant>> = StdMutex::new(Vec::new());

fn locked_out() -> bool {
    let mut f = FAILURES.lock().unwrap();
    f.retain(|t| t.elapsed() < Duration::from_secs(60));
    f.len() >= 5
}

fn record_failure() {
    FAILURES.lock().unwrap().push(Instant::now());
}

#[derive(PartialEq)]
enum Path {
    Trusted,
    Open,
    Password,
    Approved,
}

async fn recv_msg<S>(enc: &mut EncryptedStream<S>, secs: u64) -> Result<SessionMsg>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let bytes = tokio::time::timeout(Duration::from_secs(secs), enc.recv())
        .await
        .map_err(|_| anyhow::anyhow!("peer timed out"))??;
    Ok(decode_msg(&bytes)?)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

enum CaptureCmd {
    Select(u32),
}

async fn serve_host_enc<S, C, I, F>(mut enc: EncryptedStream<S>, opts: HostOptions, make_backend: F) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C: Capture + 'static,
    I: InputSink + 'static,
    F: FnOnce() -> Result<(C, I)> + Send,
{
    let fingerprint = hex(enc.remote_static());

    // ---- who is it ----
    let name = match recv_msg(&mut enc, 15).await? {
        SessionMsg::Hello { name } => name.chars().take(64).collect::<String>(),
        _ => "desconhecido".to_string(),
    };
    let peer = PeerInfo { name, fingerprint };

    // ---- authenticate ----
    let fail = |reason: &str| {
        record_failure();
        opts.emit(HostEvent::AuthFailed { peer: peer.clone(), reason: reason.to_string() });
        warn!(peer = %peer.name, fp = %&peer.fingerprint[..16.min(peer.fingerprint.len())], %reason, "access refused");
    };
    if locked_out() {
        let _ = enc.send(&encode_msg(&SessionMsg::AuthFailed)).await;
        opts.emit(HostEvent::AuthFailed { peer: peer.clone(), reason: "too many failed attempts".into() });
        bail!("locked out after repeated failures");
    }
    let salt: [u8; 16] = rand::random();
    enc.send(&encode_msg(&SessionMsg::AuthRequired { salt })).await?;
    let proof = match recv_msg(&mut enc, 30).await? {
        SessionMsg::AuthSubmit { proof } => proof,
        _ => Vec::new(),
    };
    let typed_empty = proof == auth_proof(&salt, "");
    let pw_ok = !opts.password.is_empty() && proof == auth_proof(&salt, &opts.password);

    let path = if opts.trusted.iter().any(|t| t.eq_ignore_ascii_case(&peer.fingerprint)) {
        Some(Path::Trusted)
    } else if opts.open_access {
        Some(Path::Open)
    } else if pw_ok {
        Some(Path::Password)
    } else if typed_empty {
        match &opts.approver {
            Some(ask) => {
                let allowed = tokio::time::timeout(Duration::from_secs(120), ask(peer.clone()))
                    .await
                    .unwrap_or(false);
                allowed.then_some(Path::Approved)
            }
            None => None,
        }
    } else {
        None
    };
    let Some(path) = path else {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let _ = enc.send(&encode_msg(&SessionMsg::AuthFailed)).await;
        fail(if typed_empty { "denied or no approver" } else { "wrong password" });
        bail!("controller failed authentication");
    };

    // ---- second factor ----
    if path != Path::Approved {
        if let Some(secret) = &opts.totp_secret {
            enc.send(&encode_msg(&SessionMsg::TwoFaRequired)).await?;
            let code = match recv_msg(&mut enc, 120).await? {
                SessionMsg::TwoFaSubmit { code } => code,
                _ => String::new(),
            };
            if !totp::verify(secret, &code, totp::now()) {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = enc.send(&encode_msg(&SessionMsg::AuthFailed)).await;
                fail("wrong 2FA code");
                bail!("wrong 2FA code");
            }
        }
    }
    enc.send(&encode_msg(&SessionMsg::AuthOk)).await?;

    // ---- capabilities ----
    let (want_h264, want_audio) = match recv_msg(&mut enc, 10).await {
        Ok(SessionMsg::Caps { h264, audio }) => (h264, audio),
        _ => (false, false),
    };

    // ---- backend (only now that the peer is accepted) ----
    let (mut capture, mut input) = make_backend()?;
    let permissions = opts.permissions;
    enc.send(&encode_msg(&SessionMsg::Permissions(permissions))).await?;
    let (width, height) = (capture.width(), capture.height());
    enc.send(&encode_msg(&SessionMsg::ScreenInfo { width, height })).await?;
    let monitors = capture.monitors();
    if !monitors.is_empty() {
        enc.send(&encode_msg(&SessionMsg::Monitors { list: monitors.clone() })).await?;
    }
    let area0 = current_area(&monitors, width, height);
    input.set_area(area0.0, area0.1, area0.2, area0.3);
    info!(peer = %peer.name, width, height, "host session established");

    let (tx, mut rx) = enc.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    opts.emit(HostEvent::Connected {
        peer: peer.clone(),
        session: HostSessionHandle { out: out_tx.clone(), cancel: cancel_tx },
    });

    // Publica o canal de saída desta sessão para a UI do host (envio de
    // arquivos/clipboard ao controlador). Limpo ao desconectar.
    if let Some(slot) = &opts.link_out {
        *slot.lock().unwrap() = Some(HostLink { out: out_tx.clone(), rt: tokio::runtime::Handle::current() });
    }

    // Writer: the single owner of the encrypted sender.
    let mut sender = tx;
    let writer = tokio::spawn(async move {
        while let Some(bytes) = out_rx.recv().await {
            if sender.send(&bytes).await.is_err() {
                break;
            }
        }
    });

    // Capture + encode, with monitor switching.
    let (area_tx, mut area_rx) = watch::channel(area0);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<CaptureCmd>(4);
    let frame_tx = out_tx.clone();
    let fps = opts.fps.max(1);
    let use_h264 = want_h264 && opts.prefer_h264 && crate::codec_h264::available();
    let capturer = tokio::spawn(async move {
        let mut vw = VideoEncoder::new(width, height, fps, use_h264);
        let (mut cur_w, mut cur_h) = (width, height);
        let mut ticker = tokio::time::interval(Duration::from_millis(1000 / fps as u64));
        let mut n: u64 = 0;
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(CaptureCmd::Select(i)) => {
                            if let Err(e) = capture.select_monitor(i).await {
                                warn!(error = %e, "monitor switch failed");
                                continue;
                            }
                            let (w, h) = (capture.width(), capture.height());
                            cur_w = w;
                            cur_h = h;
                            vw = VideoEncoder::new(w, h, fps, use_h264);
                            let mons = capture.monitors();
                            let _ = area_tx.send(current_area(&mons, w, h));
                            let _ = frame_tx.send(encode_msg(&SessionMsg::ScreenInfo { width: w, height: h })).await;
                            if !mons.is_empty() {
                                let _ = frame_tx.send(encode_msg(&SessionMsg::Monitors { list: mons })).await;
                            }
                        }
                        None => break,
                    }
                    continue;
                }
            }
            let buf = match capture.frame().await {
                Ok(b) => b,
                Err(e) => {
                    warn!(error = %e, "frame capture failed");
                    continue;
                }
            };
            // Resolution can change mid-session (display settings, rotation):
            // follow it with a new encoder and tell the controller.
            let (cw, ch) = (capture.width(), capture.height());
            if (cw, ch) != (cur_w, cur_h) && buf.len() == (cw as usize * ch as usize * 4) {
                cur_w = cw;
                cur_h = ch;
                vw = VideoEncoder::new(cw, ch, fps, use_h264);
                let mons = capture.monitors();
                let _ = area_tx.send(current_area(&mons, cw, ch));
                let _ = frame_tx.send(encode_msg(&SessionMsg::ScreenInfo { width: cw, height: ch })).await;
            }
            if buf.len() != cur_w as usize * cur_h as usize * 4 {
                continue;
            }
            n += 1;
            let force_key = n % (fps as u64 * 3) == 0;
            let msg = match vw.encode(&buf, force_key) {
                Ok(Some(m)) => m,
                Ok(None) => continue,
                Err(e) => {
                    warn!(error = %e, "encode failed");
                    continue;
                }
            };
            if frame_tx.send(encode_msg(&msg)).await.is_err() {
                break;
            }
        }
    });

    // Audio (loopback) → controller.
    let mut _audio: Option<AudioCapture> = None;
    let mut audio_task = None;
    if want_audio && permissions.audio && crate::audio::available() {
        let (atx, mut arx) = mpsc::channel::<(u32, u16, Vec<u8>)>(64);
        match AudioCapture::start(atx) {
            Ok(a) => {
                _audio = Some(a);
                let otx = out_tx.clone();
                audio_task = Some(tokio::spawn(async move {
                    while let Some((rate, channels, pcm)) = arx.recv().await {
                        if otx.send(encode_msg(&SessionMsg::Audio { rate, channels, pcm })).await.is_err() {
                            break;
                        }
                    }
                }));
            }
            Err(e) => warn!(error = %e, "audio capture unavailable"),
        }
    }

    // Clipboard (text) both ways.
    let (clip_tx, mut clip_rx) = mpsc::channel::<String>(8);
    let clipboard = if permissions.clipboard { ClipboardBridge::start(clip_tx) } else { None };
    let clip_out = out_tx.clone();
    let clip_task = tokio::spawn(async move {
        while let Some(t) = clip_rx.recv().await {
            if clip_out.send(encode_msg(&SessionMsg::Clipboard(t))).await.is_err() {
                break;
            }
        }
    });

    // Receive loop.
    let mut incoming: Option<IncomingFile> = None;
    let mut area = area0;
    // The cancel sender lives in the HostSessionHandle; if nobody kept it, the
    // channel just closes — that must not end the session.
    let mut cancel_alive = true;
    let result: Result<()> = loop {
        let bytes = tokio::select! {
            r = rx.recv() => match r {
                Ok(b) => b,
                Err(_) => break Ok(()),
            },
            r = cancel_rx.changed(), if cancel_alive => {
                match r {
                    Ok(()) if *cancel_rx.borrow() => {
                        info!("session ended by the host");
                        break Ok(());
                    }
                    Ok(()) => continue,
                    Err(_) => {
                        cancel_alive = false;
                        continue;
                    }
                }
            }
        };
        if area_rx.has_changed().unwrap_or(false) {
            area = *area_rx.borrow_and_update();
            input.set_area(area.0, area.1, area.2, area.3);
        }
        let msg = match decode_msg(&bytes) {
            Ok(m) => m,
            Err(e) => {
                warn!(error = %e, "undecodable session message");
                continue;
            }
        };
        match msg {
            SessionMsg::Input(ev) => {
                if permissions.control {
                    if let Err(e) = input.inject(ev) {
                        warn!(error = %e, "input injection failed");
                    }
                }
            }
            SessionMsg::Pointer { x, y } => {
                if !permissions.control && opts.show_pointer {
                    opts.emit(HostEvent::Pointer { x, y, area });
                }
            }
            SessionMsg::SelectMonitor { index } => {
                let _ = cmd_tx.send(CaptureCmd::Select(index)).await;
            }
            SessionMsg::SendSas => {
                if permissions.control {
                    if let Err(e) = input.send_sas() {
                        warn!(error = %e, "Ctrl+Alt+Del failed");
                        let _ = out_tx.send(encode_msg(&SessionMsg::Chat(format!("[Ctrl+Alt+Del indisponível: {e}]")))).await;
                    }
                }
            }
            SessionMsg::Clipboard(text) => {
                if let Some(c) = &clipboard {
                    c.set(text);
                }
            }
            SessionMsg::Chat(text) => {
                info!(from = %peer.name, chat = %text);
                opts.emit(HostEvent::Chat { peer: peer.clone(), text });
            }
            SessionMsg::Ping => {
                let _ = out_tx.send(encode_msg(&SessionMsg::Pong)).await;
            }
            SessionMsg::FileOffer { id, name, size } => {
                if permissions.file_transfer {
                    incoming = Some(IncomingFile::new(id, sanitize(&name), size));
                } else {
                    let _ = out_tx
                        .send(encode_msg(&SessionMsg::FileCancel { id, reason: "file transfer not permitted".into() }))
                        .await;
                }
            }
            SessionMsg::FileChunk { id, offset, data } => {
                if let Some(f) = &mut incoming {
                    if f.id == id {
                        if let Err(e) = f.push(offset, &data) {
                            warn!(error = %e, "bad file chunk");
                            incoming = None;
                        }
                    }
                }
            }
            SessionMsg::FileDone { .. } => {
                if let Some(f) = incoming.take() {
                    let name = f.name.clone();
                    match f.finish() {
                        Ok(data) => {
                            let path = opts.download_dir.join(&name);
                            if let Err(e) = tokio::fs::write(&path, &data).await {
                                warn!(error = %e, ?path, "writing received file failed");
                            } else {
                                info!(?path, "received file");
                                opts.emit(HostEvent::FileReceived { path });
                            }
                        }
                        Err(e) => warn!(error = %e, "incomplete file"),
                    }
                }
            }
            _ => {}
        }
    };

    capturer.abort();
    writer.abort();
    clip_task.abort();
    if let Some(t) = audio_task {
        t.abort();
    }
    drop(clipboard);
    if let Some(slot) = &opts.link_out {
        *slot.lock().unwrap() = None;
    }
    opts.emit(HostEvent::Disconnected { peer });
    result
}

/// The captured monitor's virtual-desktop rect (falls back to 0,0,w,h).
fn current_area(monitors: &[MonitorInfo], w: u32, h: u32) -> (i32, i32, u32, u32) {
    monitors
        .iter()
        .find(|m| m.current)
        .map(|m| (m.x, m.y, m.width, m.height))
        .unwrap_or((0, 0, w, h))
}

/// Tile codec or H.264, behind one interface. Falls back to tiles if H.264 fails.
enum VideoEncoder {
    Tiles(Encoder),
    H264 { enc: H264Encoder, w: u32, h: u32, fallback: Option<Encoder> },
}

impl VideoEncoder {
    fn new(w: u32, h: u32, fps: u32, h264: bool) -> Self {
        if h264 {
            let kbps = ((w as u64 * h as u64 * fps as u64) / 20_000).clamp(800, 6000) as u32;
            match H264Encoder::new(w, h, fps, kbps) {
                Ok(enc) => return VideoEncoder::H264 { enc, w, h, fallback: None },
                Err(e) => warn!(error = %e, "H.264 unavailable, using tile codec"),
            }
        }
        VideoEncoder::Tiles(Encoder::new(w, h))
    }

    fn encode(&mut self, bgra: &[u8], force_key: bool) -> Result<Option<SessionMsg>> {
        match self {
            VideoEncoder::Tiles(e) => {
                if force_key {
                    e.force_keyframe();
                }
                Ok(Some(SessionMsg::Frame(e.encode(bgra)?)))
            }
            VideoEncoder::H264 { enc, w, h, fallback } => {
                if let Some(t) = fallback {
                    return Ok(Some(SessionMsg::Frame(t.encode(bgra)?)));
                }
                match enc.encode(bgra, force_key) {
                    Ok((data, key)) if !data.is_empty() => {
                        Ok(Some(SessionMsg::FrameH264 { width: *w, height: *h, key, data }))
                    }
                    Ok(_) => Ok(None),
                    Err(e) => {
                        warn!(error = %e, "H.264 encode failed; switching to tile codec");
                        let mut t = Encoder::new(*w, *h);
                        let f = t.encode(bgra)?;
                        *fallback = Some(t);
                        Ok(Some(SessionMsg::Frame(f)))
                    }
                }
            }
        }
    }
}

/// Strip any path components from a peer-supplied file name.
fn sanitize(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download.bin".to_string())
}
