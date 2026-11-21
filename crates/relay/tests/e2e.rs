//! End-to-end test of the relay: a host registers, a controller reaches it,
//! both receive the peer's pubkey, and raw bytes flow across the spliced
//! session in both directions. Runs entirely on localhost — no Windows, no
//! screen capture — so it is CI-friendly on Linux.

use elo_protocol::{decode, encode, ControlMsg, MAX_FRAME};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn read_frame(s: &mut TcpStream) -> Vec<u8> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await.unwrap();
    let n = u32::from_be_bytes(len) as usize;
    assert!(n <= MAX_FRAME);
    let mut body = vec![0u8; n];
    s.read_exact(&mut body).await.unwrap();
    body
}

async fn write_msg(s: &mut TcpStream, m: &ControlMsg) {
    s.write_all(&encode(m).unwrap()).await.unwrap();
    s.flush().await.unwrap();
}

#[tokio::test]
async fn host_and_controller_pair_and_relay() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { elo_relay::serve(listener).await.unwrap() });

    let host_pk = vec![0xAAu8; 32];
    let ctl_pk = vec![0xBBu8; 32];

    // Host registers and waits.
    let mut host = TcpStream::connect(addr).await.unwrap();
    write_msg(
        &mut host,
        &ControlMsg::RegisterHost {
            device_id: "123456789".into(),
            pubkey: host_pk.clone(),
        },
    )
    .await;
    assert_eq!(decode(&read_frame(&mut host).await).unwrap(), ControlMsg::Registered);

    // Controller connects to the host's id.
    let mut ctl = TcpStream::connect(addr).await.unwrap();
    write_msg(
        &mut ctl,
        &ControlMsg::Connect {
            target_id: "123456789".into(),
            pubkey: ctl_pk.clone(),
        },
    )
    .await;

    // Each side learns the other's pubkey.
    assert_eq!(
        decode(&read_frame(&mut host).await).unwrap(),
        ControlMsg::Paired { peer_pubkey: ctl_pk }
    );
    assert_eq!(
        decode(&read_frame(&mut ctl).await).unwrap(),
        ControlMsg::Paired { peer_pubkey: host_pk }
    );

    // Opaque session bytes flow host -> controller ...
    host.write_all(b"hello-from-host").await.unwrap();
    host.flush().await.unwrap();
    let mut buf = [0u8; 15];
    ctl.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello-from-host");

    // ... and controller -> host.
    ctl.write_all(b"hi-from-controller").await.unwrap();
    ctl.flush().await.unwrap();
    let mut buf = [0u8; 18];
    host.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hi-from-controller");
}

#[tokio::test]
async fn controller_for_offline_id_gets_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { elo_relay::serve(listener).await.unwrap() });

    let mut ctl = TcpStream::connect(addr).await.unwrap();
    write_msg(
        &mut ctl,
        &ControlMsg::Connect {
            target_id: "000000000".into(),
            pubkey: vec![1, 2, 3],
        },
    )
    .await;

    match decode(&read_frame(&mut ctl).await).unwrap() {
        ControlMsg::Error { reason } => assert!(reason.contains("not online")),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn directory_register_and_resolve() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { elo_relay::serve(listener).await.unwrap() });

    // Host publishes its short id -> node id and keeps the connection open.
    let mut host = TcpStream::connect(addr).await.unwrap();
    write_msg(
        &mut host,
        &ControlMsg::Register {
            device_id: "123456789".into(),
            node_id: "nodeabc123".into(),
        },
    )
    .await;
    assert_eq!(decode(&read_frame(&mut host).await).unwrap(), ControlMsg::Registered);

    // A controller resolves the short id.
    let mut ctl = TcpStream::connect(addr).await.unwrap();
    write_msg(&mut ctl, &ControlMsg::Resolve { device_id: "123456789".into() }).await;
    assert_eq!(
        decode(&read_frame(&mut ctl).await).unwrap(),
        ControlMsg::Resolved { node_id: "nodeabc123".into() }
    );

    // Unknown id resolves to an error.
    let mut ctl2 = TcpStream::connect(addr).await.unwrap();
    write_msg(&mut ctl2, &ControlMsg::Resolve { device_id: "000000000".into() }).await;
    match decode(&read_frame(&mut ctl2).await).unwrap() {
        ControlMsg::Error { reason } => assert!(reason.contains("not online")),
        other => panic!("expected Error, got {other:?}"),
    }

    // When the host drops, the mapping is gone.
    drop(host);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut ctl3 = TcpStream::connect(addr).await.unwrap();
    write_msg(&mut ctl3, &ControlMsg::Resolve { device_id: "123456789".into() }).await;
    match decode(&read_frame(&mut ctl3).await).unwrap() {
        ControlMsg::Error { .. } => {}
        other => panic!("expected Error after host left, got {other:?}"),
    }
}
