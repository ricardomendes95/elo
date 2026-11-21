//! Elo — single-window, portable remote-access app.
//!
//! One executable is both ends:
//!   * **This computer** (host): accepts connections by P2P address (internet)
//!     or LAN IP, with password / approval / paired devices / 2FA. When the
//!     Windows service is installed the service hosts instead (works with the
//!     screen locked) and this window is its UI (approvals, banner, pointer).
//!   * **Remote** (controller): connect to another machine by address, short ID
//!     or IP, then view and control it (multi-monitor, audio, clipboard, files,
//!     chat, Ctrl+Alt+Del).
//!
//! Async work runs on a background Tokio runtime; the egui thread drains
//! decoded frames and host events each repaint.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod events;
mod hostrun;
mod ipc;
mod keymap;
mod logging;
mod logo_pixels;
#[cfg(windows)]
mod service;
mod theme;
mod ui_home;
mod ui_session;
mod ui_settings;
mod worker;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{mpsc as smpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::Color32;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use elo_client_core::controller::{ConnectOptions, ControllerEvent, ControllerHandle, ERR_2FA_REQUIRED};
use elo_client_core::host::{ApproveFn, EventFn};

use config::{Config, TrustedDevice};
use events::{HostCtl, UiEvent};
use hostrun::{HostRun, HostSettings, DIRECT_PORT};
use theme::{palette, Palette};
use ui_session::Session;

type ConnectResult = Result<(ControllerHandle, mpsc::Receiver<ControllerEvent>, String), String>;

fn main() -> anyhow::Result<()> {
    // Hidden Windows service modes (handled before any window is created).
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--session-worker") {
            return worker::run_worker();
        }
        if args.iter().any(|a| a == "--service") {
            return service::run_dispatcher();
        }
        if args.iter().any(|a| a == "--install-service") {
            return service::install();
        }
        if args.iter().any(|a| a == "--uninstall-service") {
            return service::uninstall();
        }
    }

    logging::init(&config::config_dir());
    let icon = egui::IconData { rgba: logo_pixels::render_icon(64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Elo")
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    eframe::run_native("elo", options, Box::new(|_cc| Ok(Box::new(App::new()))))
        .map_err(|e| anyhow::anyhow!("eframe failed: {e}"))
}

pub fn download_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join("Downloads"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()))
}

/// A fresh 6-digit access code.
pub fn random_code() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..6).map(|_| rng.gen_range(b'0'..=b'9') as char).collect()
}

/// Open a folder in the OS file manager.
pub fn open_folder(path: &std::path::Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// Best-effort local LAN IP (no packets are actually sent).
fn local_ip() -> String {
    use std::net::UdpSocket;
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

/// Today as YYYY-MM-DD (local time is not needed here).
fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Group a hex fingerprint for reading: "a1b2 c3d4 e5f6 …".
pub fn short_fp(fp: &str) -> String {
    fp.chars()
        .take(20)
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Human-friendly connection errors.
fn friendly_error(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("invalid node id") {
        "Endereço inválido. Cole o endereço Elo completo, ou o ID curto de 9 dígitos (com diretório configurado).".into()
    } else if l.contains("is not online") {
        "Esse ID não está online agora.".into()
    } else if l.contains("connecting") || l.contains("timed out") || l.contains("tempo esgotado") || l.contains("refused") || l.contains("recusou") {
        format!("Não foi possível alcançar o dispositivo (desligado, offline ou endereço errado). Detalhe: {e}")
    } else {
        e.to_string()
    }
}

pub enum Conn {
    Idle,
    Connecting(smpsc::Receiver<ConnectResult>),
    Active(Box<Session>),
    Failed(String),
}

#[derive(PartialEq, Clone, Copy)]
pub enum View {
    Home,
    Favorites,
    Settings,
}

#[derive(PartialEq, Clone, Copy)]
pub enum SettingsTab {
    General,
    Security,
    Connection,
    Service,
    Logs,
    About,
}

pub struct PendingApproval {
    pub name: String,
    pub fp: String,
    pub reply: Option<tokio::sync::oneshot::Sender<bool>>,
    pub trust: bool,
}

/// Someone connected to THIS computer.
pub struct HostSessionUi {
    pub name: String,
    pub fp: String,
    pub ctl: HostCtl,
    pub since: Instant,
    pub chat: Vec<String>,
    pub chat_input: String,
    pub chat_open: bool,
}

pub struct PointerState {
    pub x: i32,
    pub y: i32,
    pub area: (i32, i32, u32, u32),
    pub at: Instant,
}

/// 2FA enrollment in progress.
pub struct Enroll {
    pub secret: Vec<u8>,
    pub uri: String,
    pub code: String,
    pub error: Option<String>,
}

pub struct App {
    pub rt: Runtime,
    pub pal: Palette,

    // Persistent configuration.
    pub config: Config,
    pub draft: Config,
    pub live: Arc<Mutex<Config>>,

    // This computer.
    pub service_mode: bool,
    pub my_lan: String,
    pub my_node: Arc<Mutex<String>>,
    pub my_short_id: String,
    pub session_password: Arc<Mutex<String>>,
    pub host_status: Arc<Mutex<String>>,
    pub host_started: bool,
    pub node: Arc<Mutex<Option<elo_p2p::Node>>>,

    // Host-side UI.
    ui_tx: smpsc::Sender<UiEvent>,
    ui_rx: smpsc::Receiver<UiEvent>,
    pub approvals: VecDeque<PendingApproval>,
    pub host_session: Option<HostSessionUi>,
    pub pointer: Option<PointerState>,
    pub toasts: Vec<(String, Instant)>,

    // Navigation.
    pub view: View,
    pub settings_tab: SettingsTab,

    // Controller.
    pub remote_addr: String,
    pub remote_password: String,
    pub mode_lan: bool,
    pub fav_name: String,
    pub conn: Conn,
    pub need_2fa: bool,
    pub twofa_code: String,

    // Favorites presence (address -> online).
    pub presence: Arc<Mutex<HashMap<String, bool>>>,

    pub enroll: Option<Enroll>,
}

impl App {
    fn new() -> Self {
        let mut config = Config::load();
        if config.short_id.len() != 9 {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            config.short_id = (0..9).map(|_| rng.gen_range(b'0'..=b'9') as char).collect();
            config.save();
        }
        #[cfg(windows)]
        let service_mode = service::is_installed();
        #[cfg(not(windows))]
        let service_mode = false;

        let (my_node, my_short_id) = if service_mode {
            let sdir = config::service_dir();
            let id = elo_p2p::id_from_secret_file(&sdir.join("node.key")).unwrap_or_else(|| "—".into());
            let sc = Config::load_from(&sdir);
            let short = if sc.short_id.len() == 9 { sc.short_id } else { config.short_id.clone() };
            (id, short)
        } else {
            ("iniciando…".to_string(), config.short_id.clone())
        };
        let (ui_tx, ui_rx) = smpsc::channel();
        let light = config.theme == "light";
        App {
            rt: Runtime::new().expect("tokio runtime"),
            pal: palette(light),
            draft: config.clone(),
            live: Arc::new(Mutex::new(config.clone())),
            config,
            service_mode,
            my_lan: local_ip(),
            my_node: Arc::new(Mutex::new(my_node)),
            my_short_id,
            session_password: Arc::new(Mutex::new(random_code())),
            host_status: Arc::new(Mutex::new("iniciando…".into())),
            host_started: false,
            node: Arc::new(Mutex::new(None)),
            ui_tx,
            ui_rx,
            approvals: VecDeque::new(),
            host_session: None,
            pointer: None,
            toasts: Vec::new(),
            view: View::Home,
            settings_tab: SettingsTab::General,
            remote_addr: String::new(),
            remote_password: String::new(),
            mode_lan: false,
            fav_name: String::new(),
            conn: Conn::Idle,
            need_2fa: false,
            twofa_code: String::new(),
            presence: Arc::new(Mutex::new(HashMap::new())),
            enroll: None,
        }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push((text.into(), Instant::now()));
    }

    /// Persist a new configuration (user dir, service dir in service mode) and
    /// apply it to the running host.
    pub fn save_config(&mut self, cfg: Config) {
        let autostart_changed = cfg.autostart != self.config.autostart;
        let otp_turned_on = cfg.otp && !self.config.otp;
        self.config = cfg;
        self.config.save();
        if self.service_mode {
            self.config.save_to(&config::service_dir());
            if Config::load_from(&config::service_dir()).allow_control != self.config.allow_control {
                self.toast("Não consegui gravar a configuração do serviço (permissão). Reinstale o serviço.");
            }
        }
        *self.live.lock().unwrap() = self.config.clone();
        if autostart_changed {
            if let Err(e) = config::set_autostart(self.config.autostart) {
                self.toast(format!("Iniciar com o sistema: {e}"));
            }
        }
        if otp_turned_on {
            *self.session_password.lock().unwrap() = random_code();
        }
    }

    /// Start hosting (in-process) or, with the service installed, become its UI.
    fn start_host(&mut self) {
        self.host_started = true;
        let node_slot = self.node.clone();
        let status = self.host_status.clone();

        if self.service_mode {
            *status.lock().unwrap() = "online — serviço do Windows (funciona com a tela bloqueada)".into();
            ipc::spawn_gui_server(&self.rt, config::service_dir(), self.ui_tx.clone());
            self.rt.spawn(async move {
                match elo_p2p::bind_node_ephemeral().await {
                    Ok(n) => *node_slot.lock().unwrap() = Some(n),
                    Err(e) => tracing::warn!(error = %e, "P2P indisponível para conexões de saída"),
                }
            });
        } else {
            let my_node = self.my_node.clone();
            let keypair = hostrun::load_or_create_keypair(&config::config_dir());
            let live = self.live.clone();
            let pw = self.session_password.clone();
            let ui_tx = self.ui_tx.clone();
            let online = format!("online — LAN {}:{DIRECT_PORT} · Internet pronto", self.my_lan);
            let directory = self.config.directory.clone();
            let short = self.my_short_id.clone();
            self.rt.spawn(async move {
                let node = match elo_p2p::bind_node(&config::config_dir().join("node.key")).await {
                    Ok(n) => n,
                    Err(e) => {
                        *status.lock().unwrap() = format!("P2P indisponível: {e}");
                        *my_node.lock().unwrap() = "—".into();
                        return;
                    }
                };
                *my_node.lock().unwrap() = node.id();
                *node_slot.lock().unwrap() = Some(node.clone());
                hostrun::spawn_directory(directory, short, node.id());

                let live2 = live.clone();
                let pw2 = pw.clone();
                let settings: Arc<dyn Fn() -> HostSettings + Send + Sync> =
                    Arc::new(move || HostSettings::from_config(&live2.lock().unwrap(), &pw2.lock().unwrap()));
                let tx = ui_tx.clone();
                let approver: ApproveFn = Arc::new(move |peer| {
                    let tx = tx.clone();
                    Box::pin(async move {
                        let (otx, orx) = tokio::sync::oneshot::channel();
                        if tx.send(UiEvent::Approval { name: peer.name, fp: peer.fingerprint, reply: otx }).is_err() {
                            return false;
                        }
                        orx.await.unwrap_or(false)
                    })
                });
                let tx = std::sync::Mutex::new(ui_tx.clone());
                let on_event: EventFn = Arc::new(move |ev| {
                    let _ = tx.lock().unwrap().send(events::from_host_event(ev));
                });
                let live3 = live.clone();
                let after_session = Arc::new(move || {
                    if live3.lock().unwrap().otp {
                        *pw.lock().unwrap() = random_code();
                    }
                });
                hostrun::run(HostRun {
                    node,
                    keypair,
                    download_dir: download_dir(),
                    settings,
                    approver,
                    on_event,
                    status,
                    online_text: online,
                    after_session,
                })
                .await;
            });
        }
        self.spawn_presence();
    }

    /// Check every ~45 s which favorites are online.
    fn spawn_presence(&self) {
        let live = self.live.clone();
        let presence = self.presence.clone();
        let node = self.node.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            loop {
                let (favs, dir) = {
                    let c = live.lock().unwrap();
                    (c.favorites.clone(), c.directory.clone())
                };
                let n = node.lock().unwrap().clone();
                for f in favs {
                    let online = if f.lan {
                        let addr = if f.address.contains(':') { f.address.clone() } else { format!("{}:{DIRECT_PORT}", f.address) };
                        matches!(tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(addr)).await, Ok(Ok(_)))
                    } else {
                        let digits: String = f.address.chars().filter(|c| c.is_ascii_digit()).collect();
                        if digits.len() == 9 && !dir.trim().is_empty() {
                            elo_client_core::rendezvous::resolve_directory(&dir, &digits).await.is_ok()
                        } else if let Some(n) = &n {
                            n.ping(&f.address, Duration::from_secs(8)).await
                        } else {
                            false
                        }
                    };
                    presence.lock().unwrap().insert(f.address.clone(), online);
                }
                tokio::time::sleep(Duration::from_secs(45)).await;
            }
        });
    }

    /// Connect to the address in the bar (optionally with a 2FA code).
    pub fn start_connect(&mut self, code: Option<String>) {
        let keypair = match elo_session::generate_keypair() {
            Ok(k) => k,
            Err(e) => {
                self.conn = Conn::Failed(e.to_string());
                return;
            }
        };
        let password = self.remote_password.clone();
        let name = if self.config.display_name.trim().is_empty() { config::current_username() } else { self.config.display_name.clone() };
        let opts = ConnectOptions { name: Some(name), totp_code: code };
        let target = self.remote_addr.trim().to_string();
        let (tx, rx) = smpsc::channel::<ConnectResult>();
        tracing::info!(%target, lan = self.mode_lan, "conectando");

        if self.mode_lan {
            use elo_client_core::controller::{run_controller_direct_opts, DirectControllerConfig};
            let host = if target.contains(':') { target.clone() } else { format!("{target}:{DIRECT_PORT}") };
            let cfg = DirectControllerConfig { host, keypair, password, download_dir: download_dir() };
            self.rt.spawn(async move {
                let r = run_controller_direct_opts(cfg, opts).await.map(|(h, e)| (h, e, target)).map_err(|e| format!("{e:#}"));
                let _ = tx.send(r);
            });
        } else {
            let node = self.node.lock().unwrap().clone();
            let Some(node) = node else {
                self.conn = Conn::Failed("P2P ainda iniciando — tente em instantes.".into());
                return;
            };
            let directory = self.config.directory.trim().to_string();
            self.rt.spawn(async move {
                let res = async {
                    let digits: String = target.chars().filter(|c| c.is_ascii_digit()).collect();
                    let node_id = if digits.len() == 9 && target.len() <= 11 {
                        if directory.is_empty() {
                            return Err("Para usar o ID curto, configure o servidor de diretório em Configurações → Conexão (ou cole o endereço completo).".to_string());
                        }
                        elo_client_core::rendezvous::resolve_directory(&directory, &digits).await.map_err(|e| format!("{e:#}"))?
                    } else {
                        target.clone()
                    };
                    let duplex = tokio::time::timeout(Duration::from_secs(30), node.connect(&node_id))
                        .await
                        .map_err(|_| "tempo esgotado ao conectar".to_string())?
                        .map_err(|e| format!("{e:#}"))?;
                    elo_client_core::controller::connect_on_opts(duplex, &keypair, &password, download_dir(), opts)
                        .await
                        .map(|(h, e)| (h, e, target.clone()))
                        .map_err(|e| format!("{e:#}"))
                }
                .await;
                let _ = tx.send(res);
            });
        }
        self.conn = Conn::Connecting(rx);
    }

    pub fn start_connect_fav(&mut self, fav: &config::Favorite) {
        self.remote_addr = fav.address.clone();
        self.remote_password = fav.password.clone();
        self.mode_lan = fav.lan;
        self.start_connect(None);
    }

    pub fn save_favorite(&mut self) {
        let addr = self.remote_addr.trim().to_string();
        if addr.is_empty() {
            self.toast("Informe o endereço antes de salvar.");
            return;
        }
        let name = if self.fav_name.trim().is_empty() { addr.chars().take(18).collect() } else { self.fav_name.trim().to_string() };
        let mut cfg = self.config.clone();
        cfg.favorites.retain(|f| f.address != addr);
        cfg.favorites.push(config::Favorite { name, address: addr, lan: self.mode_lan, password: self.remote_password.clone() });
        self.draft.favorites = cfg.favorites.clone();
        self.save_config(cfg);
        self.fav_name.clear();
        self.toast("Favorito salvo.");
    }

    /// Drain host events into UI state.
    fn pump_events(&mut self) {
        while let Ok(ev) = self.ui_rx.try_recv() {
            match ev {
                UiEvent::Approval { name, fp, reply } => {
                    self.approvals.push_back(PendingApproval { name, fp, reply: Some(reply), trust: false });
                }
                UiEvent::Connected { name, fp, ctl } => {
                    tracing::info!(peer = %name, "conectado a este computador");
                    self.toast(format!("{name} conectou a este computador."));
                    self.host_session = Some(HostSessionUi { name, fp, ctl, since: Instant::now(), chat: Vec::new(), chat_input: String::new(), chat_open: false });
                }
                UiEvent::Disconnected { name } => {
                    self.toast(format!("{name} desconectou."));
                    self.host_session = None;
                    self.pointer = None;
                }
                UiEvent::AuthFailed { name, reason } => {
                    let why = match reason.as_str() {
                        "wrong password" => "senha incorreta",
                        "wrong 2FA code" => "código 2FA incorreto",
                        "too many failed attempts" => "muitas tentativas (bloqueado por 1 min)",
                        _ => "recusado",
                    };
                    self.toast(format!("Acesso recusado: {name} ({why})."));
                }
                UiEvent::Pointer { x, y, area } => {
                    self.pointer = Some(PointerState { x, y, area, at: Instant::now() });
                }
                UiEvent::Chat { name, text } => {
                    if let Some(s) = &mut self.host_session {
                        s.chat.push(format!("{name}: {text}"));
                        s.chat_open = true;
                    }
                }
                UiEvent::File { path } => self.toast(format!("Arquivo recebido: {}", path.display())),
            }
        }
    }

    /// Resolve the pending connection, if any.
    fn poll_connect(&mut self) {
        let Conn::Connecting(rx) = &self.conn else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.conn = match result {
            Ok((handle, events, label)) => {
                tracing::info!(target = %label, "sessão de controle iniciada");
                Conn::Active(Box::new(Session::new(handle, events, label)))
            }
            Err(e) if e.contains(ERR_2FA_REQUIRED) => {
                self.need_2fa = true;
                self.twofa_code.clear();
                Conn::Idle
            }
            Err(e) => {
                tracing::warn!(error = %e, "falha ao conectar");
                Conn::Failed(friendly_error(&e))
            }
        };
    }

    fn approval_modal(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let Some(req) = self.approvals.front_mut() else { return };
        let mut decision: Option<bool> = None;
        modal(ctx, &pal, "approval", |ui| {
            ui.horizontal(|ui| {
                theme::avatar(ui, &pal, &req.name, 40.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Pedido de conexão").size(18.0).strong().color(pal.text));
                    ui.label(egui::RichText::new(format!("{} quer acessar este computador.", req.name)).color(pal.muted));
                });
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new(format!("Dispositivo: {}", short_fp(&req.fp))).monospace().small().color(pal.muted));
            ui.add_space(6.0);
            ui.checkbox(&mut req.trust, "Confiar neste dispositivo (próximas vezes entra sem pedir)");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Negar").min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
                if theme::primary_button(ui, &pal, "  Permitir  ").clicked() {
                    decision = Some(true);
                }
            });
        });
        if let Some(allow) = decision {
            let mut req = self.approvals.pop_front().unwrap();
            if allow && req.trust {
                let mut cfg = self.config.clone();
                if !cfg.is_trusted(&req.fp) {
                    cfg.trusted.push(TrustedDevice { name: req.name.clone(), fingerprint: req.fp.clone(), added: today() });
                    self.draft.trusted = cfg.trusted.clone();
                    self.save_config(cfg);
                }
            }
            tracing::info!(peer = %req.name, allow, trust = req.trust, "pedido de conexão respondido");
            if let Some(r) = req.reply.take() {
                let _ = r.send(allow);
            }
        }
    }

    fn twofa_modal(&mut self, ctx: &egui::Context) {
        if !self.need_2fa {
            return;
        }
        let pal = self.pal;
        let mut action: Option<bool> = None;
        modal(ctx, &pal, "twofa", |ui| {
            ui.label(egui::RichText::new("Verificação em duas etapas").size(18.0).strong().color(pal.text));
            ui.add_space(4.0);
            ui.label(egui::RichText::new("O computador remoto pede o código de 6 dígitos do app autenticador.").color(pal.muted));
            ui.add_space(10.0);
            let r = ui.add(egui::TextEdit::singleline(&mut self.twofa_code).hint_text("000000").desired_width(160.0).font(egui::TextStyle::Heading));
            r.request_focus();
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Cancelar").min_size(egui::vec2(100.0, 34.0))).clicked() {
                    action = Some(false);
                }
                if theme::primary_button(ui, &pal, "  Conectar  ").clicked() || enter {
                    action = Some(true);
                }
            });
        });
        match action {
            Some(true) => {
                self.need_2fa = false;
                let code = self.twofa_code.trim().to_string();
                self.start_connect(Some(code));
            }
            Some(false) => self.need_2fa = false,
            None => {}
        }
    }

    /// Floating notifications, bottom-right.
    fn toasts_ui(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|(_, t)| t.elapsed() < Duration::from_secs(6));
        if self.toasts.is_empty() {
            return;
        }
        let pal = self.pal;
        egui::Area::new("toasts".into())
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -18.0])
            .show(ctx, |ui| {
                for (text, _) in &self.toasts {
                    egui::Frame::none()
                        .fill(pal.surface)
                        .stroke(egui::Stroke::new(1.0_f32, pal.border))
                        .rounding(10.0)
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .shadow(egui::epaint::Shadow { offset: egui::vec2(0.0, 4.0), blur: 16.0, spread: 0.0, color: Color32::from_black_alpha(90) })
                        .show(ui, |ui| {
                            ui.set_max_width(360.0);
                            ui.label(egui::RichText::new(text).color(pal.text));
                        });
                    ui.add_space(6.0);
                }
            });
    }

    /// Draw the remote operator's pointer on top of this screen (view-only sessions).
    fn pointer_overlay(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.pointer else { return };
        if p.at.elapsed() > Duration::from_secs(4) || self.host_session.is_none() {
            self.pointer = None;
            return;
        }
        let name = self.host_session.as_ref().map(|s| s.name.clone()).unwrap_or_default();
        let ppp = ctx.pixels_per_point().max(0.5);
        let (ax, ay, aw, ah) = p.area;
        let pos = egui::pos2(ax as f32 / ppp, ay as f32 / ppp);
        let size = egui::vec2(aw.max(1) as f32 / ppp, ah.max(1) as f32 / ppp);
        let o = egui::pos2(p.x as f32 / ppp, p.y as f32 / ppp);
        let pal = self.pal;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("elo-pointer"),
            egui::ViewportBuilder::default()
                .with_title("Elo — ponteiro do operador")
                .with_decorations(false)
                .with_transparent(true)
                .with_always_on_top()
                .with_mouse_passthrough(true)
                .with_taskbar(false)
                .with_resizable(false)
                .with_position(pos)
                .with_inner_size(size),
            move |ctx, _class| {
                egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                    let pts = vec![o, o + egui::vec2(0.0, 20.0), o + egui::vec2(14.0, 14.0)];
                    ui.painter().add(egui::Shape::convex_polygon(pts, pal.accent, egui::Stroke::new(1.5_f32, Color32::WHITE)));
                    let galley = ui.painter().layout_no_wrap(name.clone(), egui::FontId::proportional(13.0), pal.on_accent);
                    let r = egui::Rect::from_min_size(o + egui::vec2(14.0, 18.0), galley.size() + egui::vec2(12.0, 6.0));
                    ui.painter().rect_filled(r, 6.0, pal.accent);
                    ui.painter().galley(r.min + egui::vec2(6.0, 3.0), galley, pal.on_accent);
                });
            },
        );
    }

    /// Chat window with the operator connected to this computer.
    fn host_chat_window(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let rt = &self.rt;
        let Some(s) = &mut self.host_session else { return };
        if !s.chat_open {
            return;
        }
        let mut open = true;
        egui::Window::new(format!("Chat — {}", s.name))
            .open(&mut open)
            .default_size([320.0, 300.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).show(ui, |ui| {
                    for l in &s.chat {
                        ui.label(egui::RichText::new(l).color(pal.text));
                    }
                });
                ui.separator();
                let r = ui.add(egui::TextEdit::singleline(&mut s.chat_input).desired_width(f32::INFINITY).hint_text("mensagem…"));
                if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) && !s.chat_input.trim().is_empty() {
                    let t = s.chat_input.trim().to_string();
                    s.chat.push(format!("você: {t}"));
                    s.ctl.chat(rt, t);
                    s.chat_input.clear();
                    r.request_focus();
                }
            });
        if !open {
            s.chat_open = false;
        }
    }
}

/// A centered modal card over a dimmed background.
pub fn modal(ctx: &egui::Context, pal: &Palette, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new((id, "dim")))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(140));
            ui.allocate_rect(screen, egui::Sense::click());
        });
    egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(pal.surface)
                .stroke(egui::Stroke::new(1.0_f32, pal.border))
                .rounding(14.0)
                .inner_margin(egui::Margin::same(22.0))
                .shadow(egui::epaint::Shadow { offset: egui::vec2(0.0, 8.0), blur: 28.0, spread: 0.0, color: Color32::from_black_alpha(140) })
                .show(ui, |ui| {
                    ui.set_max_width(420.0);
                    add(ui);
                });
        });
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent so the pointer overlay viewport can be see-through; the
        // main window is fully covered by its panels anyway.
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let light = self.draft.theme == "light";
        self.pal = palette(light);
        theme::apply_theme(ctx, &self.pal, light);
        if !self.host_started {
            self.start_host();
        }
        self.pump_events();
        self.poll_connect();

        if matches!(self.conn, Conn::Active(_)) {
            ui_session::drive_session(self, ctx);
        } else {
            ui_home::draw_shell(self, ctx);
        }
        self.host_chat_window(ctx);
        self.pointer_overlay(ctx);
        self.approval_modal(ctx);
        self.twofa_modal(ctx);
        self.toasts_ui(ctx);

        if matches!(self.conn, Conn::Active(_) | Conn::Connecting(_)) || self.pointer.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
