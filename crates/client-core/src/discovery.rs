//! Descoberta de dispositivos na rede local (LAN) por broadcast UDP.
//!
//! O host (ou qualquer par que queira ser encontrado) sobe um [`Announcer`],
//! que envia um pequeno datagrama JSON a cada ~2 s para o endereço de broadcast.
//! Quem procura chama [`listen`] e consulta [`DiscoveryHandle::devices`] para a
//! lista atual. Tudo em `std` + `serde_json`; nenhuma dependência de rede extra.
//!
//! Formato do datagrama: `{"elo":1,"name":<name>,"id":<short_id>,"port":<port>}`.
//! Pacotes sem `"elo":1` ou malformados são simplesmente ignorados.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Porta UDP usada pelo anúncio e pela escuta de descoberta na LAN.
pub const DISCOVERY_PORT: u16 = 9001;

/// Por quanto tempo um dispositivo continua na lista sem ser visto de novo.
const STALE_AFTER: Duration = Duration::from_secs(10);
/// Intervalo entre anúncios.
const ANNOUNCE_EVERY: Duration = Duration::from_secs(2);

/// Um dispositivo anunciado na LAN, montado a partir de um datagrama recebido.
#[derive(Clone, Debug)]
pub struct DiscoveredDevice {
    /// Nome amigável anunciado pelo dispositivo.
    pub name: String,
    /// IP de origem do pacote (de onde o anúncio veio de fato).
    pub ip: String,
    /// Porta em que o dispositivo aceita conexões.
    pub port: u16,
    /// Identificador curto anunciado (para parear/distinguir dispositivos).
    pub short_id: String,
}

/// Anuncia este dispositivo na LAN enquanto estiver vivo.
///
/// Mantém uma thread de fundo que envia o datagrama JSON a cada ~2 s para
/// `255.255.255.255:DISCOVERY_PORT`. Ao sofrer `drop`, sinaliza a parada e
/// aguarda a thread encerrar.
pub struct Announcer {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

/// Inicia o anúncio periódico deste dispositivo na LAN.
pub fn start(name: String, short_id: String, port: u16) -> Announcer {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let handle = thread::spawn(move || {
        let payload = serde_json::json!({
            "elo": 1,
            "name": name,
            "id": short_id,
            "port": port,
        })
        .to_string();
        while !stop_thread.load(Ordering::Relaxed) {
            announce_once(payload.as_bytes());
            // Dorme ~2 s em passos curtos para reagir rápido ao pedido de parada.
            let mut slept = Duration::ZERO;
            while slept < ANNOUNCE_EVERY && !stop_thread.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(100));
                slept += Duration::from_millis(100);
            }
        }
    });
    Announcer { stop, handle: Some(handle) }
}

/// Envia o anúncio por TODAS as interfaces IPv4 ativas. Para cada uma, usa o
/// endereço de broadcast dirigido da sub-rede (ex.: `192.168.0.255`), com o
/// socket preso ao IP daquela interface — assim o pacote sai pela interface
/// certa. Isso resolve máquinas com vários adaptadores (Wi-Fi + WSL/Hyper-V/
/// VPN), onde um broadcast "limitado" (`255.255.255.255`) iria por uma só.
/// Broadcast dirigido também costuma passar melhor por filtros (inclusive no
/// Android). `255.255.255.255` é enviado junto, como reforço.
fn announce_once(payload: &[u8]) {
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        if iface.is_loopback() {
            continue;
        }
        let if_addrs::IfAddr::V4(v4) = iface.addr else {
            continue;
        };
        if v4.ip.is_unspecified() {
            continue;
        }
        let bcast = v4.broadcast.unwrap_or_else(|| directed_broadcast(v4.ip, v4.netmask));
        if let Ok(sock) = UdpSocket::bind((v4.ip, 0)) {
            let _ = sock.set_broadcast(true);
            let _ = sock.send_to(payload, (bcast, DISCOVERY_PORT));
            let _ = sock.send_to(payload, (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
        }
    }
    // Reforço: broadcast limitado a partir de um socket padrão (interface do
    // sistema), caso a enumeração acima não cubra alguma rota.
    if let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)) {
        let _ = sock.set_broadcast(true);
        let _ = sock.send_to(payload, (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
    }
}

/// Broadcast dirigido de uma sub-rede: `ip | !netmask`.
fn directed_broadcast(ip: Ipv4Addr, mask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(ip) | !u32::from(mask))
}

impl Drop for Announcer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

struct Entry {
    device: DiscoveredDevice,
    last_seen: Instant,
}

struct Shared {
    devices: Mutex<HashMap<String, Entry>>,
}

/// Alça para consultar os dispositivos descobertos. Clonável e `Send + Sync`
/// (estado interno em `Arc`). A thread de escuta para quando a última alça é
/// solta.
#[derive(Clone)]
pub struct DiscoveryHandle {
    shared: Arc<Shared>,
}

/// Começa a escutar anúncios na LAN.
///
/// Faz bind em `0.0.0.0:DISCOVERY_PORT` e sobe uma thread que lê datagramas,
/// mantém um mapa com chave = IP de origem e poda entradas não vistas há mais
/// de 10 s. Erros de parse não derrubam a thread.
pub fn listen() -> io::Result<DiscoveryHandle> {
    let sock = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT))?;
    // Timeout de leitura para acordar a thread periodicamente (podar + checar
    // se ainda há alças vivas) mesmo sem pacotes chegando.
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;

    let shared = Arc::new(Shared { devices: Mutex::new(HashMap::new()) });
    let weak = Arc::downgrade(&shared);

    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        loop {
            // Se não há mais nenhuma alça viva, encerra.
            let shared = match weak.upgrade() {
                Some(s) => s,
                None => break,
            };
            match sock.recv_from(&mut buf) {
                Ok((n, src)) => {
                    if let Some(device) = parse_announcement(&buf[..n], src.ip()) {
                        let mut map = shared.devices.lock().unwrap();
                        map.insert(device.ip.clone(), Entry { device, last_seen: Instant::now() });
                    }
                }
                // Timeout/sem dados: segue para a poda.
                Err(_) => {}
            }
            prune(&shared.devices);
            // Solta o Arc antes de reavaliar `weak.upgrade()` no topo do laço.
            drop(shared);
        }
    });

    Ok(DiscoveryHandle { shared })
}

impl DiscoveryHandle {
    /// Lista atual de dispositivos descobertos (já podando os obsoletos).
    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        prune(&self.shared.devices);
        let map = self.shared.devices.lock().unwrap();
        map.values().map(|e| e.device.clone()).collect()
    }
}

fn prune(devices: &Mutex<HashMap<String, Entry>>) {
    let mut map = devices.lock().unwrap();
    map.retain(|_, e| e.last_seen.elapsed() < STALE_AFTER);
}

/// Decodifica um datagrama de anúncio. Retorna `None` para pacotes malformados
/// ou sem `"elo":1`, sem derrubar a thread de escuta.
fn parse_announcement(bytes: &[u8], src: IpAddr) -> Option<DiscoveredDevice> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if v.get("elo").and_then(|x| x.as_i64()) != Some(1) {
        return None;
    }
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let short_id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let port = v.get("port").and_then(|x| x.as_u64())? as u16;
    Some(DiscoveredDevice { name, ip: src.to_string(), port, short_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_announcement() {
        let pkt = br#"{"elo":1,"name":"PC do Ricardo","id":"ab12","port":9000}"#;
        let d = parse_announcement(pkt, "192.168.0.5".parse().unwrap()).unwrap();
        assert_eq!(d.name, "PC do Ricardo");
        assert_eq!(d.short_id, "ab12");
        assert_eq!(d.port, 9000);
        assert_eq!(d.ip, "192.168.0.5");
    }

    #[test]
    fn rejects_malformed_and_foreign() {
        assert!(parse_announcement(b"not json", "10.0.0.1".parse().unwrap()).is_none());
        assert!(parse_announcement(br#"{"elo":2,"port":9000}"#, "10.0.0.1".parse().unwrap()).is_none());
        assert!(parse_announcement(br#"{"name":"x"}"#, "10.0.0.1".parse().unwrap()).is_none());
    }
}
