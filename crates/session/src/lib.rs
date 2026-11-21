//! End-to-end encrypted transport between the two clients.
//!
//! The channel is a Noise `XX` handshake over 25519 / ChaChaPoly / BLAKE2s,
//! then an AEAD transport. Because `XX` transmits each side's static public
//! key, the caller can pin it against the key the relay reported in
//! `ControlMsg::Paired`, defeating a malicious relay that tries to MITM.
//!
//! On top of Noise's 65535-byte message cap, [`EncryptedStream`] adds its own
//! length framing so callers can send/recv arbitrarily large logical messages
//! (video frames, file chunks): a logical message is `u32` length-prefixed,
//! then split across as many 64 KiB Noise messages as needed, each written as
//! `u16` ciphertext-length + ciphertext.
//!
//! CONTRACT — the signatures below are fixed; the bodies are the implementation.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Result};
use snow::{params::NoiseParams, Builder, HandshakeState, TransportState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

/// The Noise handshake pattern this transport speaks.
const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

/// Largest logical message a caller may send or receive (16 MiB). Bounds
/// reassembly memory and makes a hostile length prefix cheap to reject.
const MAX_LOGICAL: usize = 16 * 1024 * 1024;

/// Largest plaintext we put in one Noise transport message. The Noise cap is
/// 65535 bytes of ciphertext; ChaChaPoly adds a 16-byte tag, so plaintext must
/// stay at or below 65519.
const MAX_PT_CHUNK: usize = 65519;

fn params() -> Result<NoiseParams> {
    PATTERN
        .parse()
        .map_err(|e| anyhow!("invalid noise params: {e}"))
}

/// A Noise static keypair. `public` is what a device registers with the relay
/// and what the peer pins.
#[derive(Clone)]
pub struct Keypair {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

/// Generate a fresh X25519 static keypair.
pub fn generate_keypair() -> Result<Keypair> {
    let kp = Builder::new(params()?)
        .generate_keypair()
        .map_err(|e| anyhow!("keypair generation failed: {e}"))?;
    Ok(Keypair {
        private: kp.private,
        public: kp.public,
    })
}

/// An established, encrypted, framed transport.
pub struct EncryptedStream<S> {
    stream: S,
    transport: TransportState,
    remote_static: Vec<u8>,
    /// Decrypted bytes received but not yet delivered as a complete logical
    /// message (reassembly buffer).
    inbuf: Vec<u8>,
}

impl<S> EncryptedStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    /// The peer's static public key, learned during the handshake. Callers
    /// MUST compare this to the pubkey the relay advertised.
    pub fn remote_static(&self) -> &[u8] {
        &self.remote_static
    }

    /// Send one logical message (any length up to 16 MiB).
    pub async fn send(&mut self, plaintext: &[u8]) -> Result<()> {
        if plaintext.len() > MAX_LOGICAL {
            bail!(
                "logical message of {} bytes exceeds MAX_LOGICAL ({MAX_LOGICAL})",
                plaintext.len()
            );
        }
        let mut framed = Vec::with_capacity(4 + plaintext.len());
        framed.extend_from_slice(&(plaintext.len() as u32).to_be_bytes());
        framed.extend_from_slice(plaintext);

        for chunk in framed.chunks(MAX_PT_CHUNK) {
            let mut ct = vec![0u8; chunk.len() + 16];
            let n = self
                .transport
                .write_message(chunk, &mut ct)
                .map_err(|e| anyhow!("noise encrypt: {e}"))?;
            // n <= 65535, fits a u16; n >= 16 (the tag) so it is never zero.
            self.stream.write_all(&(n as u16).to_be_bytes()).await?;
            self.stream.write_all(&ct[..n]).await?;
        }
        self.stream.flush().await?;
        Ok(())
    }

    /// Receive one logical message.
    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        loop {
            if self.inbuf.len() >= 4 {
                let len = u32::from_be_bytes(self.inbuf[0..4].try_into().unwrap()) as usize;
                if len > MAX_LOGICAL {
                    bail!("incoming logical message of {len} bytes exceeds MAX_LOGICAL");
                }
                if self.inbuf.len() >= 4 + len {
                    let body = self.inbuf[4..4 + len].to_vec();
                    self.inbuf.drain(0..4 + len);
                    return Ok(body);
                }
            }
            self.read_wire_chunk().await?;
        }
    }

    /// Read exactly one on-wire Noise message, decrypt it, and append the
    /// plaintext to the reassembly buffer.
    async fn read_wire_chunk(&mut self) -> Result<()> {
        let mut lenb = [0u8; 2];
        self.stream.read_exact(&mut lenb).await?;
        let ctlen = u16::from_be_bytes(lenb) as usize;
        if ctlen < 16 {
            bail!("ciphertext chunk of {ctlen} bytes is smaller than the AEAD tag");
        }
        let mut ct = vec![0u8; ctlen];
        self.stream.read_exact(&mut ct).await?;
        let mut pt = vec![0u8; ctlen];
        let n = self
            .transport
            .read_message(&ct, &mut pt)
            .map_err(|e| anyhow!("noise decrypt: {e}"))?;
        pt.truncate(n);
        self.inbuf.extend_from_slice(&pt);
        Ok(())
    }
}

/// Write one length-prefixed (`u16` BE) handshake message produced by `hs`.
async fn write_hs<S>(stream: &mut S, hs: &mut HandshakeState) -> Result<()>
where
    S: AsyncWrite + Unpin + Send,
{
    let mut buf = vec![0u8; 65535];
    let n = hs
        .write_message(&[], &mut buf)
        .map_err(|e| anyhow!("noise handshake write: {e}"))?;
    stream.write_all(&(n as u16).to_be_bytes()).await?;
    stream.write_all(&buf[..n]).await?;
    stream.flush().await?;
    Ok(())
}

/// Read one length-prefixed (`u16` BE) handshake message and feed it to `hs`.
async fn read_hs<S>(stream: &mut S, hs: &mut HandshakeState) -> Result<()>
where
    S: AsyncRead + Unpin + Send,
{
    let mut lenb = [0u8; 2];
    stream.read_exact(&mut lenb).await?;
    let len = u16::from_be_bytes(lenb) as usize;
    let mut msg = vec![0u8; len];
    stream.read_exact(&mut msg).await?;
    let mut payload = vec![0u8; 65535];
    hs.read_message(&msg, &mut payload)
        .map_err(|e| anyhow!("noise handshake read: {e}"))?;
    Ok(())
}

fn finish<S>(stream: S, hs: HandshakeState) -> Result<EncryptedStream<S>> {
    let remote_static = hs
        .get_remote_static()
        .ok_or_else(|| anyhow!("peer did not present a static key"))?
        .to_vec();
    let transport = hs
        .into_transport_mode()
        .map_err(|e| anyhow!("entering transport mode: {e}"))?;
    Ok(EncryptedStream {
        stream,
        transport,
        remote_static,
        inbuf: Vec::new(),
    })
}

/// Run the Noise `XX` handshake as the initiator (the controller).
/// Messages: write e, read e/ee/s/es, write s/se.
pub async fn handshake_initiator<S>(mut stream: S, local: &Keypair) -> Result<EncryptedStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut hs = Builder::new(params()?)
        .local_private_key(&local.private)
        .build_initiator()
        .map_err(|e| anyhow!("build initiator: {e}"))?;
    write_hs(&mut stream, &mut hs).await?;
    read_hs(&mut stream, &mut hs).await?;
    write_hs(&mut stream, &mut hs).await?;
    finish(stream, hs)
}

/// Run the Noise `XX` handshake as the responder (the host).
/// Messages: read e, write e/ee/s/es, read s/se.
pub async fn handshake_responder<S>(mut stream: S, local: &Keypair) -> Result<EncryptedStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut hs = Builder::new(params()?)
        .local_private_key(&local.private)
        .build_responder()
        .map_err(|e| anyhow!("build responder: {e}"))?;
    read_hs(&mut stream, &mut hs).await?;
    write_hs(&mut stream, &mut hs).await?;
    read_hs(&mut stream, &mut hs).await?;
    finish(stream, hs)
}

// ----------------------------------------------------------------------------
// Concurrent read/write halves.
//
// The host sends video frames while simultaneously receiving input, so it
// needs to read and write at the same time. `split` separates the socket into
// independent halves and shares the single Noise `TransportState` behind a
// mutex. The two Noise directions use independent nonce counters, so
// interleaving a `write_message` and a `read_message` on the same
// `TransportState` is correct. The mutex is held ONLY across the (fast,
// non-blocking) crypto call and never across a socket `.await`.
// ----------------------------------------------------------------------------

/// Lock the shared transport, run `f` (a crypto call), and release before any
/// further `.await`. Returns `f`'s output. Keeps the std-mutex guard from ever
/// crossing an await point.
fn with_transport<T>(
    transport: &Mutex<TransportState>,
    f: impl FnOnce(&mut TransportState) -> Result<T>,
) -> Result<T> {
    let mut guard = transport
        .lock()
        .map_err(|_| anyhow!("noise transport mutex poisoned"))?;
    f(&mut guard)
}

/// Encrypt `plaintext` into the framed on-wire chunks, doing all crypto up
/// front (under the lock) so the socket writes afterwards hold no lock.
fn seal_frames(transport: &Mutex<TransportState>, plaintext: &[u8]) -> Result<Vec<Vec<u8>>> {
    if plaintext.len() > MAX_LOGICAL {
        bail!(
            "logical message of {} bytes exceeds MAX_LOGICAL ({MAX_LOGICAL})",
            plaintext.len()
        );
    }
    let mut framed = Vec::with_capacity(4 + plaintext.len());
    framed.extend_from_slice(&(plaintext.len() as u32).to_be_bytes());
    framed.extend_from_slice(plaintext);

    with_transport(transport, |t| {
        let mut out = Vec::new();
        for chunk in framed.chunks(MAX_PT_CHUNK) {
            let mut ct = vec![0u8; chunk.len() + 16];
            let n = t
                .write_message(chunk, &mut ct)
                .map_err(|e| anyhow!("noise encrypt: {e}"))?;
            ct.truncate(n);
            out.push(ct);
        }
        Ok(out)
    })
}

impl<S> EncryptedStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    /// Split into independent sender and receiver halves for concurrent
    /// read/write, sharing the Noise transport state.
    pub fn split(self) -> (EncryptedSender<S>, EncryptedReceiver<S>) {
        let (read, write) = tokio::io::split(self.stream);
        let transport = Arc::new(Mutex::new(self.transport));
        let sender = EncryptedSender {
            write,
            transport: transport.clone(),
        };
        let receiver = EncryptedReceiver {
            read,
            transport,
            remote_static: self.remote_static,
            inbuf: self.inbuf,
        };
        (sender, receiver)
    }
}

/// The write half of a split [`EncryptedStream`].
pub struct EncryptedSender<S> {
    write: WriteHalf<S>,
    transport: Arc<Mutex<TransportState>>,
}

impl<S> EncryptedSender<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    /// Send one logical message (same framing as [`EncryptedStream::send`]).
    pub async fn send(&mut self, plaintext: &[u8]) -> Result<()> {
        let frames = seal_frames(&self.transport, plaintext)?;
        for ct in frames {
            self.write.write_all(&(ct.len() as u16).to_be_bytes()).await?;
            self.write.write_all(&ct).await?;
        }
        self.write.flush().await?;
        Ok(())
    }
}

/// The read half of a split [`EncryptedStream`].
pub struct EncryptedReceiver<S> {
    read: ReadHalf<S>,
    transport: Arc<Mutex<TransportState>>,
    remote_static: Vec<u8>,
    inbuf: Vec<u8>,
}

impl<S> EncryptedReceiver<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    /// The peer's static public key, learned during the handshake.
    pub fn remote_static(&self) -> &[u8] {
        &self.remote_static
    }

    /// Receive one logical message (same framing as [`EncryptedStream::recv`]).
    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        loop {
            if self.inbuf.len() >= 4 {
                let len = u32::from_be_bytes(self.inbuf[0..4].try_into().unwrap()) as usize;
                if len > MAX_LOGICAL {
                    bail!("incoming logical message of {len} bytes exceeds MAX_LOGICAL");
                }
                if self.inbuf.len() >= 4 + len {
                    let body = self.inbuf[4..4 + len].to_vec();
                    self.inbuf.drain(0..4 + len);
                    return Ok(body);
                }
            }
            // Read one on-wire chunk (socket await holds no lock) ...
            let mut lenb = [0u8; 2];
            self.read.read_exact(&mut lenb).await?;
            let ctlen = u16::from_be_bytes(lenb) as usize;
            if ctlen < 16 {
                bail!("ciphertext chunk of {ctlen} bytes is smaller than the AEAD tag");
            }
            let mut ct = vec![0u8; ctlen];
            self.read.read_exact(&mut ct).await?;
            // ... then decrypt under the lock, releasing it immediately.
            let pt = with_transport(&self.transport, |t| {
                let mut pt = vec![0u8; ctlen];
                let n = t
                    .read_message(&ct, &mut pt)
                    .map_err(|e| anyhow!("noise decrypt: {e}"))?;
                pt.truncate(n);
                Ok(pt)
            })?;
            self.inbuf.extend_from_slice(&pt);
        }
    }
}
