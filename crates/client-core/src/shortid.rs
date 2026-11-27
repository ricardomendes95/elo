//! ID curto estável via diretório HTTPS (Cloudflare Worker em `cloud/directory`).
//!
//! O aparelho se identifica por um hash secreto de um ID do sistema que
//! sobrevive a reinstalar o Elo (Windows: `MachineGuid`; Android: `ANDROID_ID`).
//! O servidor devolve sempre o mesmo número de 9 dígitos para esse hash e
//! guarda o endereço P2P (chave extensa) atual, que muda ao reinstalar.
//!
//! O campo "servidor de diretório" das configurações aceita os dois formatos:
//! `https://...` usa este módulo; `host:porta` usa o relay TCP antigo
//! ([`crate::rendezvous`]). Ver [`is_http`].

use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

/// Diretório público padrão (Worker do Elo).
pub const DEFAULT_DIRECTORY_URL: &str = "https://elo-directory.ricardomendes-dev.workers.dev";

/// Sal fixo do Elo: o hash não serve para correlacionar o aparelho em outros apps.
const DEVICE_SALT: &str = "elo-device-v1:";

/// `true` quando o diretório configurado é o serviço HTTPS (não o relay TCP).
pub fn is_http(directory: &str) -> bool {
    let d = directory.trim();
    d.starts_with("https://") || d.starts_with("http://")
}

/// Hash secreto do aparelho (64 hex) a partir do ID bruto do sistema.
pub fn device_hash(raw_system_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(DEVICE_SALT.as_bytes());
    h.update(raw_system_id.trim().to_ascii_lowercase().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `MachineGuid` do Windows (HKLM\SOFTWARE\Microsoft\Cryptography), na visão
/// de 64 bits. Muda só se o Windows for reinstalado.
#[cfg(windows)]
pub fn windows_machine_guid() -> Option<String> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY};
    let mut buf = [0u16; 128];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: buffer gravável do tamanho informado (em bytes); strings estáticas.
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Cryptography"),
            w!("MachineGuid"),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    (!s.trim().is_empty()).then_some(s)
}

fn client() -> Result<reqwest::Client> {
    // TLS com raízes embutidas (webpki-roots) e o provedor ring: não depende do
    // verificador da plataforma, que no Android exigiria inicialização via JNI.
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .context("TLS")?
        .with_root_certificates(roots)
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("elo/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("cliente HTTPS")
}

fn base(directory: &str) -> String {
    directory.trim().trim_end_matches('/').to_string()
}

fn field(body: &str, key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get(key)?.as_str().map(str::to_string)
}

/// Registra (ou atualiza) este aparelho e devolve o seu número curto, que é
/// sempre o mesmo para o mesmo `device_hash`.
pub async fn register(directory: &str, device_hash: &str, node_id: &str) -> Result<String> {
    let body = serde_json::json!({ "device": device_hash, "node": node_id }).to_string();
    let resp = client()?
        .post(format!("{}/v1/register", base(directory)))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .context("sem resposta do servidor de ID curto")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("servidor de ID curto recusou o registro ({status}): {}", field(&text, "error").unwrap_or(text));
    }
    let short = field(&text, "short").context("resposta sem número curto")?;
    if short.len() != 9 || !short.bytes().all(|b| b.is_ascii_digit()) {
        bail!("número curto inválido na resposta: {short}");
    }
    Ok(short)
}

/// Resolve um número curto (9 dígitos, espaços ignorados) para o endereço P2P.
pub async fn resolve(directory: &str, short: &str) -> Result<String> {
    let digits: String = short.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 9 {
        bail!("o ID curto tem 9 dígitos");
    }
    let resp = client()?
        .get(format!("{}/v1/resolve/{digits}", base(directory)))
        .send()
        .await
        .context("sem resposta do servidor de ID curto")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status.as_u16() == 404 {
        bail!("ID {digits} não encontrado (o aparelho nunca se registrou ou o número está errado)");
    }
    if !status.is_success() {
        bail!("servidor de ID curto respondeu {status}: {}", field(&text, "error").unwrap_or(text));
    }
    field(&text, "node").context("resposta sem endereço")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_salted() {
        let a = device_hash("ABC-123");
        assert_eq!(a, device_hash(" abc-123 "));
        assert_eq!(a.len(), 64);
        assert_ne!(a, device_hash("abc-124"));
    }

    /// Rede real: `cargo test -p elo-client-core shortid -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn https_works() {
        let r = client().unwrap().get("https://www.cloudflare.com/cdn-cgi/trace").send().await.unwrap();
        assert!(r.status().is_success());
    }

    #[test]
    fn detects_http_directory() {
        assert!(is_http("https://x.workers.dev"));
        assert!(!is_http("relay.exemplo.com:7000"));
    }
}
