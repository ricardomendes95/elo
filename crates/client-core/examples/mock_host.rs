//! Host de teste com tela sintética (sem Windows): aceita conexões diretas em
//! `0.0.0.0:9000` com a senha `elo`, transmite um padrão animado (H.264 se o
//! controller pedir) e imprime cada evento de input/chat recebido. Serve para
//! testar controllers (ex.: o app Android no emulador, em `10.0.2.2`).
//!
//!   cargo run -p elo-client-core --example mock_host [-- PORTA [SENHA]]

use std::sync::Arc;

use anyhow::Result;
use elo_client_core::host::{serve_host_opts, HostEvent, HostOptions};
use elo_client_core::{InputSink, MockCapture};
use elo_protocol::{InputEvent, Permissions};
use tokio::net::TcpListener;

struct PrintInput;

impl InputSink for PrintInput {
    fn inject(&mut self, ev: InputEvent) -> Result<()> {
        match ev {
            InputEvent::Char { code } => println!("input: Char {:?} (U+{code:04X})", char::from_u32(code).unwrap_or('?')),
            other => println!("input: {other:?}"),
        }
        Ok(())
    }
    fn send_sas(&mut self) -> Result<()> {
        println!("input: Ctrl+Alt+Del (SendSas)");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(9000);
    let password = args.next().unwrap_or_else(|| "elo".into());
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    let keypair = elo_session::generate_keypair()?;
    println!("host de teste em 0.0.0.0:{port} (senha: {password}; sem senha = aprovação automática)");
    loop {
        let (stream, peer) = listener.accept().await?;
        println!("conexão de {peer}");
        let mut o = HostOptions::with_password(&password, Permissions::FULL, 30, std::env::temp_dir());
        o.approver = Some(Arc::new(|p| {
            println!("pedido de aprovação de \"{}\": aprovado automaticamente", p.name);
            Box::pin(async { true })
        }));
        o.on_event = Some(Arc::new(|ev| match ev {
            HostEvent::Chat { text, .. } => println!("chat: {text}"),
            HostEvent::Connected { peer, .. } => println!("conectado: {}", peer.name),
            HostEvent::Disconnected { peer } => println!("desconectado: {}", peer.name),
            HostEvent::AuthFailed { peer, reason } => println!("falha de autenticação de {}: {reason}", peer.name),
            HostEvent::Pointer { x, y, .. } => println!("ponteiro: {x},{y}"),
            HostEvent::FileReceived { path } => println!("arquivo recebido: {}", path.display()),
        }));
        let kp = keypair.clone();
        tokio::spawn(async move {
            let r = serve_host_opts(stream, &kp, o, || Ok((MockCapture::new(1280, 720), PrintInput))).await;
            println!("sessão encerrada: {r:?}");
        });
    }
}
