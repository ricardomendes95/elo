//! Full-stack end-to-end tests: a real relay on an ephemeral port, two clients
//! doing the Noise handshake, password auth, key pinning, codec, input, chat
//! and file transfer — all on Linux via mock capture/input.

use std::time::Duration;

use elo_protocol::{InputEvent, MouseButton, Permissions};
use tokio::net::TcpListener;
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use crate::controller::{run_controller, ControllerConfig, ControllerEvent};
use crate::host::{run_host, HostConfig};
use crate::{MockCapture, RecordingInput};

async fn start_relay() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move { elo_relay::serve(listener).await.unwrap() });
    addr
}

/// Wait for the next event of a given shape, with a timeout.
async fn next_event(rx: &mut Receiver<ControllerEvent>) -> ControllerEvent {
    timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for a controller event")
        .expect("event channel closed")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_session_input_chat_and_file() {
    let relay = start_relay().await;
    let host_kp = elo_session::generate_keypair().unwrap();
    let ctl_kp = elo_session::generate_keypair().unwrap();
    let device_id = "123456789".to_string();
    let password = "hunter2".to_string();

    let tmp = std::env::temp_dir().join(format!("elo-e2e-{}", std::process::id()));
    tokio::fs::create_dir_all(&tmp).await.unwrap();

    // Host with a synthetic screen and an input recorder we can inspect.
    let recorder = RecordingInput::default();
    let injected = recorder.events.clone();
    let host_cfg = HostConfig {
        relay: relay.clone(),
        device_id: device_id.clone(),
        keypair: host_kp,
        password: password.clone(),
        permissions: Permissions::FULL,
        fps: 30,
        download_dir: tmp.clone(),
    };
    let host = tokio::spawn(run_host(host_cfg, MockCapture::new(320, 200), recorder));

    // Give the host a moment to register before the controller dials in.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (handle, mut events) = run_controller(ControllerConfig {
        relay: relay.clone(),
        target_id: device_id.clone(),
        keypair: ctl_kp,
        password: password.clone(),
        download_dir: tmp.clone(),
    })
    .await
    .expect("controller failed to connect");

    assert_eq!(handle.screen, (320, 200));
    assert_eq!(handle.permissions, Permissions::FULL);

    // We should receive a decoded frame of the right dimensions.
    let mut got_frame = false;
    for _ in 0..10 {
        if let ControllerEvent::Frame { width, height, bgra } = next_event(&mut events).await {
            assert_eq!((width, height), (320, 200));
            assert_eq!(bgra.len(), 320 * 200 * 4);
            got_frame = true;
            break;
        }
    }
    assert!(got_frame, "never received a video frame");

    // Drive input; it must reach the host's injector.
    handle.send_input(InputEvent::MouseMove { x: 42, y: 24 }).await.unwrap();
    handle
        .send_input(InputEvent::MouseButton { button: MouseButton::Left, down: true })
        .await
        .unwrap();
    handle.send_input(InputEvent::Key { code: 0x41, down: true }).await.unwrap();

    // Send a chat and a file.
    handle.send_chat("olá host").await.unwrap();
    let payload: Vec<u8> = (0..50_000u32).map(|i| i as u8).collect();
    handle.send_file(1, "evidence.bin", &payload).await.unwrap();

    // Poll until the injector has recorded all three input events.
    let mut ok = false;
    for _ in 0..50 {
        if injected.lock().unwrap().len() >= 3 {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "host did not receive the injected input");
    let evs = injected.lock().unwrap().clone();
    assert_eq!(evs[0], InputEvent::MouseMove { x: 42, y: 24 });
    assert_eq!(evs[1], InputEvent::MouseButton { button: MouseButton::Left, down: true });
    assert_eq!(evs[2], InputEvent::Key { code: 0x41, down: true });

    // The host should have written the received file.
    let received = tmp.join("evidence.bin");
    let mut file_ok = false;
    for _ in 0..50 {
        if let Ok(data) = tokio::fs::read(&received).await {
            assert_eq!(data, payload);
            file_ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(file_ok, "host never wrote the transferred file");

    host.abort();
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_password_is_rejected() {
    let relay = start_relay().await;
    let host_kp = elo_session::generate_keypair().unwrap();
    let ctl_kp = elo_session::generate_keypair().unwrap();
    let device_id = "222333444".to_string();
    let tmp = std::env::temp_dir();

    let host = tokio::spawn(run_host(
        HostConfig {
            relay: relay.clone(),
            device_id: device_id.clone(),
            keypair: host_kp,
            password: "correct".into(),
            permissions: Permissions::FULL,
            fps: 15,
            download_dir: tmp.clone(),
        },
        MockCapture::new(64, 64),
        RecordingInput::default(),
    ));

    tokio::time::sleep(Duration::from_millis(100)).await;

    let res = run_controller(ControllerConfig {
        relay,
        target_id: device_id,
        keypair: ctl_kp,
        password: "wrong".into(),
        download_dir: tmp,
    })
    .await;

    assert!(res.is_err(), "controller with wrong password should be rejected");
    host.abort();
}


// ---------------------------------------------------------------------------
// Access rules (protocol v2): approval, paired devices, 2FA, open access, kick.
// ---------------------------------------------------------------------------

mod access {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use elo_protocol::Permissions;
    use elo_session::Keypair;
    use tokio::net::{TcpListener, TcpStream};

    use crate::controller::{connect_on_opts, ConnectOptions, ControllerEvent, ERR_2FA_REQUIRED};
    use crate::host::{serve_host_opts, HostEvent, HostOptions, HostSessionHandle};
    use crate::{totp, MockCapture, RecordingInput};

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn opts() -> HostOptions {
        HostOptions::with_password("", Permissions::FULL, 15, std::env::temp_dir())
    }

    async fn spawn_host(opts: HostOptions) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let kp = elo_session::generate_keypair().unwrap();
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let _ = serve_host_opts(s, &kp, opts, || Ok((MockCapture::new(64, 48), RecordingInput::default()))).await;
        });
        addr
    }

    async fn connect(
        addr: &str,
        kp: &Keypair,
        password: &str,
        code: Option<String>,
    ) -> anyhow::Result<(crate::controller::ControllerHandle, tokio::sync::mpsc::Receiver<ControllerEvent>)> {
        let s = TcpStream::connect(addr).await?;
        let o = ConnectOptions { name: Some("Teste".into()), totp_code: code, ..Default::default() };
        tokio::time::timeout(Duration::from_secs(10), connect_on_opts(s, kp, password, std::env::temp_dir(), o)).await?
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approval_allows() {
        let mut o = opts();
        o.approver = Some(Arc::new(|peer| Box::pin(async move { peer.name == "Teste" })));
        let addr = spawn_host(o).await;
        let kp = elo_session::generate_keypair().unwrap();
        assert!(connect(&addr, &kp, "", None).await.is_ok(), "approved controller should connect");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approval_denies() {
        let mut o = opts();
        o.approver = Some(Arc::new(|_| Box::pin(async { false })));
        let addr = spawn_host(o).await;
        let kp = elo_session::generate_keypair().unwrap();
        assert!(connect(&addr, &kp, "", None).await.is_err(), "denied controller must not connect");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_password_and_no_approver_is_rejected() {
        let mut o = opts();
        o.password = "secret".into();
        let addr = spawn_host(o).await;
        let kp = elo_session::generate_keypair().unwrap();
        assert!(connect(&addr, &kp, "", None).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn paired_device_needs_no_password() {
        let kp = elo_session::generate_keypair().unwrap();
        let mut o = opts();
        o.password = "secret".into();
        o.trusted = vec![hex(&kp.public)];
        let addr = spawn_host(o).await;
        assert!(connect(&addr, &kp, "", None).await.is_ok(), "paired device should connect without password");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_access_lets_anyone_in() {
        let mut o = opts();
        o.open_access = true;
        let addr = spawn_host(o).await;
        let kp = elo_session::generate_keypair().unwrap();
        assert!(connect(&addr, &kp, "", None).await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn twofa_requires_a_valid_code() {
        let secret = totp::generate_secret();
        let kp = elo_session::generate_keypair().unwrap();

        // Password only → the controller is told a code is needed.
        let mut o = opts();
        o.password = "pw".into();
        o.totp_secret = Some(secret.clone());
        let addr = spawn_host(o.clone()).await;
        let err = connect(&addr, &kp, "pw", None).await.err().expect("must ask for 2FA").to_string();
        assert!(err.contains(ERR_2FA_REQUIRED), "got: {err}");

        // Password + correct code → in.
        let addr = spawn_host(o).await;
        let code = totp::code_at(&secret, totp::now());
        assert!(connect(&addr, &kp, "pw", Some(code)).await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_can_kick_the_operator() {
        let slot: Arc<Mutex<Option<HostSessionHandle>>> = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        let mut o = opts();
        o.open_access = true;
        o.on_event = Some(Arc::new(move |ev| {
            if let HostEvent::Connected { session, .. } = ev {
                *s2.lock().unwrap() = Some(session);
            }
        }));
        let addr = spawn_host(o).await;
        let kp = elo_session::generate_keypair().unwrap();
        let (_h, mut events) = connect(&addr, &kp, "", None).await.unwrap();
        for _ in 0..50 {
            if slot.lock().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        slot.lock().unwrap().as_ref().expect("connected event").disconnect();
        let mut gone = false;
        for _ in 0..200 {
            match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
                Ok(Some(ControllerEvent::Disconnected)) | Ok(None) => {
                    gone = true;
                    break;
                }
                Ok(Some(_)) => continue,
                Err(_) => break,
            }
        }
        assert!(gone, "operator should be disconnected after a kick");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn raw_media_hands_over_h264_and_close_disconnects() {
        let mut o = opts();
        o.open_access = true;
        let addr = spawn_host(o).await;
        let kp = elo_session::generate_keypair().unwrap();
        let s = TcpStream::connect(&addr).await.unwrap();
        let co = ConnectOptions { name: Some("Teste".into()), raw_media: true, ..Default::default() };
        let (h, mut events) = connect_on_opts(s, &kp, "", std::env::temp_dir(), co).await.unwrap();

        // The host encodes H.264 (this build has OpenH264): the controller must
        // pass the Annex-B access unit through untouched, starting with an IDR.
        let mut got = false;
        for _ in 0..20 {
            match tokio::time::timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap() {
                ControllerEvent::FrameH264 { width, height, key, data } => {
                    assert_eq!((width, height), (64, 48));
                    assert!(key, "first access unit must be a keyframe");
                    assert!(data.starts_with(&[0, 0, 0, 1]) || data.starts_with(&[0, 0, 1]), "Annex-B start code");
                    got = true;
                    break;
                }
                ControllerEvent::Frame { .. } => panic!("raw mode must not decode video"),
                _ => continue,
            }
        }
        assert!(got, "expected a raw H.264 frame");

        h.close();
        let mut gone = false;
        for _ in 0..200 {
            match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
                Ok(Some(ControllerEvent::Disconnected)) | Ok(None) => {
                    gone = true;
                    break;
                }
                Ok(Some(_)) => continue,
                Err(_) => break,
            }
        }
        assert!(gone, "close() must end the session");
    }
}
