//! Talking to the relay: register a host, or connect a controller to a host's
//! device id. Returns the raw TCP stream (ready for the Noise handshake) plus
//! the peer's advertised public key, which the caller pins after the handshake.

use anyhow::{bail, Result};
use elo_protocol::{decode, encode, ControlMsg, MAX_FRAME};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn write_ctl(s: &mut TcpStream, m: &ControlMsg) -> Result<()> {
    let frame = encode(m)?;
    s.write_all(&frame).await?;
    s.flush().await?;
    Ok(())
}

async fn read_ctl(s: &mut TcpStream) -> Result<ControlMsg> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        bail!("control frame of {n} bytes exceeds MAX_FRAME");
    }
    let mut body = vec![0u8; n];
    s.read_exact(&mut body).await?;
    Ok(decode(&body)?)
}

/// Register as a host and block until a controller is routed to us. Returns the
/// live stream and the controller's advertised public key.
pub async fn register_host(
    relay: &str,
    device_id: &str,
    pubkey: &[u8],
) -> Result<(TcpStream, Vec<u8>)> {
    let mut s = TcpStream::connect(relay).await?;
    write_ctl(
        &mut s,
        &ControlMsg::RegisterHost {
            device_id: device_id.to_string(),
            pubkey: pubkey.to_vec(),
        },
    )
    .await?;
    match read_ctl(&mut s).await? {
        ControlMsg::Registered => {}
        ControlMsg::Error { reason } => bail!("relay rejected registration: {reason}"),
        m => bail!("unexpected reply to RegisterHost: {m:?}"),
    }
    match read_ctl(&mut s).await? {
        ControlMsg::Paired { peer_pubkey } => Ok((s, peer_pubkey)),
        ControlMsg::Error { reason } => bail!("relay error while waiting: {reason}"),
        m => bail!("unexpected reply while waiting for controller: {m:?}"),
    }
}

/// Connect to a host by its device id. Returns the live stream and the host's
/// advertised public key.
pub async fn connect_controller(
    relay: &str,
    target_id: &str,
    pubkey: &[u8],
) -> Result<(TcpStream, Vec<u8>)> {
    let mut s = TcpStream::connect(relay).await?;
    write_ctl(
        &mut s,
        &ControlMsg::Connect {
            target_id: target_id.to_string(),
            pubkey: pubkey.to_vec(),
        },
    )
    .await?;
    match read_ctl(&mut s).await? {
        ControlMsg::Paired { peer_pubkey } => Ok((s, peer_pubkey)),
        ControlMsg::Error { reason } => bail!("relay could not connect: {reason}"),
        m => bail!("unexpected reply to Connect: {m:?}"),
    }
}

/// Directory: publish `device_id -> node_id` and keep the returned connection
/// open (the mapping lives while it is open). The caller holds the stream.
pub async fn register_directory(
    dir: &str,
    device_id: &str,
    node_id: &str,
) -> Result<TcpStream> {
    let mut s = TcpStream::connect(dir).await?;
    write_ctl(
        &mut s,
        &ControlMsg::Register { device_id: device_id.to_string(), node_id: node_id.to_string() },
    )
    .await?;
    match read_ctl(&mut s).await? {
        ControlMsg::Registered => Ok(s),
        ControlMsg::Error { reason } => bail!("directory refused: {reason}"),
        m => bail!("unexpected reply to Register: {m:?}"),
    }
}

/// Directory: resolve a short `device_id` to its node id.
pub async fn resolve_directory(dir: &str, device_id: &str) -> Result<String> {
    let mut s = TcpStream::connect(dir).await?;
    write_ctl(&mut s, &ControlMsg::Resolve { device_id: device_id.to_string() }).await?;
    match read_ctl(&mut s).await? {
        ControlMsg::Resolved { node_id } => Ok(node_id),
        ControlMsg::Error { reason } => bail!("directory: {reason}"),
        m => bail!("unexpected reply to Resolve: {m:?}"),
    }
}

/// SHA-256(salt ‖ password) — the session-password proof sent inside the
/// encrypted channel.
pub fn auth_proof(salt: &[u8], password: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(salt);
    h.update(password.as_bytes());
    h.finalize().to_vec()
}
