//! Papel de **host** no Android: o PC vê a tela do celular e o controla.
//!
//! O Kotlin captura a tela (`MediaProjection` → `ImageReader`) e entrega cada quadro
//! RGBA_8888 a [`Host::push_frame`]. O host aceita conexões na LAN (TCP 9000) e por
//! P2P (iroh), autentica (senha ou aprovação do dono, via [`HostListener::on_approval`])
//! e serve o último quadro recebido. O input remoto chega a [`HostListener::on_input`],
//! onde o `AccessibilityService` o converte em gestos.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use elo_client_core::host::{serve_host_opts, ApproveFn, EventFn, HostEvent, HostOptions, HostSessionHandle, PeerInfo};
use elo_client_core::{Capture, InputSink};
use elo_protocol::{InputEvent, MouseButton, Permissions};
use elo_session::Keypair;
use tokio::sync::{watch, Notify};

use crate::{Elo, EloError, DIRECT_PORT};

/// Evento de input remoto. Coordenadas em pixels do quadro capturado.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum HostInput {
    MouseMove { x: i32, y: i32 },
    /// `button`: 0 = esquerdo, 1 = direito, 2 = meio.
    MouseButton { button: u8, down: bool },
    /// Em "cliques" de roda; positivo = para cima/direita.
    Scroll { dx: i32, dy: i32 },
    /// Código VK do Windows.
    Key { code: u16, down: bool },
    /// Caractere Unicode (escalar).
    Char { code: u32 },
}

/// Eventos do host, chamados em threads de fundo (nunca bloqueie por muito tempo,
/// exceto em `on_approval`, que é feito para bloquear).
#[uniffi::export(with_foreign)]
pub trait HostListener: Send + Sync {
    /// Pedido de acesso sem senha. Bloqueia até o dono responder (o host desiste em ~2 min).
    fn on_approval(&self, name: String, fingerprint: String) -> bool;
    fn on_session_started(&self, peer: String);
    fn on_session_ended(&self);
    fn on_input(&self, ev: HostInput);
    fn on_status(&self, text: String);
}

struct Frame {
    w: u32,
    h: u32,
    bgra: Vec<u8>,
}

struct Shared {
    frame: Mutex<Option<Arc<Frame>>>,
    notify: Notify,
    session: Mutex<Option<HostSessionHandle>>,
    listener: Arc<dyn HostListener>,
}

/// Um host Elo em execução (aceita LAN e P2P até `stop`).
#[derive(uniffi::Object)]
pub struct Host {
    shared: Arc<Shared>,
    address: String,
    stop_tx: watch::Sender<bool>,
}

struct AndroidCapture {
    shared: Arc<Shared>,
    w: u32,
    h: u32,
}

#[async_trait::async_trait]
impl Capture for AndroidCapture {
    fn width(&self) -> u32 {
        self.w
    }
    fn height(&self) -> u32 {
        self.h
    }
    async fn frame(&mut self) -> Result<Vec<u8>> {
        loop {
            let notified = self.shared.notify.notified();
            let f = self.shared.frame.lock().unwrap().clone();
            if let Some(f) = f {
                self.w = f.w;
                self.h = f.h;
                return Ok(f.bgra.clone());
            }
            let _ = tokio::time::timeout(Duration::from_secs(2), notified).await;
        }
    }
}

struct ListenerInput(Arc<dyn HostListener>);

impl InputSink for ListenerInput {
    fn inject(&mut self, ev: InputEvent) -> Result<()> {
        let ev = match ev {
            InputEvent::MouseMove { x, y } => HostInput::MouseMove { x, y },
            InputEvent::MouseButton { button, down } => HostInput::MouseButton {
                button: match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                },
                down,
            },
            InputEvent::Scroll { dx, dy } => HostInput::Scroll { dx, dy },
            InputEvent::Key { code, down } => HostInput::Key { code, down },
            InputEvent::Char { code } => HostInput::Char { code },
        };
        self.0.on_input(ev);
        Ok(())
    }
}

/// Chave Noise estável do host (`host.key`), como no desktop.
fn load_or_create_keypair(dir: &Path) -> Keypair {
    let path = dir.join("host.key");
    if let Ok(b) = std::fs::read(&path) {
        if b.len() == 64 {
            return Keypair { private: b[0..32].to_vec(), public: b[32..64].to_vec() };
        }
    }
    let kp = elo_session::generate_keypair().expect("keypair");
    let mut bytes = kp.private.clone();
    bytes.extend_from_slice(&kp.public);
    let _ = std::fs::write(&path, &bytes);
    kp
}

/// ID curto de 9 dígitos, derivado da chave pública (estável).
fn short_id(kp: &Keypair) -> String {
    let mut v: u64 = 0;
    for b in kp.public.iter().take(8) {
        v = (v << 8) | *b as u64;
    }
    format!("{:09}", v % 1_000_000_000)
}

enum Incoming {
    Lan(tokio::net::TcpStream),
    P2p(elo_p2p::Duplex),
}

#[uniffi::export]
impl Elo {
    /// Começa a hospedar este aparelho: LAN TCP `0.0.0.0:9000`, P2P (iroh) e anúncio
    /// na descoberta da LAN. `password` vazia = pedir aprovação (`on_approval`).
    /// Bloqueia (liga o nó P2P na primeira vez): chame em `Dispatchers.IO`.
    pub fn start_host(&self, name: String, password: String, listener: Arc<dyn HostListener>) -> Result<Arc<Host>, EloError> {
        let node = self.node()?;
        let keypair = load_or_create_keypair(&self.data_dir);
        let name: String = name.trim().chars().take(64).collect();
        let name = if name.is_empty() { "Android".to_string() } else { name };
        presence_announce(name, effective_short(&keypair), true);
        let download_dir = self.data_dir.join("Recebidos");
        let _ = std::fs::create_dir_all(&download_dir);

        let shared = Arc::new(Shared {
            frame: Mutex::new(None),
            notify: Notify::new(),
            session: Mutex::new(None),
            listener: listener.clone(),
        });
        let (stop_tx, stop_rx) = watch::channel(false);
        let host = Arc::new(Host {
            shared: shared.clone(),
            address: node.id(),
            stop_tx,
        });
        tracing::info!("host Android iniciado");
        self.rt.spawn(host_loop(node, keypair, password, download_dir, shared, stop_rx));
        Ok(host)
    }
}

async fn host_loop(
    node: elo_p2p::Node,
    keypair: Keypair,
    password: String,
    download_dir: std::path::PathBuf,
    shared: Arc<Shared>,
    mut stop: watch::Receiver<bool>,
) {
    let listener = shared.listener.clone();
    let tcp = match tokio::net::TcpListener::bind(("0.0.0.0", DIRECT_PORT)).await {
        Ok(l) => Some(l),
        Err(e) => {
            tracing::warn!(error = %e, "porta LAN {DIRECT_PORT} indisponível");
            listener.on_status(format!("Porta {DIRECT_PORT} indisponível na rede local; só P2P"));
            None
        }
    };
    let waiting = "Aguardando conexões…".to_string();
    listener.on_status(waiting.clone());

    let approver: Option<ApproveFn> = if password.is_empty() {
        let l = listener.clone();
        Some(Arc::new(move |p: PeerInfo| {
            let l = l.clone();
            Box::pin(async move {
                tokio::task::spawn_blocking(move || l.on_approval(p.name, p.fingerprint)).await.unwrap_or(false)
            })
        }))
    } else {
        None
    };
    let on_event: EventFn = {
        let shared = shared.clone();
        Arc::new(move |ev: HostEvent| match ev {
            HostEvent::Connected { peer, session } => {
                *shared.session.lock().unwrap() = Some(session);
                shared.listener.on_session_started(peer.name);
            }
            HostEvent::Disconnected { .. } => {
                *shared.session.lock().unwrap() = None;
                shared.listener.on_session_ended();
            }
            HostEvent::AuthFailed { peer, reason } => {
                shared.listener.on_status(format!("Acesso recusado a {}: {reason}", peer.name));
            }
            _ => {}
        })
    };

    loop {
        let incoming: Result<Incoming> = tokio::select! {
            _ = stop.changed() => break,
            r = async {
                match &tcp {
                    Some(l) => l.accept().await.map(|(s, _)| Incoming::Lan(s)).map_err(anyhow::Error::from),
                    None => std::future::pending().await,
                }
            } => r,
            r = node.accept() => r.map(Incoming::P2p),
        };
        let incoming = match incoming {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(error = %e, "erro ao aceitar conexão");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let opts = HostOptions {
            password: password.clone(),
            open_access: false,
            trusted: Vec::new(),
            totp_secret: None,
            approver: approver.clone(),
            on_event: Some(on_event.clone()),
            permissions: Permissions { control: true, file_transfer: false, clipboard: false, audio: false },
            fps: 15,
            download_dir: download_dir.clone(),
            prefer_h264: false,
            show_pointer: false,
            link_out: None,
        };
        let sh = shared.clone();
        let factory = move || -> Result<(AndroidCapture, ListenerInput)> {
            // O quadro vem do ImageReader; espera um pouco se ainda não chegou nenhum.
            let t0 = Instant::now();
            let size = loop {
                if let Some(f) = sh.frame.lock().unwrap().as_ref() {
                    break (f.w, f.h);
                }
                if t0.elapsed() > Duration::from_secs(5) {
                    anyhow::bail!("a captura de tela ainda não começou");
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            Ok((AndroidCapture { shared: sh.clone(), w: size.0, h: size.1 }, ListenerInput(sh.listener.clone())))
        };
        let fut = async {
            match incoming {
                Incoming::Lan(s) => serve_host_opts(s, &keypair, opts, factory).await,
                Incoming::P2p(d) => serve_host_opts(d, &keypair, opts, factory).await,
            }
        };
        tokio::pin!(fut);
        let r = tokio::select! {
            r = &mut fut => r,
            _ = stop.changed() => {
                if let Some(s) = shared.session.lock().unwrap().take() {
                    s.disconnect();
                }
                let _ = tokio::time::timeout(Duration::from_secs(2), &mut fut).await;
                break;
            }
        };
        if let Err(e) = r {
            tracing::debug!(error = %e, "conexão encerrada");
        }
        // `HostEvent::Disconnected` pode não ser emitido em todos os caminhos de erro.
        if shared.session.lock().unwrap().take().is_some() {
            shared.listener.on_session_ended();
        }
        listener.on_status(waiting.clone());
    }
    tracing::info!("host Android parado");
}

#[uniffi::export]
impl Host {
    /// Endereço Elo (P2P) deste aparelho.
    pub fn address(&self) -> String {
        self.address.clone()
    }

    /// Recebe um quadro RGBA_8888 do `ImageReader` (`row_stride` em bytes, como em
    /// `Image.Plane.rowStride`), converte para BGRA compacto e guarda como último quadro.
    pub fn push_frame(&self, data: Vec<u8>, width: u32, height: u32, row_stride: u32) {
        let (w, h, stride) = (width as usize, height as usize, row_stride as usize);
        if w == 0 || h == 0 || stride < w * 4 || data.len() < (h - 1) * stride + w * 4 {
            return;
        }
        let mut bgra = vec![0u8; w * h * 4];
        for y in 0..h {
            let src = &data[y * stride..y * stride + w * 4];
            let dst = &mut bgra[y * w * 4..(y + 1) * w * 4];
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 0xff;
            }
        }
        *self.shared.frame.lock().unwrap() = Some(Arc::new(Frame { w: width, h: height, bgra }));
        self.shared.notify.notify_waiters();
    }

    /// Encerra a sessão em andamento (o host continua aceitando novas conexões).
    pub fn end_session(&self) {
        if let Some(s) = self.shared.session.lock().unwrap().clone() {
            s.disconnect();
        }
    }

    /// Para o host: encerra a sessão e fecha a porta. Na descoberta da LAN o
    /// aparelho segue visível, mas como "sem compartilhar a tela".
    pub fn stop(&self) {
        presence_set_hosting(false);
        let _ = self.stop_tx.send(true);
    }
}

// ---------------------------------------------------------------------------
// Presença na LAN: um único anúncio por processo, com "aceita conexões" ligado
// só enquanto o host roda. Assim o PC vê o celular sempre que o app está
// aberto, e a lista diz se o compartilhamento está ligado.
// ---------------------------------------------------------------------------

struct Presence {
    name: String,
    short: String,
    accepting: Arc<std::sync::atomic::AtomicBool>,
    _announcer: elo_client_core::discovery::Announcer,
}

static PRESENCE: Mutex<Option<Presence>> = Mutex::new(None);

/// Número registrado no diretório (preenchido por `register_short_id`).
static REGISTERED_SHORT: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn set_registered_short(s: String) {
    *REGISTERED_SHORT.lock().unwrap() = Some(s.clone());
    // Se já há anúncio na LAN com outro número, recria com o registrado.
    let mut slot = PRESENCE.lock().unwrap();
    if let Some(p) = slot.as_ref() {
        if p.short != s {
            let name = p.name.clone();
            let hosting = p.accepting.load(std::sync::atomic::Ordering::Relaxed);
            *slot = None;
            drop(slot);
            presence_announce(name, s, hosting);
        }
    }
}

/// Número a anunciar: o registrado, quando houver; senão o derivado da chave.
fn effective_short(kp: &Keypair) -> String {
    REGISTERED_SHORT.lock().unwrap().clone().unwrap_or_else(|| short_id(kp))
}

/// Garante o anúncio com este nome; `hosting` define se aceita conexões.
fn presence_announce(name: String, short_id: String, hosting: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut slot = PRESENCE.lock().unwrap();
    match slot.as_ref() {
        Some(p) if p.name == name && p.short == short_id => p.accepting.store(hosting, Ordering::Relaxed),
        _ => {
            // Troca de nome (ou primeiro anúncio): recria. O anterior para no drop.
            *slot = None;
            let accepting = Arc::new(AtomicBool::new(hosting));
            let announcer =
                elo_client_core::discovery::start_with_flag(name.clone(), short_id.clone(), DIRECT_PORT, accepting.clone());
            *slot = Some(Presence { name, short: short_id, accepting, _announcer: announcer });
        }
    }
}

fn presence_set_hosting(hosting: bool) {
    if let Some(p) = PRESENCE.lock().unwrap().as_ref() {
        p.accepting.store(hosting, std::sync::atomic::Ordering::Relaxed);
    }
}

fn presence_hosting() -> bool {
    PRESENCE.lock().unwrap().as_ref().map(|p| p.accepting.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(false)
}

#[uniffi::export]
impl Elo {
    /// Anuncia este aparelho na LAN enquanto o app está aberto (para o PC vê-lo),
    /// mesmo sem compartilhar a tela. Idempotente.
    pub fn announce_presence(&self, name: String) {
        let keypair = load_or_create_keypair(&self.data_dir);
        let name: String = name.trim().chars().take(64).collect();
        let name = if name.is_empty() { "Android".to_string() } else { name };
        let hosting = presence_hosting();
        presence_announce(name, effective_short(&keypair), hosting);
    }

    /// Para o anúncio, a menos que o host esteja rodando (aí ele continua).
    pub fn stop_presence(&self) {
        let mut slot = PRESENCE.lock().unwrap();
        let hosting = slot.as_ref().map(|p| p.accepting.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(false);
        if !hosting {
            *slot = None;
        }
    }
}
