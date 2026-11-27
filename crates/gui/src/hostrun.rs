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

/// Estado do registro do ID curto (diretório HTTPS), lido pela UI.
#[derive(Default, Clone)]
pub struct ShortState {
    /// Último número de 9 dígitos devolvido pelo servidor.
    pub id: Option<String>,
    pub registering: bool,
    /// Último erro do registro (amigável), se o último ciclo falhou.
    pub error: Option<String>,
    /// Sobe a cada erro novo (diferente do anterior), para a UI dar um toast só uma vez.
    pub err_seq: u64,
}

pub type SharedShort = Arc<Mutex<ShortState>>;

/// Re-registro periódico do ID curto no diretório HTTPS.
const REREGISTER: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// Mensagem amigável (pt-BR) para uma falha do diretório de ID curto.
pub fn friendly_directory_error(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("não encontrado") || l.contains("not found") {
        "ID não encontrado: confira o número ou veja se o outro computador já abriu o Elo.".into()
    } else if l.contains("sem resposta") || l.contains("timed out") || l.contains("dns") || l.contains("connect") {
        "Servidor de ID curto fora do ar ou sem internet. Tente de novo em instantes.".into()
    } else if l.contains("9 dígitos") {
        "O ID curto tem 9 dígitos.".into()
    } else {
        format!("Servidor de ID curto: {e}")
    }
}

/// Mantém o ID curto registrado. Diretório HTTPS: registra com o hash do
/// MachineGuid (número estável, mesmo após reinstalar), repete a cada ~6 h e
/// quando `node_id()` muda; `on_id` recebe o número. Relay TCP (`host:porta`):
/// comportamento antigo com `short_id` aleatório.
pub fn spawn_directory(
    directory: String,
    short_id: String,
    node_id: Arc<dyn Fn() -> String + Send + Sync>,
    state: Option<SharedShort>,
    on_id: Option<Arc<dyn Fn(&str) + Send + Sync>>,
) {
    use elo_client_core::shortid;
    if shortid::is_http(&directory) {
        let set_err = move |state: &Option<SharedShort>, msg: String| {
            if let Some(st) = state {
                let mut g = st.lock().unwrap();
                g.registering = false;
                if g.error.as_deref() != Some(msg.as_str()) {
                    g.err_seq += 1;
                }
                g.error = Some(msg);
            }
        };
        #[cfg(windows)]
        let guid = shortid::windows_machine_guid();
        #[cfg(not(windows))]
        let guid: Option<String> = None;
        let Some(guid) = guid else {
            tracing::warn!("MachineGuid indisponível; ID curto estável desativado");
            set_err(&state, "Não consegui ler o identificador do Windows para o ID curto estável.".into());
            return;
        };
        let hash = shortid::device_hash(&guid);
        tokio::spawn(async move {
            let mut last_node = String::new();
            let mut last_ok: Option<std::time::Instant> = None;
            let mut retry_at = std::time::Instant::now();
            loop {
                let node = node_id();
                let valid = node.len() > 16 && node.chars().all(|c| c.is_ascii_alphanumeric());
                let due = last_ok.map_or(true, |t| t.elapsed() >= REREGISTER)
                    || node != last_node;
                if valid && due && std::time::Instant::now() >= retry_at {
                    if let Some(st) = &state {
                        st.lock().unwrap().registering = true;
                    }
                    match shortid::register(&directory, &hash, &node).await {
                        Ok(short) => {
                            tracing::info!(%directory, "ID curto {short} registrado");
                            if let Some(st) = &state {
                                let mut g = st.lock().unwrap();
                                g.registering = false;
                                g.error = None;
                                g.id = Some(short.clone());
                            }
                            if let Some(f) = &on_id {
                                f(&short);
                            }
                            last_ok = Some(std::time::Instant::now());
                            last_node = node;
                        }
                        Err(e) => {
                            tracing::warn!(%directory, error = %format!("{e:#}"), "registro do ID curto falhou");
                            set_err(&state, friendly_directory_error(&format!("{e:#}")));
                            retry_at = std::time::Instant::now() + std::time::Duration::from_secs(60);
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            }
        });
        return;
    }
    if directory.trim().is_empty() || short_id.len() != 9 {
        return;
    }
    tokio::spawn(async move {
        loop {
            let node_id = node_id();
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
    /// When set, the host publishes a live link to the connected controller here
    /// (`Some` while connected, `None` otherwise) so the UI can send it files.
    pub link_out: Option<Arc<Mutex<Option<elo_client_core::HostLink>>>>,
    /// Estado do registro do ID curto, para a linha de saúde do log.
    pub short: Option<SharedShort>,
}

/// Intervalo da linha de saúde no log.
const HEALTH_EVERY: std::time::Duration = std::time::Duration::from_secs(600);

/// Sessão em andamento: id da conexão, controle e início.
type Current = Arc<Mutex<Option<(u64, elo_client_core::host::HostSessionHandle, std::time::Instant)>>>;

/// Uma linha de log com o estado do host (a cada 10 min).
fn spawn_health_log(node: elo_p2p::Node, current: Current, short: Option<SharedShort>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(HEALTH_EVERY);
        loop {
            tick.tick().await;
            let session = match current.lock().unwrap().as_ref() {
                Some((_, _, since)) => format!("sim, há {} s", since.elapsed().as_secs()),
                None => "não".into(),
            };
            let last = elo_client_core::host::last_controller_activity()
                .map_or("nunca".into(), |t| format!("há {} s", t.elapsed().as_secs()));
            let short = match &short {
                Some(st) => {
                    let g = st.lock().unwrap();
                    match (&g.error, &g.id) {
                        (Some(e), _) => format!("erro: {e}"),
                        (None, Some(id)) => format!("ok ({id})"),
                        (None, None) if g.registering => "registrando".into(),
                        (None, None) => "ainda não registrado".into(),
                    }
                }
                None => "n/d".into(),
            };
            tracing::info!(
                sessao = %session,
                ultima_atividade = %last,
                relay = %node.relay_status(),
                id_curto = %short,
                "saúde do host"
            );
        }
    });
}

/// Run the host forever.
///
/// As conexões são atendidas em paralelo: uma sessão antiga meio-aberta não
/// bloqueia as novas. Quando uma conexão nova autentica com uma sessão em
/// andamento, a antiga é encerrada (normalmente é o mesmo usuário reconectando).
pub async fn run(h: HostRun) {
    #[cfg(windows)]
    {
        use std::collections::HashSet;
        use std::sync::atomic::{AtomicU64, Ordering};

        use elo_client_core::host::{serve_host_opts, HostEvent, HostOptions};
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

        let current: Current = Arc::new(Mutex::new(None));
        // Sessões derrubadas por uma conexão nova: o "desconectou" delas não
        // vai para a UI (a nova já tomou o lugar).
        let superseded: Arc<Mutex<HashSet<u64>>> = Arc::new(Mutex::new(HashSet::new()));
        let next_id = AtomicU64::new(1);
        spawn_health_log(h.node.clone(), current.clone(), h.short.clone());

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
            let id = next_id.fetch_add(1, Ordering::Relaxed);
            let busy = current.lock().unwrap().is_some();
            match &incoming {
                In::Lan(st) => {
                    elo_client_core::set_tcp_keepalive(st);
                    let from = st.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "?".into());
                    tracing::info!(conn = id, origem = "LAN", %from, sessao_ativa = busy, "conexão recebida");
                }
                In::P2p(_) => tracing::info!(conn = id, origem = "P2P", sessao_ativa = busy, "conexão recebida"),
            }

            let s = (h.settings)();
            let fwd = h.on_event.clone();
            let (cur, sup) = (current.clone(), superseded.clone());
            let on_event: elo_client_core::host::EventFn = Arc::new(move |ev| {
                match &ev {
                    HostEvent::Connected { session, .. } => {
                        *cur.lock().unwrap() = Some((id, session.clone(), std::time::Instant::now()));
                    }
                    HostEvent::Disconnected { .. } => {
                        if sup.lock().unwrap().remove(&id) {
                            tracing::info!(conn = id, "sessão anterior encerrada (substituída)");
                            return;
                        }
                        let mut g = cur.lock().unwrap();
                        if g.as_ref().is_some_and(|c| c.0 == id) {
                            *g = None;
                        }
                    }
                    _ => {}
                }
                fwd(ev)
            });
            let opts = HostOptions {
                password: s.password,
                open_access: s.open_access,
                trusted: s.trusted,
                totp_secret: s.totp_secret,
                approver: s.allow_requests.then(|| h.approver.clone()),
                on_event: Some(on_event),
                permissions: s.permissions,
                fps: 30,
                download_dir: h.download_dir.clone(),
                prefer_h264: true,
                show_pointer: s.show_pointer,
                link_out: h.link_out.clone(),
            };
            let (cur, sup) = (current.clone(), superseded.clone());
            // Chamado só depois que o novo controlador autenticou.
            let factory = move || -> anyhow::Result<(DxgiCapture, SendInputSink)> {
                let old = cur.lock().unwrap().take();
                let had_old = old.is_some();
                if let Some((old_id, handle, _)) = old {
                    tracing::info!(conn = id, anterior = old_id, "nova conexão autenticada; encerrando a sessão anterior");
                    sup.lock().unwrap().insert(old_id);
                    handle.disconnect();
                }
                // A captura da sessão anterior pode levar um instante para ser
                // liberada: tenta por até ~3 s.
                let mut tries = 0;
                let c = loop {
                    match DxgiCapture::new() {
                        Ok(c) => break c,
                        Err(e) if tries < 15 && had_old => {
                            tracing::debug!(error = %e, "captura ainda ocupada; tentando de novo");
                            tries += 1;
                            std::thread::sleep(std::time::Duration::from_millis(200));
                        }
                        Err(e) => return Err(e),
                    }
                };
                let i = SendInputSink::new(c.width(), c.height());
                Ok((c, i))
            };
            let keypair = h.keypair.clone();
            let after_session = h.after_session.clone();
            let status = h.status.clone();
            let online_text = h.online_text.clone();
            tokio::spawn(async move {
                let r = match incoming {
                    In::Lan(st) => serve_host_opts(st, &keypair, opts, factory).await,
                    In::P2p(d) => serve_host_opts(d, &keypair, opts, factory).await,
                };
                match r {
                    Ok(()) => after_session(),
                    Err(e) => tracing::info!(conn = id, error = %e, "conexão encerrada"),
                }
                *status.lock().unwrap() = online_text;
            });
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (&h.keypair, &h.download_dir, &h.settings, &h.approver, &h.on_event, &h.after_session, &h.online_text);
        spawn_health_log(h.node.clone(), Arc::new(Mutex::new(None)), h.short.clone());
        *h.status.lock().unwrap() = "pronto para conectar (servir a própria tela requer Windows)".into();
        std::future::pending::<()>().await
    }
}
