//! Loopback test of the Noise XX transport over an in-memory duplex pipe:
//! the handshake completes, each side sees the other's real static key, and
//! logical messages — including one large enough to force multi-chunk framing
//! — round-trip in both directions.

use elo_session::{generate_keypair, handshake_initiator, handshake_responder};
use tokio::io::{AsyncRead, AsyncWrite};

/// Cheap deterministic pseudo-random bytes (xorshift64), no extra deps.
fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut out = vec![0u8; len];
    let mut s = seed.wrapping_add(0x9e3779b97f4a7c15);
    for b in out.iter_mut() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s & 0xff) as u8;
    }
    out
}

#[tokio::test]
async fn xx_handshake_keypinning_and_large_roundtrip() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);

    let host_kp = generate_keypair().unwrap();
    let ctl_kp = generate_keypair().unwrap();
    let host_pub = host_kp.public.clone();
    let ctl_pub = ctl_kp.public.clone();

    // Responder (host) side runs concurrently.
    let server = tokio::spawn(async move {
        let mut enc = handshake_responder(server_io, &host_kp).await.unwrap();
        // The responder must see the initiator's (controller's) static key.
        assert_eq!(enc.remote_static(), ctl_pub.as_slice());

        // Receive the small message, then the large one.
        let small = enc.recv().await.unwrap();
        assert_eq!(small, b"ola host");
        let big = enc.recv().await.unwrap();
        assert_eq!(big, pseudo_random(200_000, 7));

        // Reply.
        enc.send(b"ola controller").await.unwrap();
        enc.send(&pseudo_random(123_457, 99)).await.unwrap();
    });

    // Initiator (controller) side.
    let mut enc = handshake_initiator(client_io, &ctl_kp).await.unwrap();
    // The initiator must see the responder's (host's) static key.
    assert_eq!(enc.remote_static(), host_pub.as_slice());

    enc.send(b"ola host").await.unwrap();
    enc.send(&pseudo_random(200_000, 7)).await.unwrap();

    let reply = enc.recv().await.unwrap();
    assert_eq!(reply, b"ola controller");
    let big_reply = enc.recv().await.unwrap();
    assert_eq!(big_reply, pseudo_random(123_457, 99));

    server.await.unwrap();
}

#[tokio::test]
async fn empty_and_boundary_messages_roundtrip() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let host_kp = generate_keypair().unwrap();
    let ctl_kp = generate_keypair().unwrap();

    let server = tokio::spawn(async move {
        let mut enc = handshake_responder(server_io, &host_kp).await.unwrap();
        for _ in 0..3 {
            let m = enc.recv().await.unwrap();
            enc.send(&m).await.unwrap(); // echo
        }
    });

    let mut enc = handshake_initiator(client_io, &ctl_kp).await.unwrap();
    // Empty, exactly one plaintext chunk, and just over one chunk.
    for payload in [vec![], pseudo_random(65519, 1), pseudo_random(65520, 2)] {
        enc.send(&payload).await.unwrap();
        assert_eq!(enc.recv().await.unwrap(), payload);
    }
    server.await.unwrap();
}

#[tokio::test]
async fn split_allows_concurrent_bidirectional_traffic() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let host_kp = generate_keypair().unwrap();
    let ctl_kp = generate_keypair().unwrap();
    let ctl_pub = ctl_kp.public.clone();

    const N: usize = 50;

    // Host: split, then read N messages while concurrently writing N messages.
    let server = tokio::spawn(async move {
        let enc = handshake_responder(server_io, &host_kp).await.unwrap();
        let (mut tx, rx) = enc.split();
        assert_eq!(rx.remote_static(), ctl_pub.as_slice());

        let writer = tokio::spawn(async move {
            for i in 0..N {
                tx.send(format!("host-{i}").as_bytes()).await.unwrap();
            }
        });
        let mut rx = rx;
        for i in 0..N {
            // Messages from one direction arrive in order.
            assert_eq!(rx.recv().await.unwrap(), format!("ctl-{i}").into_bytes());
        }
        writer.await.unwrap();
    });

    // Controller: same, mirrored.
    let enc = handshake_initiator(client_io, &ctl_kp).await.unwrap();
    let (mut tx, mut rx) = enc.split();
    let writer = tokio::spawn(async move {
        for i in 0..N {
            tx.send(format!("ctl-{i}").as_bytes()).await.unwrap();
        }
    });
    for i in 0..N {
        assert_eq!(rx.recv().await.unwrap(), format!("host-{i}").into_bytes());
    }
    writer.await.unwrap();
    server.await.unwrap();
}

/// Compile-time proof that the split halves are `Send` and `'static`, i.e.
/// usable from independent `tokio::spawn` tasks.
fn _assert_halves_are_send<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>() {
    fn needs_send<T: Send + 'static>() {}
    needs_send::<elo_session::EncryptedSender<S>>();
    needs_send::<elo_session::EncryptedReceiver<S>>();
}
