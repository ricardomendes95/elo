//! Command-line entry point.
//!
//! Usage:
//!   elo-client host    --relay HOST:PORT [--id 123456789] --password PW
//!   elo-client connect --relay HOST:PORT  --id 123456789  --password PW
//!
//! The `host` role captures this machine's screen and injects remote input, so
//! it requires Windows. The `connect` (controller) role runs anywhere; this CLI
//! build is headless (prints frame stats and relays stdin as chat). A graphical
//! controller that renders frames and forwards real input is the next step.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use elo_client_core::controller::{run_controller, ControllerConfig, ControllerEvent};
use elo_protocol::format_id;

pub fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let opts = parse_opts(args);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    match mode.as_str() {
        "relay" => rt.block_on(run_relay_mode(opts)),
        "host" => rt.block_on(run_host_mode(opts)),
        "connect" => rt.block_on(run_controller_mode(opts)),
        _ => {
            eprintln!(
                "usage:\n  elo-client relay   [--bind 0.0.0.0:9000]\n  elo-client host    --relay HOST:PORT [--id 123456789] --password PW\n  elo-client host    --listen 0.0.0.0:9000 --password PW            (direct, no relay)\n  elo-client connect --relay HOST:PORT  --id 123456789  --password PW\n  elo-client connect --direct HOST:PORT --password PW               (direct, no relay)\n  elo-client host    --p2p --password PW                             (internet P2P; prints the address)\n  elo-client connect --p2p ADDRESS|ID [--directory HOST:PORT] [--password PW] [--code 2FA]"
            );
            Ok(())
        }
    }
}

/// Run the bundled relay server (same binary — no separate download).
async fn run_relay_mode(opts: HashMap<String, String>) -> Result<()> {
    let bind = opts.get("bind").cloned().unwrap_or_else(|| "0.0.0.0:9000".to_string());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("elo relay listening on {bind}");
    elo_relay::serve(listener).await
}

fn parse_opts(args: impl Iterator<Item = String>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut args = args.peekable();
    while let Some(a) = args.next() {
        if let Some(key) = a.strip_prefix("--") {
            let val = args.next().unwrap_or_default();
            map.insert(key.to_string(), val);
        }
    }
    map
}

fn require<'a>(opts: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    opts.get(key)
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required --{key}"))
}

fn download_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// A pseudo-random 9-digit device id (no crypto strength needed; it is only a
/// rendezvous handle, and the session is authenticated by key pinning + password).
#[cfg_attr(not(windows), allow(dead_code))]
fn random_device_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = (nanos % 1_000_000_000) as u64;
    format!("{n:09}")
}

#[cfg(windows)]
async fn run_host_mode(opts: HashMap<String, String>) -> Result<()> {
    use elo_client_core::host::{run_host, HostConfig};
    use elo_client_core::Capture; // brings width()/height() into scope
    use elo_protocol::Permissions;

    let password = require(&opts, "password")?.to_string();
    // --relay is required only in relay mode (not when --listen is given).
    let relay = opts.get("relay").cloned().unwrap_or_default();
    if relay.is_empty() && !opts.contains_key("listen") && !opts.contains_key("p2p") {
        anyhow::bail!("missing --relay (or use --listen HOST:PORT for direct mode)");
    }
    let device_id = opts
        .get("id")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(random_device_id);
    let keypair = elo_session::generate_keypair()?;

    // P2P mode (internet, no server of ours): print the address, serve forever.
    if opts.contains_key("p2p") {
        use elo_client_core::host::serve_host_on;
        let dir = download_dir().join(".elo-cli");
        let node = elo_p2p::bind_node(&dir.join("node.key")).await?;
        println!("elo host (P2P) ready. address: {}", node.id());
        loop {
            let duplex = node.accept().await?;
            let capture = elo_client_core::platform::DxgiCapture::new()?;
            let input = elo_client_core::platform::SendInputSink::new(capture.width(), capture.height());
            if let Err(e) = serve_host_on(duplex, &keypair, &password, Permissions::FULL, 30, download_dir(), capture, input).await {
                eprintln!("session ended: {e:#}");
            }
        }
    }

    let capture = elo_client_core::platform::DxgiCapture::new()?;
    let input = elo_client_core::platform::SendInputSink::new(capture.width(), capture.height());

    // Direct mode (no relay): listen on a port for a controller that can reach us.
    if let Some(bind) = opts.get("listen").filter(|s| !s.is_empty()).cloned() {
        use elo_client_core::host::{run_host_direct, DirectHostConfig};
        println!("elo host listening directly on {bind} (no relay).");
        return run_host_direct(
            DirectHostConfig {
                bind,
                keypair,
                password,
                permissions: Permissions::FULL,
                fps: 30,
                download_dir: download_dir(),
            },
            capture,
            input,
        )
        .await;
    }

    println!("elo host ready. device id: {}", format_id(&device_id));
    println!("share this id and the session password with the controller.");

    run_host(
        HostConfig {
            relay,
            device_id,
            keypair,
            password,
            permissions: Permissions::FULL,
            fps: 30,
            download_dir: download_dir(),
        },
        capture,
        input,
    )
    .await
}

#[cfg(not(windows))]
async fn run_host_mode(_opts: HashMap<String, String>) -> Result<()> {
    anyhow::bail!("the host role captures the screen and injects input, so it requires Windows; build and run `elo-client host` on the Windows machine");
}

async fn run_controller_mode(opts: HashMap<String, String>) -> Result<()> {
    // Empty password = ask the host for approval.
    let password = opts.get("password").cloned().unwrap_or_default();
    let keypair = elo_session::generate_keypair()?;
    let mut _node_keepalive = None;

    let (handle, mut events, label) = if let Some(target) = opts.get("p2p").filter(|s| !s.is_empty()) {
        // Internet P2P: by address, or by short id through a directory.
        use elo_client_core::controller::{connect_on_opts, ConnectOptions};
        let digits: String = target.chars().filter(|c| c.is_ascii_digit()).collect();
        let node_id = if digits.len() == 9 && target.len() <= 11 {
            let dir = require(&opts, "directory")?;
            elo_client_core::rendezvous::resolve_directory(dir, &digits).await?
        } else {
            target.clone()
        };
        let node = elo_p2p::bind_node_ephemeral().await?;
        let duplex = node.connect(&node_id).await?;
        let co = ConnectOptions { name: None, totp_code: opts.get("code").cloned(), ..Default::default() };
        let (h, e) = connect_on_opts(duplex, &keypair, &password, download_dir(), co).await?;
        _node_keepalive = Some(node);
        (h, e, target.clone())
    } else if let Some(host) = opts.get("direct").filter(|s| !s.is_empty()) {
        use elo_client_core::controller::{run_controller_direct, DirectControllerConfig};
        let (h, e) = run_controller_direct(DirectControllerConfig {
            host: host.clone(),
            keypair,
            password,
            download_dir: download_dir(),
        })
        .await?;
        (h, e, host.clone())
    } else {
        let relay = require(&opts, "relay")?.to_string();
        let target_id = require(&opts, "id")?.to_string();
        let (h, e) = run_controller(ControllerConfig {
            relay,
            target_id: target_id.clone(),
            keypair,
            password,
            download_dir: download_dir(),
        })
        .await?;
        (h, e, format_id(&target_id))
    };

    println!(
        "connected to {} — screen {}x{}, permissions: control={} files={}",
        label,
        handle.screen.0,
        handle.screen.1,
        handle.permissions.control,
        handle.permissions.file_transfer,
    );
    println!("(headless controller: type a line to chat; Ctrl-C to quit)");

    // Relay stdin lines as chat messages.
    let chat_handle = handle.clone();
    tokio::spawn(async move {
        let stdin = tokio::io::BufReader::new(tokio::io::stdin());
        use tokio::io::AsyncBufReadExt;
        let mut lines = stdin.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.is_empty() {
                let _ = chat_handle.send_chat(&line).await;
            }
        }
    });

    let mut frames = 0u64;
    while let Some(ev) = events.recv().await {
        match ev {
            ControllerEvent::ScreenChanged { width, height } => println!("[screen {width}x{height}]"),
            ControllerEvent::Monitors(list) => println!("[{} monitor(s)]", list.len()),
            ControllerEvent::Frame { .. } => {
                frames += 1;
                if frames % 30 == 0 {
                    println!("[{frames} frames received]");
                }
            }
            ControllerEvent::Chat(text) => println!("host: {text}"),
            ControllerEvent::FileReceived { path } => println!("received file: {}", path.display()),
            // Only with `ConnectOptions::raw_media` (not used by the CLI).
            ControllerEvent::FrameH264 { .. } | ControllerEvent::Audio { .. } | ControllerEvent::Clipboard(_) => {}
            ControllerEvent::Disconnected => {
                println!("session ended.");
                break;
            }
        }
    }
    Ok(())
}
