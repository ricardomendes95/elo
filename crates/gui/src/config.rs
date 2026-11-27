//! Persistent configuration: access rules (password, open access, approval
//! requests, 2FA, paired devices), session permissions, appearance, startup and
//! saved connections. Lives in a JSON file under the OS config dir, alongside
//! the persistent node/host keys. The Windows service uses its own copy in
//! `C:\ProgramData\elo`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A saved connection the user can one-click (address + stored password).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Favorite {
    pub name: String,
    pub address: String,
    /// true = LAN (IP), false = Internet (P2P node id / short id).
    pub lan: bool,
    pub password: String,
    /// Último node id (endereço P2P completo) visto para este ID curto; serve
    /// para avisar quando a identidade do aparelho muda.
    #[serde(default)]
    pub last_node: String,
}

/// A recently accessed computer (controller side), kept for one-click
/// reconnection. Like a favorite but managed automatically on each connect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Recent {
    pub name: String,
    pub address: String,
    /// true = LAN (IP), false = Internet (P2P node id / short id).
    pub lan: bool,
    /// Saved password (may be empty — then the user types it on reconnect).
    pub password: String,
    /// Último node id visto para este ID curto (aviso de identidade).
    #[serde(default)]
    pub last_node: String,
}

/// A paired (trusted) device: connects without password or approval.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TrustedDevice {
    pub name: String,
    /// Hex of the device's static key (see `PeerInfo::fingerprint`).
    pub fingerprint: String,
    /// When it was paired (YYYY-MM-DD).
    #[serde(default)]
    pub added: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // ---- access ----
    /// Fixed connect password: anyone with it connects without approval.
    /// Empty = a random per-launch session password is used instead.
    #[serde(default)]
    pub unattended_password: String,
    /// The session password rotates after each successful connection.
    #[serde(default)]
    pub otp: bool,
    /// "Unattended access": no password and no approval (anyone who reaches the
    /// device connects). Dangerous; off by default.
    #[serde(default)]
    pub open_access: bool,
    /// Controllers without a password may ask to come in (Allow/Deny prompt).
    #[serde(default = "yes")]
    pub allow_requests: bool,
    /// Two-factor authentication on for password / open / paired logins.
    #[serde(default)]
    pub twofa: bool,
    /// TOTP secret (base32) used when `twofa` is on.
    #[serde(default)]
    pub totp_secret: String,
    /// Paired devices.
    #[serde(default)]
    pub trusted: Vec<TrustedDevice>,

    // ---- control (what the operator may do) ----
    #[serde(default = "yes")]
    pub allow_control: bool,
    #[serde(default = "yes")]
    pub allow_clipboard: bool,
    #[serde(default = "yes")]
    pub allow_audio: bool,
    #[serde(default = "yes")]
    pub allow_files: bool,
    /// Show the operator's pointer and name on this screen (view-only sessions).
    #[serde(default = "yes")]
    pub show_pointer: bool,

    // ---- general ----
    #[serde(default)]
    pub autostart: bool,
    /// Fechar a janela esconde o app na bandeja (mantém a conexão/host vivos)
    /// em vez de encerrar. Ligado por padrão, no estilo do AnyDesk.
    #[serde(default = "yes")]
    pub close_to_tray: bool,
    /// Impede a suspensão por inatividade enquanto o Elo aceita conexões
    /// (power request do Windows). A tela ainda pode apagar.
    #[serde(default = "yes")]
    pub keep_awake: bool,
    /// UI theme: "dark" or "light".
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Servidor de ID curto: `https://...` (diretório HTTPS, padrão) ou
    /// `host:porta` (relay TCP antigo). Vazio = diretório padrão.
    #[serde(default = "default_directory")]
    pub directory: String,
    /// Último ID curto estável recebido do diretório HTTPS (mostrado enquanto
    /// o registro não responde ou sem internet).
    #[serde(default)]
    pub short_id_cache: String,
    /// This machine's stable short numeric id (9 digits), used with a directory.
    #[serde(default)]
    pub short_id: String,
    /// Display name sent to hosts (empty = OS user name).
    #[serde(default)]
    pub display_name: String,
    /// Saved connections.
    #[serde(default)]
    pub favorites: Vec<Favorite>,
    /// Recently accessed computers (newest first), maintained on each connect.
    #[serde(default)]
    pub recents: Vec<Recent>,
    /// O aviso do Windows Defender já foi atendido (exceção adicionada) ou
    /// dispensado pelo usuário; quando true, não aparece mais.
    #[serde(default)]
    pub defender_hidden: bool,
}

fn default_theme() -> String {
    "dark".to_string()
}

fn default_directory() -> String {
    elo_client_core::shortid::DEFAULT_DIRECTORY_URL.to_string()
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        // Same values as deserializing an empty object (serde defaults).
        serde_json::from_str("{}").expect("config defaults")
    }
}

/// The current OS user's login name.
pub fn current_username() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "usuário".into())
}

/// This computer's name.
pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()))
        .unwrap_or_else(|_| "este computador".into())
}

/// Directory holding Elo's persistent state (keys + config).
pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("elo")
}

/// Machine-wide state dir used by the Windows service (LocalSystem does not
/// share the user's `%APPDATA%`). `C:\ProgramData\elo`.
pub fn service_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("elo")
}

#[cfg_attr(not(windows), allow(dead_code))]
/// Copy the user's identity + config into the service dir so the installed
/// service keeps the same address, fingerprint and access rules.
pub fn seed_service_dir() {
    let src = config_dir();
    let dst = service_dir();
    let _ = std::fs::create_dir_all(&dst);
    for name in ["node.key", "host.key", "config.json"] {
        if let Ok(bytes) = std::fs::read(src.join(name)) {
            let _ = std::fs::write(dst.join(name), bytes);
        }
    }
}

impl Config {
    pub fn load() -> Config {
        Self::load_from(&config_dir())
    }

    pub fn load_from(dir: &Path) -> Config {
        std::fs::read(dir.join("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        self.save_to(&config_dir());
    }

    pub fn save_to(&self, dir: &Path) {
        let _ = self.try_save_to(dir);
    }

    /// Como `save_to`, mas devolve o motivo da falha para a UI mostrar.
    pub fn try_save_to(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("config.json"), json).map_err(|e| e.to_string())
    }

    /// Diretório efetivo: o configurado, ou o padrão quando vazio.
    pub fn directory_url(&self) -> String {
        let d = self.directory.trim();
        if d.is_empty() { default_directory() } else { d.to_string() }
    }

    pub fn is_trusted(&self, fingerprint: &str) -> bool {
        self.trusted.iter().any(|t| t.fingerprint.eq_ignore_ascii_case(fingerprint))
    }
}

/// Run a console tool without flashing a window (Windows).
#[cfg(windows)]
pub fn quiet(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}

/// Enable or disable launch-at-login. Windows: a HKCU `...\Run` value pointing
/// at the current exe. No-op (returns Ok) on other platforms.
pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::process::Command;
        let exe = std::env::current_exe()?;
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let status = if enabled {
            quiet(Command::new("reg").args(["add", key, "/v", "Elo", "/t", "REG_SZ", "/d", &format!("\"{}\"", exe.display()), "/f"]))
                .status()?
        } else {
            quiet(Command::new("reg").args(["delete", key, "/v", "Elo", "/f"])).status()?
        };
        if !status.success() && enabled {
            return Err(std::io::Error::other("reg add failed"));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_and_sensible() {
        let c = Config::default();
        assert!(!c.open_access);
        assert!(c.allow_requests && c.allow_control && c.show_pointer && c.keep_awake);
        assert_eq!(c.theme, "dark");
    }

    #[test]
    fn old_files_still_load() {
        let c: Config = serde_json::from_str(r#"{"unattended_password":"x","approval_required":true,"trusted":[]}"#).unwrap();
        assert_eq!(c.unattended_password, "x");
        assert!(c.allow_requests);
    }
}
