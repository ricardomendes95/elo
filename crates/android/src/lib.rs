//! Biblioteca nativa do app Android do Elo (papel de controller).
//!
//! Reaproveita `elo-client-core` (sessão, Noise, protocolo) e `elo-p2p` (iroh) e
//! expõe uma API fina ao Kotlin via UniFFI. O vídeo H.264, o áudio PCM e a área
//! de transferência chegam **crus** (`ConnectOptions::raw_media`): o app os entrega
//! ao `MediaCodec`, ao `AudioTrack` e ao `ClipboardManager`.
//!
//! As chamadas de [`Elo`] que fazem rede (`device_id`, `connect`, `is_online`)
//! **bloqueiam** até terminar: chame-as fora da thread principal
//! (`Dispatchers.IO`). Os métodos de [`Session`] não bloqueiam: enfileiram o
//! comando, que é enviado na ordem em que foi pedido.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use elo_client_core::controller::{connect_on_opts, run_controller_direct_opts, ConnectOptions, ControllerEvent, ControllerHandle, DirectControllerConfig, ERR_2FA_REQUIRED};
use elo_protocol::{InputEvent, MouseButton};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

uniffi::setup_scaffolding!();

/// Porta padrão do host Elo para conexão direta na LAN (igual ao desktop).
pub const DIRECT_PORT: u16 = 9000;

/// Erros devolvidos ao Kotlin.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EloError {
    /// O host tem 2FA: peça o código de 6 dígitos e chame `connect` de novo com ele.
    #[error("o host pede o código 2FA")]
    TwoFaRequired,
    /// Qualquer outra falha, com a mensagem em português para mostrar ao usuário.
    #[error("{msg}")]
    Failed { msg: String },
}

impl EloError {
    fn from_text(e: String) -> Self {
        if e.contains(ERR_2FA_REQUIRED) {
            EloError::TwoFaRequired
        } else {
            EloError::Failed { msg: e }
        }
    }
}

/// Um monitor do host (`Monitors`). Coordenadas no desktop virtual do host.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Monitor {
    pub index: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    /// O monitor que está sendo capturado agora.
    pub current: bool,
}

/// Dados da sessão logo após a autenticação.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SessionInfo {
    /// Tamanho do monitor capturado (pixels). As coordenadas de mouse são relativas a ele.
    pub width: u32,
    pub height: u32,
    /// Permissões concedidas pelo host.
    pub control: bool,
    pub file_transfer: bool,
    pub clipboard: bool,
    pub audio: bool,
    /// Impressão digital da chave estática (Noise) do host, em hexadecimal.
    pub fingerprint: String,
}

/// Eventos da sessão, chamados numa thread de fundo (não na principal).
#[uniffi::export(with_foreign)]
pub trait SessionListener: Send + Sync {
    /// Uma unidade de acesso H.264 Annex-B (`key` = IDR). `width`/`height` são o
    /// tamanho real do host (o fluxo codificado pode ter 1 px a mais se for ímpar).
    fn on_video_h264(&self, width: u32, height: u32, key: bool, data: Vec<u8>);
    /// Um quadro já decodificado em BGRA (`width*height*4`), quando o host usa o
    /// codec de tiles em vez de H.264.
    fn on_video_bgra(&self, width: u32, height: u32, bgra: Vec<u8>);
    /// O host trocou de monitor ou de resolução; o vídeo recomeça com um keyframe.
    fn on_screen_changed(&self, width: u32, height: u32);
    fn on_monitors(&self, monitors: Vec<Monitor>);
    /// PCM s16le intercalado (hoje 24 kHz mono, blocos de 20 ms). Não chega com o som desligado.
    fn on_audio(&self, rate: u32, channels: u16, pcm: Vec<u8>);
    /// Texto copiado no host.
    fn on_clipboard(&self, text: String);
    fn on_chat(&self, text: String);
    /// Arquivo enviado pelo host e salvo na pasta de downloads informada.
    fn on_file_received(&self, path: String);
    /// A sessão terminou (pelo host, pela rede ou por `close`). Último evento.
    fn on_disconnected(&self);
}

static LOG_INIT: Once = Once::new();

fn init_logging(data_dir: &std::path::Path) {
    LOG_INIT.call_once(|| {
        let path = data_dir.join("elo-android.log");
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = tracing_subscriber::fmt()
                .with_writer(Mutex::new(file))
                .with_ansi(false)
                .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
                .try_init();
        }
    });
}

/// Ponto de entrada: guarda a runtime Tokio e o nó P2P (iroh) do aparelho.
#[derive(uniffi::Object)]
pub struct Elo {
    rt: Arc<Runtime>,
    data_dir: PathBuf,
    node: Mutex<Option<elo_p2p::Node>>,
}

#[uniffi::export]
impl Elo {
    /// `data_dir`: pasta privada do app (`Context.filesDir`). Guarda a chave do
    /// nó P2P (`p2p.key`) e o log (`elo-android.log`).
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Result<Arc<Self>, EloError> {
        let data_dir = PathBuf::from(data_dir);
        let _ = std::fs::create_dir_all(&data_dir);
        init_logging(&data_dir);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("elo-rt")
            .build()
            .map_err(|e| EloError::Failed { msg: format!("runtime: {e}") })?;
        tracing::info!("elo-android iniciado");
        Ok(Arc::new(Self { rt: Arc::new(rt), data_dir, node: Mutex::new(None) }))
    }

    /// Endereço Elo (P2P) deste aparelho. Na primeira chamada cria a chave e
    /// liga o nó iroh (pode levar alguns segundos). Bloqueia.
    pub fn device_id(&self) -> Result<String, EloError> {
        Ok(self.node()?.id())
    }

    /// Conecta a um host e devolve a sessão. Bloqueia até autenticar (inclui a
    /// espera pela aprovação do dono, até ~2 min).
    ///
    /// - `target`: endereço Elo (P2P), ID curto de 9 dígitos (com `directory`) ou,
    ///   com `lan = true`, `IP` ou `IP:porta` (porta padrão 9000).
    /// - `password` vazia = pedir aprovação ao dono do host.
    /// - `name`: nome mostrado ao host (≤ 64 caracteres).
    /// - `totp_code`: código 2FA, quando o erro anterior foi `TwoFaRequired`.
    /// - `download_dir`: onde salvar arquivos que o host enviar.
    #[allow(clippy::too_many_arguments)]
    pub fn connect(
        &self,
        target: String,
        lan: bool,
        password: String,
        name: String,
        totp_code: Option<String>,
        directory: String,
        download_dir: String,
        listener: Arc<dyn SessionListener>,
    ) -> Result<Arc<Session>, EloError> {
        let keypair = elo_session::generate_keypair().map_err(|e| EloError::Failed { msg: e.to_string() })?;
        let name: String = name.trim().chars().take(64).collect();
        let opts = ConnectOptions {
            name: if name.is_empty() { Some("Android".into()) } else { Some(name) },
            totp_code: totp_code.filter(|c| !c.trim().is_empty()),
            raw_media: true,
        };
        let target = target.trim().to_string();
        let download_dir = PathBuf::from(download_dir);
        let _ = std::fs::create_dir_all(&download_dir);
        tracing::info!(%target, lan, "conectando");

        let result: Result<(ControllerHandle, mpsc::Receiver<ControllerEvent>), String> = if lan {
            let host = if target.contains(':') { target.clone() } else { format!("{target}:{DIRECT_PORT}") };
            let cfg = DirectControllerConfig { host, keypair, password, download_dir };
            self.rt.block_on(run_controller_direct_opts(cfg, opts)).map_err(|e| format!("{e:#}"))
        } else {
            let node = self.node()?;
            let directory = directory.trim().to_string();
            self.rt.block_on(async move {
                let node_id = resolve_target(&target, &directory).await?;
                let duplex = tokio::time::timeout(Duration::from_secs(30), node.connect(&node_id))
                    .await
                    .map_err(|_| "tempo esgotado ao conectar".to_string())?
                    .map_err(|e| format!("{e:#}"))?;
                connect_on_opts(duplex, &keypair, &password, download_dir, opts).await.map_err(|e| format!("{e:#}"))
            })
        };
        let (handle, events) = result.map_err(|e| {
            tracing::warn!(error = %e, "falha ao conectar");
            EloError::from_text(e)
        })?;
        tracing::info!("sessão iniciada");
        Ok(Session::start(&self.rt, handle, events, listener))
    }

    /// Presença de um favorito: `true` se o host responde agora. Bloqueia (até ~8 s).
    pub fn is_online(&self, target: String, lan: bool, directory: String) -> bool {
        let target = target.trim().to_string();
        if lan {
            let addr = if target.contains(':') { target } else { format!("{target}:{DIRECT_PORT}") };
            return self.rt.block_on(async {
                matches!(tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(addr)).await, Ok(Ok(_)))
            });
        }
        let digits: String = target.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() == 9 && target.len() <= 11 {
            // ID curto: presença = registrado no diretório (como no desktop).
            if directory.trim().is_empty() {
                return false;
            }
            return self.rt.block_on(elo_client_core::rendezvous::resolve_directory(directory.trim(), &digits)).is_ok();
        }
        let Ok(node) = self.node() else { return false };
        self.rt.block_on(node.ping(&target, Duration::from_secs(8)))
    }
}

impl Elo {
    fn node(&self) -> Result<elo_p2p::Node, EloError> {
        let mut slot = self.node.lock().unwrap();
        if let Some(n) = slot.as_ref() {
            return Ok(n.clone());
        }
        let path = self.data_dir.join("p2p.key");
        let node = self
            .rt
            .block_on(elo_p2p::bind_node(&path))
            .map_err(|e| EloError::Failed { msg: format!("P2P indisponível: {e:#}") })?;
        *slot = Some(node.clone());
        Ok(node)
    }
}

/// ID curto (9 dígitos, com ou sem espaços) → endereço Elo via diretório.
async fn resolve_target(target: &str, directory: &str) -> Result<String, String> {
    let digits: String = target.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 9 && target.len() <= 11 {
        if directory.is_empty() {
            return Err("Para usar o ID curto, configure o servidor de diretório em Configurações (ou cole o endereço completo).".into());
        }
        return elo_client_core::rendezvous::resolve_directory(directory, &digits).await.map_err(|e| format!("{e:#}"));
    }
    Ok(target.to_string())
}

/// Comandos enviados ao host, na ordem em que foram pedidos.
enum Cmd {
    Input(InputEvent),
    Pointer(i32, i32),
    Chat(String),
    Monitor(u32),
    Sas,
    Clipboard(String),
    File { id: u32, name: String, data: Vec<u8> },
}

/// Uma sessão de controle ativa.
#[derive(uniffi::Object)]
pub struct Session {
    info: SessionInfo,
    handle: ControllerHandle,
    cmds: mpsc::UnboundedSender<Cmd>,
    next_file: std::sync::atomic::AtomicU32,
}

impl Session {
    fn start(
        rt: &Runtime,
        handle: ControllerHandle,
        mut events: mpsc::Receiver<ControllerEvent>,
        listener: Arc<dyn SessionListener>,
    ) -> Arc<Self> {
        let p = handle.permissions;
        let info = SessionInfo {
            width: handle.screen.0,
            height: handle.screen.1,
            control: p.control,
            file_transfer: p.file_transfer,
            clipboard: p.clipboard,
            audio: p.audio,
            fingerprint: handle.peer_fingerprint.iter().map(|b| format!("{b:02x}")).collect(),
        };

        // Fila de comandos → host (uma tarefa, para manter a ordem dos eventos de input).
        let (cmds, mut rx) = mpsc::unbounded_channel::<Cmd>();
        let h = handle.clone();
        rt.spawn(async move {
            while let Some(cmd) = rx.recv().await {
                let r = match cmd {
                    Cmd::Input(ev) => h.send_input(ev).await,
                    Cmd::Pointer(x, y) => h.send_pointer(x, y).await,
                    Cmd::Chat(t) => h.send_chat(&t).await,
                    Cmd::Monitor(i) => h.select_monitor(i).await,
                    Cmd::Sas => h.send_sas().await,
                    Cmd::Clipboard(t) => h.send_clipboard(&t).await,
                    Cmd::File { id, name, data } => h.send_file(id, &name, &data).await,
                };
                if r.is_err() {
                    break;
                }
            }
        });

        // Eventos do host → Kotlin.
        rt.spawn(async move {
            while let Some(ev) = events.recv().await {
                match ev {
                    ControllerEvent::FrameH264 { width, height, key, data } => listener.on_video_h264(width, height, key, data),
                    ControllerEvent::Frame { width, height, bgra } => listener.on_video_bgra(width, height, bgra),
                    ControllerEvent::ScreenChanged { width, height } => listener.on_screen_changed(width, height),
                    ControllerEvent::Monitors(list) => listener.on_monitors(
                        list.into_iter()
                            .map(|m| Monitor {
                                index: m.index,
                                name: m.name,
                                x: m.x,
                                y: m.y,
                                width: m.width,
                                height: m.height,
                                primary: m.primary,
                                current: m.current,
                            })
                            .collect(),
                    ),
                    ControllerEvent::Audio { rate, channels, pcm } => listener.on_audio(rate, channels, pcm),
                    ControllerEvent::Clipboard(t) => listener.on_clipboard(t),
                    ControllerEvent::Chat(t) => listener.on_chat(t),
                    ControllerEvent::FileReceived { path } => listener.on_file_received(path.to_string_lossy().into_owned()),
                    ControllerEvent::Disconnected => {
                        tracing::info!("sessão encerrada");
                        listener.on_disconnected();
                        return;
                    }
                }
            }
            listener.on_disconnected();
        });

        Arc::new(Self { info, handle, cmds, next_file: std::sync::atomic::AtomicU32::new(1) })
    }

    fn push(&self, c: Cmd) {
        let _ = self.cmds.send(c);
    }
}

#[uniffi::export]
impl Session {
    pub fn info(&self) -> SessionInfo {
        self.info.clone()
    }

    /// Move o mouse para (x, y) em pixels do monitor capturado do host.
    pub fn mouse_move(&self, x: i32, y: i32) {
        self.push(Cmd::Input(InputEvent::MouseMove { x, y }));
    }

    /// `button`: 0 = esquerdo, 1 = direito, 2 = meio.
    pub fn mouse_button(&self, button: u8, down: bool) {
        let button = match button {
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            _ => MouseButton::Left,
        };
        self.push(Cmd::Input(InputEvent::MouseButton { button, down }));
    }

    /// Roda do mouse em "cliques" (o host multiplica por `WHEEL_DELTA` = 120); positivo = para cima/direita.
    pub fn scroll(&self, dx: i32, dy: i32) {
        self.push(Cmd::Input(InputEvent::Scroll { dx, dy }));
    }

    /// Tecla por código VK do Windows (`down` = pressionar / soltar).
    pub fn key(&self, vk: u16, down: bool) {
        self.push(Cmd::Input(InputEvent::Key { code: vk, down }));
    }

    /// Um caractere Unicode (escalar), digitado no host independente do layout.
    pub fn char(&self, code: u32) {
        self.push(Cmd::Input(InputEvent::Char { code }));
    }

    /// Digita um texto inteiro, um `Char` por code point.
    pub fn text(&self, text: String) {
        for c in text.chars() {
            self.push(Cmd::Input(InputEvent::Char { code: c as u32 }));
        }
    }

    /// Mostra nosso ponteiro na tela do host (sessões sem controle).
    pub fn pointer(&self, x: i32, y: i32) {
        self.push(Cmd::Pointer(x, y));
    }

    pub fn chat(&self, text: String) {
        self.push(Cmd::Chat(text));
    }

    pub fn select_monitor(&self, index: u32) {
        self.push(Cmd::Monitor(index));
    }

    /// Ctrl+Alt+Del no host (requer o serviço do Elo no Windows).
    pub fn send_sas(&self) {
        self.push(Cmd::Sas);
    }

    /// Liga/desliga o som (local: com o som desligado `on_audio` deixa de ser chamado).
    pub fn set_audio(&self, on: bool) {
        self.handle.set_audio(on);
    }

    /// Envia texto para a área de transferência do host (≤ 1 MiB).
    pub fn send_clipboard(&self, text: String) {
        if text.len() <= 1024 * 1024 {
            self.push(Cmd::Clipboard(text));
        }
    }

    /// Envia um arquivo para a pasta de downloads do host.
    pub fn send_file(&self, name: String, data: Vec<u8>) {
        let id = self.next_file.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.push(Cmd::File { id, name, data });
    }

    /// Encerra a sessão; `on_disconnected` é chamado em seguida. (Não se chama
    /// `close` porque o UniFFI já gera `AutoCloseable.close()`, que só libera o objeto.)
    pub fn hangup(&self) {
        self.handle.close();
    }
}

/// Um host Elo anunciado na LAN (descoberta por broadcast UDP).
#[derive(Debug, Clone, uniffi::Record)]
pub struct DiscoveredDevice {
    /// Nome amigável anunciado pelo host.
    pub name: String,
    /// IP do host na rede local.
    pub ip: String,
    /// Porta de conexão direta do host (normalmente 9000).
    pub port: u16,
    /// ID curto de 9 dígitos do host.
    pub short_id: String,
}

/// Descoberta de hosts Elo na rede local.
///
/// Escuta os anúncios UDP de `elo_client_core::discovery`. Chame [`Discovery::start`]
/// uma vez e leia [`Discovery::devices`] periodicamente; a escuta para no `drop`
/// (ou em [`Discovery::stop`]).
///
/// **Android:** receber broadcast UDP exige segurar um `WifiManager.MulticastLock`
/// enquanto a descoberta está ativa — adquira antes de `start` e libere depois de
/// `stop`/fechar.
#[derive(uniffi::Object)]
pub struct Discovery {
    handle: Mutex<Option<elo_client_core::discovery::DiscoveryHandle>>,
}

#[uniffi::export]
impl Discovery {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self { handle: Mutex::new(None) })
    }

    /// Começa a escutar os anúncios na LAN (idempotente). Bloqueia só para abrir o socket.
    pub fn start(&self) -> Result<(), EloError> {
        let mut slot = self.handle.lock().unwrap();
        if slot.is_some() {
            return Ok(());
        }
        let h = elo_client_core::discovery::listen()
            .map_err(|e| EloError::Failed { msg: format!("descoberta na rede indisponível: {e}") })?;
        *slot = Some(h);
        tracing::info!("descoberta LAN iniciada");
        Ok(())
    }

    /// Dispositivos vistos até agora (vazio se `start` ainda não foi chamado).
    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        let slot = self.handle.lock().unwrap();
        match slot.as_ref() {
            Some(h) => h
                .devices()
                .into_iter()
                .map(|d| DiscoveredDevice { name: d.name, ip: d.ip, port: d.port, short_id: d.short_id })
                .collect(),
            None => Vec::new(),
        }
    }

    /// Para a escuta e libera o socket (a descoberta também para no `drop`).
    pub fn stop(&self) {
        if self.handle.lock().unwrap().take().is_some() {
            tracing::info!("descoberta LAN parada");
        }
    }
}
