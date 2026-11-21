//! Elo relay / rendezvous server.
//!
//! A host registers under a 9-digit device id and waits. A controller asks for
//! that id; the relay hands each side the other's public key ([`ControlMsg::Paired`])
//! and then splices the two TCP streams, copying bytes blindly in both
//! directions. From that point the traffic is the clients' end-to-end
//! encrypted session, which the relay cannot read.
//!
//! Run: `elo-relay 0.0.0.0:9000`

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use elo_protocol::{decode, encode, ControlMsg, DeviceId, PubKey, MAX_FRAME};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};
use tracing::{info, warn};

/// A controller arriving for a registered host, handed to the host's task.
struct ControllerArrival {
    stream: TcpStream,
    pubkey: PubKey,
}

/// device id -> the channel that wakes the waiting host task.
type Registry = Arc<Mutex<HashMap<DeviceId, oneshot::Sender<ControllerArrival>>>>;

/// Directory: short device id -> iroh node id, for P2P short-id lookup. Lives
/// while the host's connection is open. The relay never sees session traffic.
type Directory = Arc<Mutex<HashMap<DeviceId, String>>>;

/// Accept connections forever on `listener`, pairing hosts and controllers and
/// serving the short-id directory. Kept as a library entry point so integration
/// tests can drive a real relay on an ephemeral port.
pub async fn serve(listener: TcpListener) -> Result<()> {
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    let directory: Directory = Arc::new(Mutex::new(HashMap::new()));
    loop {
        let (stream, peer) = listener.accept().await?;
        let registry = registry.clone();
        let directory = directory.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, registry, directory).await {
                warn!(%peer, error = %e, "connection closed with error");
            }
        });
    }
}

async fn handle_conn(mut stream: TcpStream, registry: Registry, directory: Directory) -> Result<()> {
    let hello = read_frame(&mut stream).await.context("reading hello")?;
    match decode(&hello).context("decoding hello")? {
        ControlMsg::RegisterHost { device_id, pubkey } => {
            handle_host(stream, registry, device_id, pubkey).await
        }
        ControlMsg::Connect { target_id, pubkey } => {
            handle_controller(stream, registry, target_id, pubkey).await
        }
        ControlMsg::Register { device_id, node_id } => {
            handle_dir_register(stream, directory, device_id, node_id).await
        }
        ControlMsg::Resolve { device_id } => {
            handle_dir_resolve(stream, directory, device_id).await
        }
        other => bail!("unexpected first message: {other:?}"),
    }
}

/// Directory host: publish `device_id -> node_id`, ack, and keep the mapping
/// alive until this connection closes.
async fn handle_dir_register(
    mut stream: TcpStream,
    directory: Directory,
    device_id: DeviceId,
    node_id: String,
) -> Result<()> {
    if !valid_id(&device_id) {
        write_msg(&mut stream, &ControlMsg::Error { reason: "invalid device id".into() }).await?;
        bail!("directory register with invalid id {device_id:?}");
    }
    directory.lock().await.insert(device_id.clone(), node_id);
    write_msg(&mut stream, &ControlMsg::Registered).await?;
    info!(%device_id, "directory host registered");

    // Hold the mapping until the host disconnects.
    let mut buf = [0u8; 256];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {} // ignore any keepalive bytes
        }
    }
    directory.lock().await.remove(&device_id);
    info!(%device_id, "directory host left");
    Ok(())
}

/// Directory query: resolve a short id to a node id.
async fn handle_dir_resolve(
    mut stream: TcpStream,
    directory: Directory,
    device_id: DeviceId,
) -> Result<()> {
    let node = directory.lock().await.get(&device_id).cloned();
    match node {
        Some(node_id) => write_msg(&mut stream, &ControlMsg::Resolved { node_id }).await?,
        None => {
            write_msg(
                &mut stream,
                &ControlMsg::Error { reason: format!("id {device_id} is not online") },
            )
            .await?
        }
    }
    Ok(())
}

async fn handle_host(
    mut stream: TcpStream,
    registry: Registry,
    device_id: DeviceId,
    host_pubkey: PubKey,
) -> Result<()> {
    if !valid_id(&device_id) {
        write_msg(
            &mut stream,
            &ControlMsg::Error {
                reason: "invalid device id".into(),
            },
        )
        .await?;
        bail!("host sent invalid id {device_id:?}");
    }

    let (tx, rx) = oneshot::channel::<ControllerArrival>();
    {
        let mut reg = registry.lock().await;
        if reg.contains_key(&device_id) {
            drop(reg);
            write_msg(
                &mut stream,
                &ControlMsg::Error {
                    reason: "device id already registered".into(),
                },
            )
            .await?;
            bail!("duplicate registration for {device_id}");
        }
        reg.insert(device_id.clone(), tx);
    }
    // Ensure the id is freed no matter how this task ends.
    let _guard = RegistryGuard {
        registry: registry.clone(),
        id: device_id.clone(),
    };

    write_msg(&mut stream, &ControlMsg::Registered).await?;
    info!(%device_id, "host registered, waiting for controller");

    // Wait for a controller (or for the relay to drop the sender).
    let arrival = match rx.await {
        Ok(a) => a,
        Err(_) => bail!("host {device_id} woke without a controller"),
    };

    // Tell the host who is connecting, then splice.
    write_msg(
        &mut stream,
        &ControlMsg::Paired {
            peer_pubkey: arrival.pubkey,
        },
    )
    .await?;

    let mut controller = arrival.stream;
    // Hand the host its peer's identity on the controller side too.
    write_msg(
        &mut controller,
        &ControlMsg::Paired {
            peer_pubkey: host_pubkey,
        },
    )
    .await?;

    info!(%device_id, "paired; splicing session");
    // Blind bidirectional copy of the opaque E2E session.
    tokio::io::copy_bidirectional(&mut stream, &mut controller)
        .await
        .context("splicing session")?;
    Ok(())
}

async fn handle_controller(
    mut stream: TcpStream,
    registry: Registry,
    target_id: DeviceId,
    controller_pubkey: PubKey,
) -> Result<()> {
    let tx = {
        let mut reg = registry.lock().await;
        reg.remove(&target_id)
    };
    let Some(tx) = tx else {
        write_msg(
            &mut stream,
            &ControlMsg::Error {
                reason: format!("device {target_id} is not online"),
            },
        )
        .await?;
        bail!("controller asked for offline id {target_id}");
    };

    // Hand our stream to the host task, which drives the splice.
    if tx
        .send(ControllerArrival {
            stream,
            pubkey: controller_pubkey,
        })
        .is_err()
    {
        bail!("host {target_id} vanished before pairing");
    }
    Ok(())
}

/// Removes a host's id from the registry when its task ends.
struct RegistryGuard {
    registry: Registry,
    id: DeviceId,
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        let registry = self.registry.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            registry.lock().await.remove(&id);
        });
    }
}

fn valid_id(id: &DeviceId) -> bool {
    id.len() == 9 && id.bytes().all(|b| b.is_ascii_digit())
}

/// Read one length-prefixed frame body (`u32` BE length, then that many bytes).
async fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        bail!("frame length {len} exceeds MAX_FRAME");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

async fn write_msg(stream: &mut TcpStream, msg: &ControlMsg) -> Result<()> {
    let frame = encode(msg)?;
    stream.write_all(&frame).await?;
    stream.flush().await?;
    Ok(())
}
