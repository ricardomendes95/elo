//! Instalação "um clique" no Windows: copia o Elo para `C:\Program Files\Elo`,
//! registra uma tarefa agendada que o inicia no logon (em segundo plano, na
//! bandeja), cria um atalho no Menu Iniciar, a entrada em "Aplicativos
//! instalados" e o serviço do Windows — tudo com uma única confirmação (UAC).
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

        // 0) Atualização/reinstalação: tira o serviço antigo e fecha o Elo que
        //    roda de Arquivos de Programas, para o exe poder ser substituído.
        if crate::service::is_installed() {
            let _ = crate::service::uninstall();
        }
        let ps = format!(
            "Get-CimInstance Win32_Process | Where-Object {{ $_.ProcessId -ne {pid} -and $_.ExecutablePath -eq '{exe}' }} | \
               ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
            pid = std::process::id(),
            exe = target.display().to_string().replace('\'', "''"),
        );
        run_quiet("powershell", &["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps]);

        // a) Cria a pasta e copia o exe atual para C:\Program Files\Elo\elo.exe.
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(error = %e, "não foi possível criar o diretório de instalação");
        }
        match std::env::current_exe() {
            Ok(src) => {
                // Se já estivermos rodando de dentro do destino, não copia sobre
                // si mesmo (arquivo em uso).
                if src != target {
                    // Processos recém-encerrados demoram a soltar o exe: tenta
                    // por até ~15 s antes de desistir.
                    let mut copied = false;
                    for _ in 0..30 {
                        match std::fs::copy(&src, &target) {
                            Ok(_) => {
                                copied = true;
                                break;
                            }
                            Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
                        }
                    }
                    if !copied {
                        tracing::warn!("não foi possível copiar o exe para Arquivos de Programas (arquivo em uso)");
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
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut([System.IO.Path]::Combine($env:ProgramData,'Microsoft\\Windows\\Start Menu\\Programs\\Elo.lnk')); \
             $s.TargetPath='{exe}'; $s.WorkingDirectory='{dir}'; $s.IconLocation='{exe},0'; $s.Description='Elo - acesso remoto'; $s.Save()",
            exe = target.display().to_string().replace('\'', "''"),
            dir = dir.display().to_string().replace('\'', "''"),
        );
        run_quiet("powershell", &["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps]);

        // d) Entrada em "Aplicativos instalados" do Windows, com desinstalação.
        let exe = target.display().to_string();
        let uninstall_cmd = format!("\"{exe}\" --uninstall-app");
        let size_kb = std::fs::metadata(&target).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
        for (name, kind, value) in [
            ("DisplayName", "REG_SZ", "Elo".to_string()),
            ("DisplayVersion", "REG_SZ", env!("CARGO_PKG_VERSION").to_string()),
            ("Publisher", "REG_SZ", "Elo".to_string()),
            ("DisplayIcon", "REG_SZ", exe.clone()),
            ("InstallLocation", "REG_SZ", dir.display().to_string()),
            ("UninstallString", "REG_SZ", uninstall_cmd.clone()),
            ("QuietUninstallString", "REG_SZ", uninstall_cmd.clone()),
            ("EstimatedSize", "REG_DWORD", size_kb.to_string()),
            ("NoModify", "REG_DWORD", "1".to_string()),
            ("NoRepair", "REG_DWORD", "1".to_string()),
        ] {
            run_quiet("reg", &["add", UNINSTALL_KEY, "/v", name, "/t", kind, "/d", &value, "/f"]);
        }

        // e) Serviço do Windows (roda a cópia de Arquivos de Programas): host
        //    antes do login, tela bloqueada e Ctrl+Alt+Del.
        if let Err(e) = crate::service::install() {
            tracing::warn!(error = %e, "não foi possível instalar o serviço");
        }
    }

    /// Chave de "Aplicativos instalados" do Windows.
    const UNINSTALL_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Elo";

    /// Remove o Elo por completo, elevado: serviço, processos, tarefa agendada,
    /// atalhos, regras de firewall, exceções do Defender, registro, pastas do
    /// programa, dos dados do serviço e da configuração do usuário.
    /// Cada passo é melhor esforço: uma falha não impede os seguintes.
    pub fn run_uninstall() {
        let dir = install_dir();
        let me = std::env::current_exe().ok();
        // Todos os executáveis do Elo que conhecemos: o instalado, o do serviço
        // (pode estar fora de Arquivos de Programas) e este.
        let mut exes: Vec<PathBuf> = vec![installed_exe()];
        exes.extend(crate::service::service_exe());
        exes.extend(me.clone());
        exes.sort();
        exes.dedup();

        // a) Serviço (para, espera soltar o exe, apaga) e política de Ctrl+Alt+Del.
        let _ = crate::service::uninstall();

        // b) Tarefa agendada de logon.
        run_quiet("schtasks", &["/Delete", "/TN", "Elo", "/F"]);

        // c) Processos que sobraram (janela, bandeja, worker de sessão), menos este;
        //    firewall; exceções do Defender; atalhos.
        let quoted = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
        let list = exes.iter().map(|p| quoted(p)).collect::<Vec<_>>().join(",");
        let ps = format!(
            "$exes=@({list}); \
             Get-CimInstance Win32_Process | Where-Object {{ $_.ProcessId -ne $PID -and $_.ProcessId -ne {me_pid} -and \
               ($exes -contains $_.ExecutablePath -or $_.CommandLine -like '*--session-worker*') }} | \
               ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}; \
             Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue | Where-Object {{ $exes -contains $_.Program }} | \
               Get-NetFirewallRule -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue; \
             Get-NetFirewallRule -DisplayName 'Elo - acesso remoto' -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue; \
             foreach ($e in $exes) {{ Remove-MpPreference -ExclusionPath $e -ErrorAction SilentlyContinue }}; \
             Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $env:ProgramData 'Microsoft\\Windows\\Start Menu\\Programs\\Elo.lnk'); \
             Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Elo.lnk')",
            me_pid = std::process::id(),
        );
        run_quiet("powershell", &["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps]);

        // d) Registro: "Aplicativos instalados" e a chave própria do Elo.
        run_quiet("reg", &["delete", UNINSTALL_KEY, "/f"]);
        run_quiet("reg", &["delete", crate::service::ELO_REG_KEY, "/f"]);

        // e) Dados: do serviço (identidade da máquina, logs, recebidos) e do
        //    usuário (configuração, favoritos, senha).
        for data in [crate::config::service_dir(), crate::config::config_dir()] {
            if let Err(e) = std::fs::remove_dir_all(&data) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(error = %e, dir = %data.display(), "não foi possível remover a pasta de dados");
                }
            }
        }

        // f) Programa. Se este processo roda de dentro da pasta, agenda a
        //    remoção para quando ele sair.
        let running_from_dir = me.as_ref().map(|c| c.starts_with(&dir)).unwrap_or(false);
        if running_from_dir {
            schedule_dir_removal(&dir);
        } else if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!(error = %e, "não foi possível remover o diretório de instalação");
                schedule_dir_removal(&dir);
            }
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
