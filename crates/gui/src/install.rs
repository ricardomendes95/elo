//! Instalação "um clique" no Windows: copia o Elo para `C:\Program Files\Elo`,
//! registra uma tarefa agendada que o inicia no logon (em segundo plano, na
//! bandeja) e cria um atalho no Menu Iniciar.
//!
//! Nada aqui é silencioso ou escondido: a ação de instalar/desinstalar relança
//! o próprio Elo ELEVADO, e o Windows SEMPRE pede a confirmação de
//! administrador (UAC). O app não contorna o UAC nem mexe no antivírus.
//! Fora do Windows tudo vira no-op, para o `cargo check` do Linux passar.

/// Caminho exibido na interface do diretório de instalação.
pub const INSTALL_DIR_DISPLAY: &str = r"C:\Program Files\Elo";

/// Elo está instalado? Verdadeiro quando o `elo.exe` existe no destino.
#[cfg(windows)]
pub fn is_installed() -> bool {
    installed_exe().exists()
}

#[cfg(not(windows))]
pub fn is_installed() -> bool {
    false
}

/// Relança o Elo elevado (UAC) para instalar.
#[cfg(windows)]
pub fn elevate_install() -> anyhow::Result<()> {
    crate::service::elevate("--install-app")
}

/// Relança o Elo elevado (UAC) para desinstalar.
#[cfg(windows)]
pub fn elevate_uninstall() -> anyhow::Result<()> {
    crate::service::elevate("--uninstall-app")
}

#[cfg(not(windows))]
pub fn elevate_install() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn elevate_uninstall() -> anyhow::Result<()> {
    Ok(())
}

// ----------------------------------------------------------------------------
// Rotinas executadas no processo ELEVADO (`--install-app` / `--uninstall-app`).
// ----------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};

    /// Diretório de instalação: `%ProgramFiles%\Elo` (`C:\Program Files\Elo`).
    pub fn install_dir() -> PathBuf {
        let base = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        base.join("Elo")
    }

    /// Caminho do executável instalado.
    pub fn installed_exe() -> PathBuf {
        install_dir().join("elo.exe")
    }

    /// Faz toda a instalação, elevado. Cada passo trata o próprio erro e segue.
    pub fn run_install() {
        let dir = install_dir();
        let target = installed_exe();

        // a) Cria a pasta e copia o exe atual para C:\Program Files\Elo\elo.exe.
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(error = %e, "não foi possível criar o diretório de instalação");
        }
        match std::env::current_exe() {
            Ok(src) => {
                // Se já estivermos rodando de dentro do destino, não copia sobre
                // si mesmo (arquivo em uso).
                if src != target {
                    if let Err(e) = std::fs::copy(&src, &target) {
                        tracing::warn!(error = %e, "não foi possível copiar o exe para Arquivos de Programas");
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "não foi possível resolver o caminho do exe atual"),
        }

        // b) Tarefa agendada: inicia no logon, com privilégios máximos (admin),
        //    em segundo plano (--tray).
        let tr = format!("\"{}\" --tray", target.display());
        run_quiet(
            "schtasks",
            &["/Create", "/TN", "Elo", "/TR", &tr, "/SC", "ONLOGON", "/RL", "HIGHEST", "/F"],
        );

        // c) Atalho no Menu Iniciar (todos os usuários) via COM do PowerShell.
        let ps = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut([System.IO.Path]::Combine($env:ProgramData,'Microsoft\\Windows\\Start Menu\\Programs\\Elo.lnk')); $s.TargetPath='{}'; $s.Save()",
            target.display()
        );
        run_quiet("powershell", &["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps]);
    }

    /// Desfaz a instalação, elevado.
    pub fn run_uninstall() {
        // Remove a tarefa agendada.
        run_quiet("schtasks", &["/Delete", "/TN", "Elo", "/F"]);

        // Remove o atalho do Menu Iniciar.
        let ps = "Remove-Item -Force -ErrorAction SilentlyContinue ([System.IO.Path]::Combine($env:ProgramData,'Microsoft\\Windows\\Start Menu\\Programs\\Elo.lnk'))";
        run_quiet("powershell", &["-NoProfile", "-WindowStyle", "Hidden", "-Command", ps]);

        // Remove a pasta de instalação, com cuidado para não tentar apagar o
        // próprio exe em execução.
        let dir = install_dir();
        let running_from_dir = std::env::current_exe()
            .ok()
            .map(|c| c.starts_with(&dir))
            .unwrap_or(false);
        if running_from_dir {
            // O exe está em uso (este processo e a janela aberta). Agenda a
            // remoção: um cmd solto tenta apagar a pasta por ~60 s, até o Elo
            // aberto ser fechado e liberar o arquivo.
            schedule_dir_removal(&dir);
        } else if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!(error = %e, "não foi possível remover o diretório de instalação");
        }
    }

    /// Roda uma ferramenta de console sem piscar janela; melhor esforço.
    fn run_quiet(program: &str, args: &[&str]) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new(program)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }

    /// Dispara um processo solto que tenta apagar a pasta por ~60 s, para que
    /// ela suma assim que o Elo aberto (que trava o exe) for fechado.
    fn schedule_dir_removal(dir: &Path) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let d = dir.display().to_string();
        let script = format!(
            "for /l %i in (1,1,30) do (rmdir /s /q \"{d}\" 2>nul & if not exist \"{d}\" exit & ping -n 3 127.0.0.1 >nul)"
        );
        let _ = std::process::Command::new("cmd")
            .args(["/c", &script])
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn();
    }
}

#[cfg(windows)]
pub use imp::{installed_exe, run_install, run_uninstall};
