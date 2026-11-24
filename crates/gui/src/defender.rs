//! Ajuda, opcional e transparente, para o falso positivo do Windows Defender.
//!
//! O `elo.exe` não é assinado, então a heurística de aprendizado de máquina do
//! Defender (`Trojan:Win32/...!ml`) pode marcá-lo como falso positivo e até
//! removê-lo. Enquanto não há assinatura de código, o app oferece — só quando a
//! exceção ainda não existe — um atalho para adicioná-la, e também mostra o
//! comando para quem preferir fazer à mão.
//!
//! Nada aqui é silencioso: adicionar exceção ao Defender é uma operação oficial
//! da Microsoft e o Windows SEMPRE exige o consentimento de administrador (UAC);
//! o app não consegue contornar isso. Ler as exceções não precisa de admin.
//! Fora do Windows tudo vira no-op.

/// Caminho completo deste executável, como string (para a exclusão e para o
/// comando manual). `None` se não der para resolver.
pub fn exe_path() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.display().to_string())
}

/// Comando manual que o usuário pode rodar num PowerShell de administrador.
pub fn manual_command() -> String {
    let exe = exe_path().unwrap_or_else(|| "C:\\caminho\\elo.exe".into());
    format!("Add-MpPreference -ExclusionPath '{}'", exe)
}

use std::sync::{Arc, Mutex};

/// Verifica a exclusão numa thread (a chamada ao PowerShell leva ~1 s) e grava
/// o resultado, para não travar a abertura da janela. Fora do Windows marca
/// como "coberto", para o painel nunca aparecer.
pub fn spawn_check(cell: Arc<Mutex<Option<bool>>>) {
    std::thread::spawn(move || {
        let r = is_excluded().or(Some(true));
        *cell.lock().unwrap() = r;
    });
}

/// `Some(true)` se este exe já está coberto por uma exclusão do Defender,
/// `Some(false)` se não está, `None` se não deu para verificar (ou não-Windows).
#[cfg(windows)]
pub fn is_excluded() -> Option<bool> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = exe_path()?.to_lowercase();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-MpPreference).ExclusionPath",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Coberto se alguma exclusão for o próprio arquivo ou uma pasta que o contém.
    let covered = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).any(|entry| {
        let e = entry.to_lowercase();
        exe == e || exe.starts_with(&format!("{}\\", e.trim_end_matches('\\')))
    });
    Some(covered)
}

#[cfg(not(windows))]
pub fn is_excluded() -> Option<bool> {
    None
}

/// Dispara o PowerShell elevado (UAC) para adicionar a exclusão deste exe.
/// Retorna assim que a janela do UAC é solicitada; o resultado real depende de
/// o usuário confirmar. Erro só se nem foi possível iniciar a solicitação.
#[cfg(windows)]
pub fn request_exclusion() -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let exe = exe_path().ok_or("não foi possível resolver o caminho do executável")?;
    let verb = HSTRING::from("runas");
    let file = HSTRING::from("powershell.exe");
    // Aspas simples no caminho; o PowerShell as trata como literal.
    let params = HSTRING::from(format!(
        "-NoProfile -WindowStyle Hidden -Command \"Add-MpPreference -ExclusionPath '{}'\"",
        exe
    ));
    // SAFETY: buffers HSTRING próprios e terminados em nulo, vivos durante a
    // chamada. Ignoramos o handle de instância retornado.
    let rc = unsafe {
        ShellExecuteW(HWND::default(), &verb, &file, &params, PCWSTR::null(), SW_HIDE)
    };
    // ShellExecuteW: > 32 = sucesso ao iniciar. Cancelar o UAC devolve <= 32.
    if rc.0 as isize > 32 {
        Ok(())
    } else {
        Err("não foi possível solicitar a elevação (UAC cancelado?)".into())
    }
}

#[cfg(not(windows))]
pub fn request_exclusion() -> Result<(), String> {
    Err("disponível apenas no Windows".into())
}
