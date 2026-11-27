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
mod defender;
mod events;
mod hostrun;
mod install;
mod ipc;
mod keymap;
mod logging;
mod logo_pixels;
mod power;
#[cfg(windows)]
mod service;
mod theme;
mod tray;
mod ui_home;
mod ui_manual;
mod ui_session;
mod ui_settings;
mod watchdog;
mod worker;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{mpsc as smpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::Color32;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use elo_client_core::controller::{ConnectOptions, ControllerEvent, ControllerHandle, ERR_2FA_REQUIRED};
use elo_client_core::discovery::{Announcer, DiscoveryHandle};
use elo_client_core::host::{ApproveFn, EventFn};
use elo_client_core::HostLink;

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
        // Instalação/desinstalação no Windows: rodam ELEVADAS (relançadas via
        // UAC) e NÃO abrem janela.
        if args.iter().any(|a| a == "--install-app") {
            install::run_install();
            return Ok(());
        }
        if args.iter().any(|a| a == "--uninstall-app") {
            install::run_uninstall();
            return Ok(());
        }
        // Instância única: só para o modo GUI normal (e `--tray`, que é GUI). Se
        // já houver uma instância, traz a janela existente para frente e sai.
        if !acquire_single_instance() {
            return Ok(());
        }
    }

    // `--tray`: inicia minimizado na bandeja (janela escondida). O ícone da
    // bandeja é criado normalmente e "Abrir Elo" mostra a janela.
    let start_hidden = std::env::args().any(|a| a == "--tray");

    logging::init(&config::config_dir());
    let icon = egui::IconData { rgba: logo_pixels::render_icon(64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Elo")
            .with_visible(!start_hidden)
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    eframe::run_native("elo", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
        .map_err(|e| anyhow::anyhow!("eframe failed: {e}"))
}

/// Acha o `HWND` da janela principal desta thread (a do eframe/winit). As
/// janelas auxiliares e message-only do winit não têm `WS_CAPTION`; a janela
/// real do app (título "Elo") tem. Devolve 0 se não achar.
#[cfg(windows)]
fn find_main_window() -> isize {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetWindowLongPtrW, GWL_STYLE, WS_CAPTION,
    };

    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut isize);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 {
            *out = hwnd.0 as isize;
            return BOOL(0); // achou: para a enumeração
        }
        BOOL(1) // continua
    }

    let mut found: isize = 0;
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(cb),
            LPARAM(&mut found as *mut isize as isize),
        );
    }
    found
}

/// Stub fora do Windows: nunca há HWND a descobrir.
#[cfg(not(windows))]
fn find_main_window() -> isize {
    0
}

/// Instância única (modo GUI). Cria um mutex nomeado por máquina/sessão; se já
/// existe outra instância, traz a janela "Elo" dela para frente e devolve
/// `false` (o chamador deve encerrar sem abrir uma segunda janela). Devolve
/// `true` quando esta é a primeira instância; nesse caso o handle do mutex é
/// vazado de propósito para permanecer vivo por todo o processo.
#[cfg(windows)]
fn acquire_single_instance() -> bool {
    // `--restarted` (ex.: "Reiniciar como admin"): a instância anterior ainda
    // está saindo; espera ela soltar o mutex em vez de desistir na hora.
    let wait = std::env::args().any(|a| a == "--restarted");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        match try_single_instance(!wait) {
            Some(acquired) => return acquired,
            None if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(250)),
            None => return false,
        }
    }
}

/// `Some(true)` = primeira instância; `Some(false)` = já existe (janela dela
/// trazida para frente); `None` = já existe e `focus_existing` é falso.
#[cfg(windows)]
fn try_single_instance(focus_existing: bool) -> Option<bool> {
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_SHOW,
    };
    unsafe {
        let handle = CreateMutexW(None, true, w!("Local\\EloApp_Singleton"));
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        if already {
            // Solta o nosso handle: segurá-lo manteria o mutex vivo para sempre.
            if let Ok(h) = handle {
                let _ = CloseHandle(h);
            }
            if !focus_existing {
                return None;
            }
            // Já há uma instância: traz a janela dela para frente e desiste.
            if let Ok(h) = FindWindowW(None, w!("Elo")) {
                if !h.0.is_null() {
                    let _ = ShowWindow(h, SW_SHOW);
                    let _ = SetForegroundWindow(h);
                }
            }
            return Some(false);
        }
        // Primeira instância: mantém o mutex vivo pelo resto do processo
        // (o handle simplesmente não é fechado).
        let _ = handle;
        Some(true)
    }
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
pub fn open_folder(path: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("não consegui criar a pasta: {e}"))?;
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    r.map(|_| ()).map_err(|e| format!("não consegui abrir o Explorador de Arquivos: {e}"))
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
/// Prefixo do erro interno que pede confirmação quando o node id de um ID curto mudou.
const ERR_IDENTITY_CHANGED: &str = "identity-changed:";

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
    Manual,
}

/// Which saved connection is being renamed in-place (name only).
#[derive(PartialEq, Clone, Copy)]
pub enum RenameTarget {
    Favorite(usize),
    Recent(usize),
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

/// Resultado de uma ação em segundo plano (ver `App::run_bg`).
struct BgDone {
    key: &'static str,
    result: Result<String, String>,
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
    /// Estado do registro do ID curto estável (diretório HTTPS).
    pub short_state: hostrun::SharedShort,
    short_err_seen: u64,
    /// Aviso de identidade: node id novo aguardando confirmação do usuário.
    pub identity_prompt: Option<String>,
    /// Node id que o usuário já aceitou conectar (pula o aviso uma vez).
    identity_accept: Option<String>,
    /// Node id resolvido pelo diretório na tentativa de conexão em curso.
    resolved_node: Arc<Mutex<Option<String>>>,
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
    /// (texto, quando surgiu, é erro?)
    pub toasts: Vec<(String, Instant, bool)>,
    /// Última vez que cada botão "Copiar" foi clicado (chave -> instante).
    pub copied: HashMap<String, Instant>,
    /// Ações em segundo plano em andamento (chave -> início).
    pub busy: HashMap<&'static str, Instant>,
    bg_tx: smpsc::Sender<BgDone>,
    bg_rx: smpsc::Receiver<BgDone>,

    // Navigation.
    pub view: View,
    pub settings_tab: SettingsTab,

    // Controller.
    pub remote_addr: String,
    pub remote_password: String,
    pub mode_lan: bool,
    pub fav_name: String,
    pub conn: Conn,
    /// Task of the pending connection (aborted by "Cancelar").
    connect_abort: Option<tokio::task::AbortHandle>,
    pub need_2fa: bool,
    pub twofa_code: String,

    // Favorites/recents presence (address -> online).
    pub presence: Arc<Mutex<HashMap<String, bool>>>,

    // In-place rename state for favorites/recents.
    pub rename: Option<RenameTarget>,
    pub rename_buf: String,

    // LAN discovery: devices seen on the network, and our own announcement
    // (kept alive while this machine hosts).
    pub discovery: Option<DiscoveryHandle>,
    pub announcer: Option<Announcer>,
    /// Busca na rede em andamento (botão Atualizar) até este instante.
    pub discovery_scan_until: Option<std::time::Instant>,
    /// Por que a descoberta na LAN não está disponível, se não estiver.
    pub discovery_error: Option<String>,

    // Live link to the connected controller (host side), for sending files to
    // it. The host loop fills it with `Some` while a controller is connected.
    pub host_link: Arc<Mutex<Option<HostLink>>>,

    pub enroll: Option<Enroll>,

    // Ajuda do Defender: `None` = ainda verificando, `Some(true/false)` = já sabe
    // se este exe está excluído. Preenchido por uma thread em segundo plano.
    pub defender_excluded: Arc<Mutex<Option<bool>>>,
    pub defender_dismissed: bool,
    pub defender_show_manual: bool,

    // Bandeja do sistema (estilo AnyDesk): o app continua rodando ao fechar a
    // janela. `tray` remove o ícone no Drop; `tray_rx` entrega cliques/menu.
    pub tray: Option<tray::Tray>,
    pub tray_rx: Option<smpsc::Receiver<tray::TrayEvent>>,
    /// Verdadeiro só quando o usuário escolheu "Sair" (fecha de verdade).
    /// Compartilhado com a thread da bandeja, que o liga via Win32 ao sair.
    pub really_quit: Arc<AtomicBool>,
    /// `HWND` da janela principal (0 até ser descoberto em `update`),
    /// compartilhado com a bandeja para controlá-la diretamente pelo Win32.
    pub main_hwnd: Arc<AtomicIsize>,
    /// Já avisamos uma vez que o app ficou na bandeja ao fechar.
    pub tray_notice_shown: bool,
    /// Mantém a janela sempre no topo durante uma sessão de controle.
    pub always_on_top: bool,
    /// Chamado de qualquer thread quando chega um pedido de aprovação: balão na
    /// bandeja + janela à frente (mesmo escondida, quando `update()` não roda).
    attention: Arc<dyn Fn(&str) + Send + Sync>,
    /// Chegou pedido de aprovação neste frame: mostrar/focar o viewport.
    approval_arrived: bool,
    /// Há alguém conectado a este computador (atualizado pelo próprio host).
    host_busy: Arc<AtomicBool>,
    /// Há uma conexão de controle ativa ou em andamento (atualizado no `update`).
    ctl_busy: Arc<AtomicBool>,
    /// Aviso do watchdog (rede mudou com a janela aberta) para virar toast.
    net_notice: Arc<Mutex<Option<String>>>,
    egui_ctx: egui::Context,
}

impl App {
    fn new(egui_ctx: &egui::Context) -> Self {
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

        let http_dir = elo_client_core::shortid::is_http(&config.directory_url());
        let (my_node, my_short_id) = if service_mode {
            let sdir = config::service_dir();
            let id = elo_p2p::id_from_secret_file(&sdir.join("node.key")).unwrap_or_else(|| "—".into());
            let sc = Config::load_from(&sdir);
            let short = if http_dir {
                if sc.short_id_cache.len() == 9 { sc.short_id_cache } else { config.short_id_cache.clone() }
            } else if sc.short_id.len() == 9 {
                sc.short_id
            } else {
                config.short_id.clone()
            };
            (id, short)
        } else {
            ("iniciando…".to_string(), if http_dir { config.short_id_cache.clone() } else { config.short_id.clone() })
        };
        let (ui_tx, ui_rx) = smpsc::channel();
        let (bg_tx, bg_rx) = smpsc::channel();
        let light = config.theme == "light";
        // Ícone na bandeja: se falhar, o app segue sem ele. O contexto do egui
        // é passado para a thread da bandeja acordar o `update()` (via
        // `request_repaint`) quando o usuário clica/escolhe algo com a janela
        // escondida — senão o evento "Sair"/"Abrir" ficaria sem ser lido.
        // Compartilhados com a thread da bandeja ANTES de iniciá-la: o HWND da
        // janela principal (descoberto depois, em `update`) e o flag de "sair".
        let main_hwnd = Arc::new(AtomicIsize::new(0));
        let really_quit = Arc::new(AtomicBool::new(false));
        let (tray, tray_rx) = match tray::start("Elo", egui_ctx.clone(), main_hwnd.clone(), really_quit.clone()) {
            Ok((t, rx)) => (Some(t), Some(rx)),
            Err(e) => {
                tracing::warn!(error = %e, "bandeja do sistema indisponível");
                (None, None)
            }
        };
        let attention: Arc<dyn Fn(&str) + Send + Sync> = {
            let tray_hwnd = tray.as_ref().map_or(0, |t: &tray::Tray| t.hwnd());
            let main_hwnd = main_hwnd.clone();
            let ctx = egui_ctx.clone();
            Arc::new(move |name: &str| {
                tray::alert(
                    tray_hwnd,
                    main_hwnd.load(Ordering::SeqCst),
                    "Pedido de conexão — Elo",
                    &format!("{name} quer acessar este computador"),
                );
                ctx.request_repaint();
            })
        };
        App {
            attention,
            approval_arrived: false,
            host_busy: Arc::new(AtomicBool::new(false)),
            ctl_busy: Arc::new(AtomicBool::new(false)),
            net_notice: Arc::new(Mutex::new(None)),
            egui_ctx: egui_ctx.clone(),
            rt: Runtime::new().expect("tokio runtime"),
            pal: palette(light),
            draft: config.clone(),
            live: Arc::new(Mutex::new(config.clone())),
            config,
            service_mode,
            my_lan: local_ip(),
            my_node: Arc::new(Mutex::new(my_node)),
            my_short_id,
            short_state: Arc::new(Mutex::new(hostrun::ShortState::default())),
            short_err_seen: 0,
            identity_prompt: None,
            identity_accept: None,
            resolved_node: Arc::new(Mutex::new(None)),
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
            copied: HashMap::new(),
            busy: HashMap::new(),
            bg_tx,
            bg_rx,
            view: View::Home,
            settings_tab: SettingsTab::General,
            remote_addr: String::new(),
            remote_password: String::new(),
            mode_lan: false,
            fav_name: String::new(),
            conn: Conn::Idle,
            connect_abort: None,
            need_2fa: false,
            twofa_code: String::new(),
            presence: Arc::new(Mutex::new(HashMap::new())),
            rename: None,
            rename_buf: String::new(),
            discovery: None,
            discovery_scan_until: None,
            discovery_error: None,
            announcer: None,
            host_link: Arc::new(Mutex::new(None)),
            enroll: None,
            defender_excluded: {
                let cell = Arc::new(Mutex::new(None));
                defender::spawn_check(cell.clone());
                cell
            },
            defender_dismissed: false,
            defender_show_manual: false,
            tray,
            tray_rx,
            really_quit,
            main_hwnd,
            tray_notice_shown: false,
            always_on_top: false,
        }
    }

    /// Processa cliques/menu da bandeja: mostrar a janela ou sair de verdade.
    fn drain_tray(&mut self, ctx: &egui::Context) {
        let mut events = Vec::new();
        if let Some(rx) = &self.tray_rx {
            while let Ok(ev) = rx.try_recv() {
                events.push(ev);
            }
        }
        for ev in events {
            match ev {
                tray::TrayEvent::Open => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::TrayEvent::Quit => {
                    self.really_quit.store(true, Ordering::SeqCst);
                    // Mostra a janela antes de fechar: fechar uma janela escondida
                    // pode não encerrar o loop em alguns casos. (Backup do canal; a
                    // garantia é o `WM_CLOSE` direto que a bandeja já postou.)
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    /// Intercepta o fechar da janela: com "fechar para a bandeja" ligado e um
    /// ícone de bandeja disponível, apenas esconde a janela (a conexão/host
    /// continuam vivos) em vez de sair. "Sair" pela bandeja ignora isto.
    fn handle_close(&mut self, ctx: &egui::Context) {
        let closing = ctx.input(|i| i.viewport().close_requested());
        if closing && !self.really_quit.load(Ordering::SeqCst) && self.config.close_to_tray && self.tray.is_some() && cfg!(windows) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            if !self.tray_notice_shown {
                self.tray_notice_shown = true;
                self.toast("O Elo continua rodando na bandeja.");
            }
        }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push((text.into(), Instant::now(), false));
    }

    /// Toast vermelho de erro; também vai para o log.
    pub fn toast_error(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::warn!(erro = %text, "falha mostrada ao usuário");
        self.toasts.push((text, Instant::now(), true));
    }

    /// Toast de sucesso ou de erro conforme o resultado.
    pub fn toast_result(&mut self, r: Result<String, String>) {
        match r {
            Ok(m) if m.is_empty() => {}
            Ok(m) => self.toast(m),
            Err(e) => self.toast_error(e),
        }
    }

    /// A ação em segundo plano `key` está rodando?
    pub fn is_busy(&self, key: &str) -> bool {
        self.busy.contains_key(key)
    }

    /// Roda `work` numa thread (nunca bloqueia o frame). Enquanto roda, `key`
    /// fica em `busy`; o resultado vira toast (sucesso ou erro; texto vazio =
    /// sem toast). Ignora se `key` já está rodando.
    pub fn run_bg(&mut self, ctx: &egui::Context, key: &'static str, work: impl FnOnce() -> Result<String, String> + Send + 'static) {
        if self.busy.contains_key(key) {
            return;
        }
        self.busy.insert(key, Instant::now());
        let tx = self.bg_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = work();
            let _ = tx.send(BgDone { key, result: r });
            ctx.request_repaint();
        });
    }

    fn poll_bg(&mut self) {
        while let Ok(done) = self.bg_rx.try_recv() {
            self.busy.remove(done.key);
            self.toast_result(done.result);
        }
    }

    /// Botão "Copiar": copia `text`, mostra "Copiado ✔" por ~1,5 s e dá um toast.
    pub fn copy_button(&mut self, ui: &mut egui::Ui, key: &str, label: &str, text: &str, toast: &str, small: bool) -> egui::Response {
        const FOR: Duration = Duration::from_millis(1500);
        let since = self.copied.get(key).map(|t| t.elapsed());
        let done = since.is_some_and(|e| e < FOR);
        if let Some(e) = since.filter(|e| *e < FOR) {
            ui.ctx().request_repaint_after(FOR - e + Duration::from_millis(20));
        }
        let shown = if done { "Copiado ✔" } else { label };
        let resp = if small { ui.small_button(shown) } else { ui.button(shown) };
        if resp.clicked() {
            ui.output_mut(|o| o.copied_text = text.to_string());
            self.copied.insert(key.to_string(), Instant::now());
            self.toast(toast);
            ui.ctx().request_repaint_after(FOR + Duration::from_millis(20));
        }
        resp
    }

    /// Persist a new configuration (user dir, service dir in service mode) and
    /// apply it to the running host.
    pub fn save_config(&mut self, cfg: Config) {
        let autostart_changed = cfg.autostart != self.config.autostart;
        let otp_turned_on = cfg.otp && !self.config.otp;
        self.config = cfg;
        if let Err(e) = self.config.try_save_to(&config::config_dir()) {
            self.toast_error(format!("Não consegui gravar as configurações: {e}"));
        }
        if self.service_mode {
            self.config.save_to(&config::service_dir());
            if Config::load_from(&config::service_dir()).allow_control != self.config.allow_control {
                self.toast_error("Não consegui gravar a configuração do serviço (permissão). Reinstale o serviço.");
            }
        }
        *self.live.lock().unwrap() = self.config.clone();
        if autostart_changed {
            if let Err(e) = config::set_autostart(self.config.autostart) {
                self.toast_error(format!("Não consegui alterar o início com o sistema: {e}"));
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

        // LAN discovery: listen for other devices and announce ourselves while
        // we host. Needs the tokio runtime context.
        let disp = self.display_name();
        {
            let _g = self.rt.enter();
            match elo_client_core::discovery::listen() {
                Ok(h) => self.discovery = Some(h),
                Err(e) => {
                    tracing::warn!(error = %e, "descoberta na LAN indisponível");
                    self.discovery_error = Some(e.to_string());
                }
            }
            self.announcer = Some(elo_client_core::discovery::start(disp, self.my_short_id.clone(), DIRECT_PORT));
        }

        if self.service_mode {
            *status.lock().unwrap() = "online — serviço do Windows (funciona com a tela bloqueada)".into();
            ipc::spawn_gui_server(&self.rt, config::service_dir(), self.ui_tx.clone(), self.attention.clone());
            {
                // O serviço também registra (idempotente); a GUI registra para exibir o número.
                let _g = self.rt.enter();
                let my_node = self.my_node.clone();
                hostrun::spawn_directory(
                    self.config.directory_url(),
                    self.config.short_id.clone(),
                    Arc::new(move || my_node.lock().unwrap().clone()),
                    Some(self.short_state.clone()),
                    None,
                );
            }
            self.rt.spawn(async move {
                match elo_p2p::bind_node_ephemeral().await {
                    Ok(n) => *node_slot.lock().unwrap() = Some(n),
                    Err(e) => tracing::warn!(error = %e, "P2P indisponível para conexões de saída"),
                }
            });
        } else {
            {
                let live = self.live.clone();
                power::spawn_keep_awake(Arc::new(move || live.lock().unwrap().keep_awake));
            }
            self.spawn_watchdog();
            let attention = self.attention.clone();
            let host_busy = self.host_busy.clone();
            let my_node = self.my_node.clone();
            let keypair = hostrun::load_or_create_keypair(&config::config_dir());
            let live = self.live.clone();
            let pw = self.session_password.clone();
            let ui_tx = self.ui_tx.clone();
            let link_out = self.host_link.clone();
            let online = format!("online — LAN {}:{DIRECT_PORT} · Internet pronto", self.my_lan);
            let directory = self.config.directory_url();
            let short = self.config.short_id.clone();
            let short_state = self.short_state.clone();
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
                let nid = node.id();
                hostrun::spawn_directory(directory, short, Arc::new(move || nid.clone()), Some(short_state.clone()), None);

                let live2 = live.clone();
                let pw2 = pw.clone();
                let settings: Arc<dyn Fn() -> HostSettings + Send + Sync> =
                    Arc::new(move || HostSettings::from_config(&live2.lock().unwrap(), &pw2.lock().unwrap()));
                let tx = ui_tx.clone();
                let approver: ApproveFn = Arc::new(move |peer| {
                    let tx = tx.clone();
                    let attention = attention.clone();
                    Box::pin(async move {
                        // Tela bloqueada: ninguém vai ver o pedido; recusa já,
                        // com o motivo para o controlador.
                        if power::session_locked() {
                            tracing::info!(peer = %peer.name, "pedido de aprovação recusado: sessão bloqueada");
                            elo_client_core::host::set_deny_reason(power::NOBODY_TO_APPROVE);
                            return false;
                        }
                        let (otx, orx) = tokio::sync::oneshot::channel();
                        let name = peer.name.clone();
                        if tx.send(UiEvent::Approval { name: peer.name, fp: peer.fingerprint, reply: otx }).is_err() {
                            return false;
                        }
                        attention(&name);
                        power::unless_locked(async move { orx.await.unwrap_or(false) }).await
                    })
                });
                let tx = std::sync::Mutex::new(ui_tx.clone());
                let on_event: EventFn = Arc::new(move |ev| {
                    match &ev {
                        elo_client_core::host::HostEvent::Connected { .. } => host_busy.store(true, Ordering::SeqCst),
                        elo_client_core::host::HostEvent::Disconnected { .. } => host_busy.store(false, Ordering::SeqCst),
                        _ => {}
                    }
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
                    link_out: Some(link_out),
                    short: Some(short_state),
                })
                .await;
            });
        }
        self.spawn_presence();
    }

    /// Watchdog da GUI hospedando sem serviço: após suspensão/troca de rede,
    /// recria o host reiniciando o Elo (`--restarted --tray`) quando ele está
    /// escondido na bandeja e ocioso; com a janela aberta, só avisa (uma vez).
    fn spawn_watchdog(&self) {
        let host_busy = self.host_busy.clone();
        let ctl_busy = self.ctl_busy.clone();
        let main_hwnd = self.main_hwnd.clone();
        let tray_hwnd = self.tray.as_ref().map_or(0, |t| t.hwnd());
        let notice = self.net_notice.clone();
        let ctx = self.egui_ctx.clone();
        let warned = AtomicBool::new(false);
        watchdog::spawn(
            Arc::new(move || host_busy.load(Ordering::SeqCst)),
            Box::new(move |reason| {
                let hidden = !tray::main_window_visible(main_hwnd.load(Ordering::SeqCst));
                if hidden && tray_hwnd != 0 && !ctl_busy.load(Ordering::SeqCst) {
                    let spawned = std::env::current_exe()
                        .and_then(|exe| std::process::Command::new(exe).args(["--restarted", "--tray"]).spawn());
                    match spawned {
                        Ok(_) if tray::post_quit(tray_hwnd) => {
                            tracing::warn!(motivo = %reason, "watchdog: reiniciando o Elo para recriar o host");
                            return true;
                        }
                        Ok(_) => tracing::warn!("watchdog: não consegui encerrar esta instância"),
                        Err(e) => tracing::warn!(error = %e, "watchdog: não consegui reiniciar o Elo"),
                    }
                }
                if !warned.swap(true, Ordering::SeqCst) {
                    tracing::warn!(motivo = %reason, "watchdog: rede mudou; aguardando o Elo ficar ocioso na bandeja para reiniciar");
                    *notice.lock().unwrap() = Some(reason.to_string());
                    ctx.request_repaint();
                }
                false
            }),
        );
    }

    fn display_name(&self) -> String {
        if self.config.display_name.trim().is_empty() { config::computer_name() } else { self.config.display_name.clone() }
    }

    /// Traz para a UI o número vindo do diretório e avisa (uma vez) sobre erros.
    fn sync_short_id(&mut self) {
        let st = self.short_state.lock().unwrap().clone();
        if let Some(id) = st.id {
            if id != self.my_short_id {
                self.my_short_id = id.clone();
                let _g = self.rt.enter();
                self.announcer = None;
                self.announcer = Some(elo_client_core::discovery::start(self.display_name(), id.clone(), DIRECT_PORT));
            }
            if self.config.short_id_cache != id {
                let mut cfg = self.config.clone();
                cfg.short_id_cache = id;
                self.draft.short_id_cache = cfg.short_id_cache.clone();
                self.save_config(cfg);
            }
        }
        if st.err_seq != self.short_err_seen {
            self.short_err_seen = st.err_seq;
            if let Some(e) = st.error {
                self.toast_error(e);
            }
        }
    }

    /// Check every ~45 s which favorites are online.
    fn spawn_presence(&self) {
        let live = self.live.clone();
        let presence = self.presence.clone();
        let node = self.node.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            loop {
                // Favorites + recents, deduplicated by address.
                let (targets, dir) = {
                    let c = live.lock().unwrap();
                    let mut t: Vec<(String, bool)> = c.favorites.iter().map(|f| (f.address.clone(), f.lan)).collect();
                    for r in &c.recents {
                        if !t.iter().any(|(a, _)| a == &r.address) {
                            t.push((r.address.clone(), r.lan));
                        }
                    }
                    (t, c.directory_url())
                };
                let n = node.lock().unwrap().clone();
                for (address, lan) in targets {
                    let online = if lan {
                        let addr = if address.contains(':') { address.clone() } else { format!("{}:{DIRECT_PORT}", address) };
                        matches!(tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(addr)).await, Ok(Ok(_)))
                    } else {
                        let digits: String = address.chars().filter(|c| c.is_ascii_digit()).collect();
                        if digits.len() == 9 && !dir.trim().is_empty() {
                            if elo_client_core::shortid::is_http(&dir) {
                                match elo_client_core::shortid::resolve(&dir, &digits).await {
                                    Ok(node_id) => match &n {
                                        Some(n) => n.ping(&node_id, Duration::from_secs(8)).await,
                                        None => true,
                                    },
                                    Err(_) => false,
                                }
                            } else {
                                elo_client_core::rendezvous::resolve_directory(&dir, &digits).await.is_ok()
                            }
                        } else if let Some(n) = &n {
                            n.ping(&address, Duration::from_secs(8)).await
                        } else {
                            false
                        }
                    };
                    presence.lock().unwrap().insert(address.clone(), online);
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
                self.toast_error(format!("Não foi possível iniciar a conexão: {e}"));
                self.conn = Conn::Failed(e.to_string());
                return;
            }
        };
        let password = self.remote_password.clone();
        let name = if self.config.display_name.trim().is_empty() { config::current_username() } else { self.config.display_name.clone() };
        let opts = ConnectOptions { name: Some(name), totp_code: code, ..Default::default() };
        let target = self.remote_addr.trim().to_string();
        let (tx, rx) = smpsc::channel::<ConnectResult>();
        tracing::info!(%target, lan = self.mode_lan, "conectando");

        if self.mode_lan {
            use elo_client_core::controller::{run_controller_direct_opts, DirectControllerConfig};
            let host = if target.contains(':') { target.clone() } else { format!("{target}:{DIRECT_PORT}") };
            let cfg = DirectControllerConfig { host, keypair, password, download_dir: download_dir() };
            let task = self.rt.spawn(async move {
                let r = run_controller_direct_opts(cfg, opts).await.map(|(h, e)| (h, e, target)).map_err(|e| format!("{e:#}"));
                let _ = tx.send(r);
            });
            self.connect_abort = Some(task.abort_handle());
        } else {
            let node = self.node.lock().unwrap().clone();
            let Some(node) = node else {
                self.toast_error("A conexão P2P ainda está iniciando — tente de novo em instantes.");
                self.conn = Conn::Failed("P2P ainda iniciando — tente em instantes.".into());
                return;
            };
            let directory = self.config.directory_url();
            *self.resolved_node.lock().unwrap() = None;
            let resolved_slot = self.resolved_node.clone();
            // Último node id visto para este ID curto (favoritos/recentes) e
            // eventual confirmação do usuário para a identidade nova.
            let known_node = {
                let t = target.trim();
                self.config
                    .favorites
                    .iter()
                    .map(|f| (&f.address, &f.last_node))
                    .chain(self.config.recents.iter().map(|r| (&r.address, &r.last_node)))
                    .find(|(a, n)| a.trim() == t && !n.is_empty())
                    .map(|(_, n)| n.clone())
                    .unwrap_or_default()
            };
            let accepted = self.identity_accept.clone();
            let task = self.rt.spawn(async move {
                let res = async {
                    let digits: String = target.chars().filter(|c| c.is_ascii_digit()).collect();
                    let node_id = if digits.len() == 9 && target.len() <= 11 {
                        let r = if elo_client_core::shortid::is_http(&directory) {
                            elo_client_core::shortid::resolve(&directory, &digits).await
                        } else {
                            elo_client_core::rendezvous::resolve_directory(&directory, &digits).await
                        };
                        let nid = r.map_err(|e| hostrun::friendly_directory_error(&format!("{e:#}")))?;
                        *resolved_slot.lock().unwrap() = Some(nid.clone());
                        if !known_node.is_empty() && known_node != nid && accepted.as_deref() != Some(nid.as_str()) {
                            return Err(format!("{ERR_IDENTITY_CHANGED}{nid}"));
                        }
                        nid
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
            self.connect_abort = Some(task.abort_handle());
        }
        self.conn = Conn::Connecting(rx);
    }

    /// Abandon the pending connection (including a wait for approval).
    pub fn cancel_connect(&mut self) {
        if let Some(task) = self.connect_abort.take() {
            task.abort();
        }
        if matches!(self.conn, Conn::Connecting(_)) {
            tracing::info!("conexão cancelada pelo usuário");
            self.conn = Conn::Idle;
        }
    }

    pub fn start_connect_fav(&mut self, fav: &config::Favorite) {
        self.remote_addr = fav.address.clone();
        self.remote_password = fav.password.clone();
        self.mode_lan = fav.lan;
        self.start_connect(None);
    }

    /// Upsert a recent on a successful controller connection: dedup by address,
    /// newest first, capped at 10. Keeps an existing (possibly renamed) name,
    /// else the matching favorite's name, else the address itself.
    fn record_recent(&mut self, address: &str, lan: bool, password: String, node: Option<String>) {
        let addr = address.trim().to_string();
        if addr.is_empty() {
            return;
        }
        let mut cfg = self.config.clone();
        let name = cfg
            .recents
            .iter()
            .find(|r| r.address == addr)
            .map(|r| r.name.clone())
            .or_else(|| cfg.favorites.iter().find(|f| f.address == addr).map(|f| f.name.clone()))
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| addr.clone());
        let prev_node = cfg
            .recents
            .iter()
            .find(|r| r.address == addr)
            .map(|r| r.last_node.clone())
            .or_else(|| cfg.favorites.iter().find(|f| f.address == addr).map(|f| f.last_node.clone()))
            .unwrap_or_default();
        let last_node = node.clone().unwrap_or(prev_node);
        if let Some(n) = &node {
            for f in cfg.favorites.iter_mut().filter(|f| f.address == addr) {
                f.last_node = n.clone();
            }
            self.draft.favorites = cfg.favorites.clone();
        }
        cfg.recents.retain(|r| r.address != addr);
        cfg.recents.insert(0, config::Recent { name, address: addr, lan, password, last_node });
        cfg.recents.truncate(10);
        self.draft.recents = cfg.recents.clone();
        self.save_config(cfg);
    }

    /// Adiciona um favorito evitando duplicar por `address` + `lan`. Retorna
    /// `true` se adicionou, `false` se já existia um favorito equivalente.
    pub fn add_favorite_unique(&mut self, name: String, address: String, lan: bool, password: String) -> bool {
        let addr = address.trim().to_string();
        if addr.is_empty() || self.config.favorites.iter().any(|f| f.address == addr && f.lan == lan) {
            return false;
        }
        let mut cfg = self.config.clone();
        cfg.favorites.push(config::Favorite { name, address: addr, lan, password, last_node: String::new() });
        self.draft.favorites = cfg.favorites.clone();
        self.save_config(cfg);
        true
    }

    pub fn save_favorite(&mut self) {
        let addr = self.remote_addr.trim().to_string();
        if addr.is_empty() {
            self.toast_error("Informe o endereço antes de salvar.");
            return;
        }
        let name = if self.fav_name.trim().is_empty() { addr.chars().take(18).collect() } else { self.fav_name.trim().to_string() };
        let mut cfg = self.config.clone();
        cfg.favorites.retain(|f| f.address != addr);
        cfg.favorites.push(config::Favorite { name, address: addr, lan: self.mode_lan, password: self.remote_password.clone(), last_node: String::new() });
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
                    self.approval_arrived = true;
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
        self.connect_abort = None;
        self.conn = match result {
            Ok((handle, events, label)) => {
                tracing::info!(target = %label, "sessão de controle iniciada");
                self.toast(format!("Conectado a {label}."));
                self.identity_accept = None;
                let node = self.resolved_node.lock().unwrap().take();
                self.record_recent(&label, self.mode_lan, self.remote_password.clone(), node);
                Conn::Active(Box::new(Session::new(handle, events, label)))
            }
            Err(e) if e.contains(ERR_2FA_REQUIRED) => {
                self.need_2fa = true;
                self.twofa_code.clear();
                Conn::Idle
            }
            Err(e) if e.starts_with(ERR_IDENTITY_CHANGED) => {
                self.identity_prompt = Some(e[ERR_IDENTITY_CHANGED.len()..].to_string());
                Conn::Idle
            }
            Err(e) => {
                tracing::warn!(error = %e, "falha ao conectar");
                let msg = friendly_error(&e);
                self.toast_error(format!("Não foi possível conectar: {msg}"));
                Conn::Failed(msg)
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
                    self.toast(format!("{} agora é um dispositivo confiável.", req.name));
                }
            }
            tracing::info!(peer = %req.name, allow, trust = req.trust, "pedido de conexão respondido");
            if allow {
                self.toast(format!("Acesso permitido a {}.", req.name));
            } else {
                self.toast(format!("Acesso negado a {}.", req.name));
            }
            if let Some(r) = req.reply.take() {
                let _ = r.send(allow);
            }
        }
    }

    fn identity_modal(&mut self, ctx: &egui::Context) {
        let Some(new_node) = self.identity_prompt.clone() else { return };
        let pal = self.pal;
        let mut decision: Option<bool> = None;
        modal(ctx, &pal, "identity", |ui| {
            ui.label(egui::RichText::new("Identidade alterada").size(18.0).strong().color(pal.text));
            ui.add_space(6.0);
            ui.label(egui::RichText::new("A identidade deste aparelho mudou (reinstalação?). Conectar mesmo assim?").color(pal.muted));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(format!("Novo endereço: {}", short_fp(&new_node))).monospace().small().color(pal.muted));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Cancelar").min_size(egui::vec2(100.0, 34.0))).clicked() {
                    decision = Some(false);
                }
                if theme::primary_button(ui, &pal, "  Conectar  ").clicked() {
                    decision = Some(true);
                }
            });
        });
        match decision {
            Some(true) => {
                self.identity_prompt = None;
                self.identity_accept = Some(new_node);
                let code = self.need_2fa.then(|| self.twofa_code.clone());
                self.start_connect(code);
            }
            Some(false) => self.identity_prompt = None,
            None => {}
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
                let code = self.twofa_code.trim().to_string();
                if code.is_empty() {
                    self.toast_error("Digite o código de 6 dígitos do app autenticador.");
                } else {
                    self.need_2fa = false;
                    self.start_connect(Some(code));
                }
            }
            Some(false) => {
                self.need_2fa = false;
                self.toast("Conexão cancelada.");
            }
            None => {}
        }
    }

    /// Aviso opcional do Windows Defender: só aparece quando a verificação já
    /// terminou e este exe NÃO está excluído. Explica a situação e oferece o
    /// atalho (UAC) e o caminho manual. Em outros sistemas nunca aparece.
    fn defender_modal(&mut self, ctx: &egui::Context) {
        if self.defender_dismissed || self.config.defender_hidden {
            return;
        }
        let excluded = *self.defender_excluded.lock().unwrap();
        if excluded != Some(false) {
            return; // ainda verificando, já excluído, ou não foi possível checar
        }
        let pal = self.pal;
        let mut add = false;
        let mut dismiss = false;
        let mut copy_cmd = false;
        modal(ctx, &pal, "defender", |ui| {
            ui.set_max_width(460.0);
            ui.label(egui::RichText::new("Windows Defender pode bloquear o Elo").size(18.0).strong().color(pal.text));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(
                "O Elo ainda não tem assinatura de código. Por isso o Windows Defender \
                 pode marcá-lo como falso positivo e até removê-lo do computador. \
                 Adicionar uma exceção para o Elo evita isso até a assinatura ficar pronta.",
            ).color(pal.muted));
            ui.add_space(8.0);
            ui.label(egui::RichText::new(
                "Adicionar exceção é uma função oficial do Windows e vai pedir confirmação \
                 de administrador (UAC). A escolha é sua.",
            ).size(12.0).color(pal.muted));

            if self.defender_show_manual {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Para fazer à mão, rode num PowerShell como administrador:").size(12.0).color(pal.muted));
                ui.add_space(4.0);
                let mut cmd = defender::manual_command();
                egui::Frame::none().fill(pal.surface).stroke(egui::Stroke::new(1.0_f32, pal.border)).rounding(6.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut cmd).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(2));
                });
                let copied_now = self.copied.get("defender-cmd").is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
                if copied_now {
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                }
                copy_cmd = ui.small_button(if copied_now { "Copiado ✔" } else { "Copiar comando" }).on_hover_text("Copia o comando para colar no PowerShell de administrador").clicked();
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if theme::primary_button(ui, &pal, "  Adicionar exceção  ").clicked() {
                    add = true;
                }
                if ui.add(egui::Button::new(if self.defender_show_manual { "Ocultar passos" } else { "Ver passos manuais" }).min_size(egui::vec2(130.0, 34.0))).clicked() {
                    self.defender_show_manual = !self.defender_show_manual;
                }
                if ui.add(egui::Button::new("Agora não").min_size(egui::vec2(90.0, 34.0))).clicked() {
                    dismiss = true;
                }
            });
        });

        if copy_cmd {
            ctx.output_mut(|o| o.copied_text = defender::manual_command());
            self.copied.insert("defender-cmd".into(), Instant::now());
            self.toast("Comando copiado.");
        }
        if add {
            match defender::request_exclusion() {
                Ok(()) => {
                    self.toast("Se você confirmou no UAC, a exceção foi adicionada.");
                    // Re-verifica depois de alguns segundos; o painel some se der certo.
                    *self.defender_excluded.lock().unwrap() = None;
                    let cell = self.defender_excluded.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(3));
                        defender::spawn_check(cell);
                    });
                    self.defender_dismissed = true;
                    // Não mostrar de novo em execuções futuras.
                    self.config.defender_hidden = true;
                    self.config.save();
                }
                Err(e) => self.toast_error(format!("Não foi possível adicionar a exceção: {e}")),
            }
        }
        if dismiss {
            self.defender_dismissed = true;
            // "Agora não" / ignorar: não aparece mais nas próximas execuções.
            self.config.defender_hidden = true;
            self.config.save();
            self.toast("Aviso dispensado. Ele não aparecerá de novo.");
        }
    }

    /// Floating notifications, bottom-right.
    fn toasts_ui(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|(_, t, err)| t.elapsed() < Duration::from_secs(if *err { 9 } else { 5 }));
        if self.toasts.is_empty() {
            return;
        }
        let pal = self.pal;
        egui::Area::new("toasts".into())
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -18.0])
            .show(ctx, |ui| {
                for (text, _, err) in &self.toasts {
                    egui::Frame::none()
                        .fill(if *err { { let (a, b, t) = (pal.surface, pal.danger, 0.15_f32); let m = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t) as u8; Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b())) } } else { pal.surface })
                        .stroke(egui::Stroke::new(1.0_f32, if *err { pal.danger } else { pal.border }))
                        .rounding(10.0)
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .shadow(egui::epaint::Shadow { offset: egui::vec2(0.0, 4.0), blur: 16.0, spread: 0.0, color: Color32::from_black_alpha(90) })
                        .show(ui, |ui| {
                            ui.set_max_width(360.0);
                            ui.label(egui::RichText::new(text).color(if *err { pal.danger } else { pal.text }));
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
        // Descobre o HWND da janela principal na primeira vez (aqui estamos na
        // thread que a criou), e o compartilha com a bandeja via o Arc.
        if self.main_hwnd.load(Ordering::SeqCst) == 0 {
            let h = find_main_window();
            if h != 0 {
                self.main_hwnd.store(h, Ordering::SeqCst);
            }
        }
        self.pump_events();
        // Pedidos já respondidos por outro caminho (tempo esgotado, tela
        // bloqueada, worker caiu) saem da fila.
        self.approvals.retain(|a| a.reply.as_ref().is_some_and(|r| !r.is_closed()));
        if std::mem::take(&mut self.approval_arrived) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if self.net_notice.lock().unwrap().take().is_some() {
            self.toast_error("Rede mudou — reinicie o Elo (ele reinicia sozinho quando estiver na bandeja e ocioso).");
        }
        self.poll_connect();
        self.ctl_busy.store(matches!(self.conn, Conn::Active(_) | Conn::Connecting(_)), Ordering::SeqCst);
        self.poll_bg();
        self.sync_short_id();
        self.drain_tray(ctx);
        self.handle_close(ctx);

        if matches!(self.conn, Conn::Active(_)) {
            ui_session::drive_session(self, ctx);
        } else {
            ui_home::draw_shell(self, ctx);
        }
        self.host_chat_window(ctx);
        self.pointer_overlay(ctx);
        self.approval_modal(ctx);
        self.identity_modal(ctx);
        self.twofa_modal(ctx);
        self.defender_modal(ctx);
        self.toasts_ui(ctx);

        if matches!(self.conn, Conn::Active(_) | Conn::Connecting(_)) || self.pointer.is_some() || !self.busy.is_empty() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
