//! Headless host: what the Windows service runs inside the user's console
//! session (`elo.exe --session-worker`, launched by the service as SYSTEM so it
//! can follow the secure desktop / lock screen). Also the service's in-process
//! fallback.
//!
//! State (identity, password, 2FA, paired devices, permissions) lives in
//! `state_dir` (`C:\ProgramData\elo`) and is re-read for every connection, so
//! changes made in the GUI apply without restarting the service. Approval
//! prompts, banners and the operator pointer go to the user's GUI over IPC.

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use elo_client_core::host::{set_deny_reason, ApproveFn, EventFn, HostEvent};

use crate::config::Config;
use crate::hostrun::{self, HostRun, HostSettings, DIRECT_PORT};
use crate::ipc::{self, Msg, WorkerLink};

/// Host loop forever using the state in `state_dir`. `watchdog`: encerra o
/// processo (código 3) após suspensão/troca de rede, para o supervisor do
/// serviço relançar o worker — só faz sentido no `--session-worker`.
pub async fn run_headless(state_dir: PathBuf, watchdog: bool) {
    crate::logging::init(&state_dir);
    let _ = std::fs::create_dir_all(&state_dir);
    {
        let dir = state_dir.clone();
        crate::power::spawn_keep_awake(Arc::new(move || Config::load_from(&dir).keep_awake));
    }
    let keypair = hostrun::load_or_create_keypair(&state_dir);
    let node = loop {
        match elo_p2p::bind_node(&state_dir.join("node.key")).await {
            Ok(n) => break n,
            Err(e) => {
                tracing::warn!(error = %e, "P2P indisponível; tentando de novo");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    };
    tracing::info!(node = %node.id(), "host do serviço ativo");

    let cfg0 = Config::load_from(&state_dir);
    let short_state: hostrun::SharedShort = Arc::new(Mutex::new(hostrun::ShortState::default()));
    {
        let nid = node.id();
        let dir = state_dir.clone();
        let on_id: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |short| {
            let mut c = Config::load_from(&dir);
            if c.short_id_cache != short {
                c.short_id_cache = short.to_string();
                c.save_to(&dir);
            }
        });
        hostrun::spawn_directory(cfg0.directory_url(), cfg0.short_id.clone(), Arc::new(move || nid.clone()), Some(short_state.clone()), Some(on_id));
    }

    let link = WorkerLink::start(ipc::token(&state_dir));

    if watchdog {
        let l = link.clone();
        crate::watchdog::spawn(
            Arc::new(move || l.session.lock().unwrap().is_some()),
            Box::new(|reason| {
                tracing::warn!(motivo = %reason, "watchdog: encerrando o worker para o serviço relançá-lo");
                std::process::exit(3)
            }),
        );
    }

    let dir = state_dir.clone();
    let settings: Arc<dyn Fn() -> HostSettings + Send + Sync> =
        Arc::new(move || {
            // No screen to show a session password here: OTP doesn't apply, the
            // fixed password does.
            let mut c = Config::load_from(&dir);
            c.otp = false;
            HostSettings::from_config(&c, "")
        });

    let l = link.clone();
    let approver: ApproveFn = Arc::new(move |peer| {
        let l = l.clone();
        Box::pin(async move {
            // Ninguém para responder (tela bloqueada ou nenhuma interface
            // aberta): recusa na hora, com o motivo para o controlador, em vez
            // de deixá-lo "conectando" até o tempo esgotar.
            let gui = l.has_gui();
            if !gui || crate::power::session_locked() {
                tracing::info!(peer = %peer.name, gui, "pedido de aprovação recusado: ninguém para aprovar");
                set_deny_reason(crate::power::NOBODY_TO_APPROVE);
                return false;
            }
            let allowed = crate::power::unless_locked(l.approve(peer.name, peer.fingerprint)).await;
            if !allowed && !l.has_gui() {
                // A interface fechou no meio do pedido.
                set_deny_reason(crate::power::NOBODY_TO_APPROVE);
            }
            allowed
        })
    });

    let l = link.clone();
    let on_event: EventFn = Arc::new(move |ev| match ev {
        HostEvent::Connected { peer, session } => {
            tracing::info!(peer = %peer.name, "sessão iniciada");
            *l.session.lock().unwrap() = Some(session);
            l.send(Msg::Connected { name: peer.name, fp: peer.fingerprint });
        }
        HostEvent::Disconnected { peer } => {
            tracing::info!(peer = %peer.name, "sessão encerrada");
            *l.session.lock().unwrap() = None;
            l.send(Msg::Disconnected { name: peer.name });
        }
        HostEvent::AuthFailed { peer, reason } => l.send(Msg::AuthFailed { name: peer.name, reason }),
        HostEvent::Pointer { x, y, area } => l.send(Msg::Pointer { x, y, ax: area.0, ay: area.1, aw: area.2, ah: area.3 }),
        HostEvent::Chat { peer, text } => l.send(Msg::Chat { name: peer.name, text }),
        HostEvent::FileReceived { path } => l.send(Msg::File { path: path.display().to_string() }),
    });

    hostrun::run(HostRun {
        node,
        keypair,
        download_dir: state_dir.join("recebidos"),
        settings,
        approver,
        on_event,
        status: Arc::new(Mutex::new(String::new())),
        online_text: format!("serviço online — LAN :{DIRECT_PORT}"),
        after_session: Arc::new(|| {}),
        link_out: None,
        short: Some(short_state),
    })
    .await
}

/// Entry for `elo.exe --session-worker`.
pub fn run_worker() -> anyhow::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_headless(crate::config::service_dir(), true));
    Ok(())
}
