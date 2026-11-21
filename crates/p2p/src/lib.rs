//! Peer-to-peer transport for Elo over the internet, built on `iroh` 1.x.
//!
//! iroh gives QUIC connections dialed by a node's public key, with NAT hole
//! punching and n0's free public relays as automatic fallback — the same idea
//! torrents/WebRTC use. We expose a [`Duplex`] (one bidirectional stream) that
//! implements `AsyncRead + AsyncWrite`, so the Elo session (Noise + codec) runs
//! on top of it unchanged via `serve_host_on` / `connect_on`.

use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{Context as _, Result};
use iroh::endpoint::presets;
use iroh::endpoint::{RecvStream, SendStream};
use iroh::{Endpoint, EndpointId, SecretKey};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Application-layer protocol id for Elo over iroh.
pub const ALPN: &[u8] = b"elo/1";

/// A single bidirectional QUIC stream presented as one duplex byte stream.
pub struct Duplex {
    send: SendStream,
    recv: RecvStream,
}

impl AsyncRead for Duplex {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        <RecvStream as AsyncRead>::poll_read(Pin::new(&mut self.recv), cx, buf)
    }
}

impl AsyncWrite for Duplex {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        <SendStream as AsyncWrite>::poll_write(Pin::new(&mut self.send), cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        <SendStream as AsyncWrite>::poll_flush(Pin::new(&mut self.send), cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        <SendStream as AsyncWrite>::poll_shutdown(Pin::new(&mut self.send), cx)
    }
}

/// Load a persistent node secret from `path`, creating and saving one if absent.
/// A stable secret keeps this machine's P2P address stable across runs.
pub fn load_or_create_secret(path: &Path) -> Result<SecretKey> {
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() == 32 {
            let arr: [u8; 32] = bytes[..32].try_into().unwrap();
            return Ok(SecretKey::from_bytes(&arr));
        }
    }
    let secret = SecretKey::generate();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, secret.to_bytes()).context("saving node secret")?;
    Ok(secret)
}

/// Bind an iroh endpoint with the N0 preset (n0 relays + discovery, so peers can
/// be dialed by id across networks) and Elo's ALPN.
pub async fn bind(secret: SecretKey) -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("binding iroh endpoint")
}

/// This endpoint's id, as the string a peer types to reach us.
pub fn node_id_string(ep: &Endpoint) -> String {
    ep.id().to_string()
}

/// Accept the next incoming P2P connection and open its session stream.
pub async fn accept(ep: &Endpoint) -> Result<Duplex> {
    let incoming = ep.accept().await.context("endpoint closed")?;
    let conn = incoming.await.context("accepting connection")?;
    let (send, recv) = conn.accept_bi().await.context("accepting bi stream")?;
    Ok(Duplex { send, recv })
}

/// Dial a host by its id string and open a session stream.
pub async fn connect(ep: &Endpoint, node: &str) -> Result<Duplex> {
    let node_id: EndpointId = node.trim().parse().context("invalid node id")?;
    let conn = ep.connect(node_id, ALPN).await.context("connecting")?;
    let (send, recv) = conn.open_bi().await.context("opening bi stream")?;
    Ok(Duplex { send, recv })
}

/// ALPN used for presence checks: the host accepts and closes immediately.
pub const PING_ALPN: &[u8] = b"elo/ping/1";

/// A bound P2P node, usable as both host (accept) and controller (connect).
/// Cheap to clone (the endpoint is reference-counted internally).
///
/// Incoming connections are accepted by a background task, concurrently: a
/// slow or idle peer cannot block others, and presence pings (see
/// [`PING_ALPN`]) are answered there without ever reaching [`Node::accept`].
#[derive(Clone)]
pub struct Node {
    ep: Endpoint,
    incoming: std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Duplex>>>,
}

/// Bind a persistent P2P node using a secret stored at `secret_path`.
pub async fn bind_node(secret_path: &Path) -> Result<Node> {
    let secret = load_or_create_secret(secret_path)?;
    Node::from_secret(secret).await
}

/// Bind a throw-away node (fresh key, not persisted) — for controller-only use.
pub async fn bind_node_ephemeral() -> Result<Node> {
    Node::from_secret(SecretKey::generate()).await
}

/// The Elo address (node id) stored in a secret-key file, without binding.
pub fn id_from_secret_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let arr: [u8; 32] = bytes.get(..32)?.try_into().ok()?;
    Some(SecretKey::from_bytes(&arr).public().to_string())
}

impl Node {
    async fn from_secret(secret: SecretKey) -> Result<Node> {
        let ep = Endpoint::builder(presets::N0)
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec(), PING_ALPN.to_vec()])
            .bind()
            .await
            .context("binding iroh endpoint")?;
        let (tx, rx) = tokio::sync::mpsc::channel::<Duplex>(8);
        let acc = ep.clone();
        tokio::spawn(async move {
            while let Some(incoming) = acc.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else { return };
                    if conn.alpn() == PING_ALPN {
                        // Presence probe: answering the handshake is the answer.
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        conn.close(0u32.into(), b"pong");
                        return;
                    }
                    let bi = tokio::time::timeout(std::time::Duration::from_secs(20), conn.accept_bi()).await;
                    if let Ok(Ok((send, recv))) = bi {
                        let _ = tx.send(Duplex { send, recv }).await;
                    }
                });
            }
        });
        Ok(Node { ep, incoming: std::sync::Arc::new(tokio::sync::Mutex::new(rx)) })
    }

    /// This node's address string (what a peer types to reach it).
    pub fn id(&self) -> String {
        node_id_string(&self.ep)
    }

    /// Next incoming session stream.
    pub async fn accept(&self) -> Result<Duplex> {
        self.incoming.lock().await.recv().await.context("endpoint closed")
    }

    /// Dial a peer by id and open a session stream.
    pub async fn connect(&self, node: &str) -> Result<Duplex> {
        connect(&self.ep, node).await
    }

    /// Is the peer reachable right now? (Quick presence probe.)
    pub async fn ping(&self, node: &str, timeout: std::time::Duration) -> bool {
        let Ok(node_id) = node.trim().parse::<EndpointId>() else { return false };
        match tokio::time::timeout(timeout, self.ep.connect(node_id, PING_ALPN)).await {
            Ok(Ok(conn)) => {
                conn.close(0u32.into(), b"bye");
                true
            }
            _ => false,
        }
    }
}
