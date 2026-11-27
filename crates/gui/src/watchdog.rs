//! Watchdog de rede/suspensão do host.
//!
//! Depois de uma suspensão ou de uma troca de rede (Wi-Fi diferente, cabo,
//! VPN), o nó P2P e o registro do ID curto podem ficar "mortos" sem erro
//! visível. Uma thread verifica a cada ~30 s:
//!
//! * salto de relógio > 60 s entre duas verificações (o sistema dormiu);
//! * mudança no conjunto de IPv4 locais (estável por duas verificações).
//!
//! Ao detectar, chama `act(motivo)`; quem chamou decide como recriar o host
//! (o worker do serviço encerra o processo para o supervisor relançar). Nunca
//! age nos primeiros 2 min de vida do processo (no máximo um reinício a cada
//! 2 min) nem com uma sessão remota em andamento (`busy`).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const TICK: Duration = Duration::from_secs(30);
/// Atraso além do `TICK` que indica que o sistema dormiu.
const JUMP: Duration = Duration::from_secs(60);
/// Intervalo mínimo entre reinícios (contado desde o início do processo).
const COOLDOWN: Duration = Duration::from_secs(120);

/// IPv4 locais (sem loopback nem link-local 169.254/16). IPv6 fica de fora:
/// endereços temporários mudam sozinhos e causariam reinícios à toa.
pub fn local_ipv4s() -> BTreeSet<String> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() => Some(v4.to_string()),
            _ => None,
        })
        .collect()
}

/// Inicia o watchdog. `act` devolve `true` quando agiu (a thread termina) e
/// `false` quando não pôde agir agora (o motivo fica pendente e `act` é
/// chamado de novo no próximo ciclo; cabe a ele não repetir avisos).
pub fn spawn(
    busy: Arc<dyn Fn() -> bool + Send + Sync>,
    act: Box<dyn Fn(&str) -> bool + Send>,
) {
    let _ = std::thread::Builder::new().name("elo-watchdog".into()).spawn(move || {
        let started = Instant::now();
        let mut last_wall = SystemTime::now();
        let mut last_mono = Instant::now();
        let mut baseline = local_ipv4s();
        let mut candidate: Option<BTreeSet<String>> = None;
        let mut pending: Option<String> = None;
        tracing::info!(ips = ?baseline, "watchdog de rede/suspensão ativo");
        loop {
            std::thread::sleep(TICK);
            let (wall, mono) = (SystemTime::now(), Instant::now());
            let gap = wall.duration_since(last_wall).unwrap_or_default().max(mono - last_mono);
            (last_wall, last_mono) = (wall, mono);
            if gap > TICK + JUMP && pending.is_none() {
                pending = Some(format!("salto de relógio de {} s (o sistema suspendeu?)", gap.as_secs()));
            }

            let ips = local_ipv4s();
            if ips.is_empty() || ips == baseline {
                candidate = None;
            } else if candidate.as_ref() == Some(&ips) {
                if pending.is_none() {
                    pending = Some(format!("rede mudou: {baseline:?} -> {ips:?}"));
                }
                baseline = ips;
                candidate = None;
            } else {
                candidate = Some(ips);
            }

            let Some(reason) = pending.clone() else { continue };
            if started.elapsed() < COOLDOWN || busy() {
                continue;
            }
            if act(&reason) {
                return;
            }
        }
    });
}
