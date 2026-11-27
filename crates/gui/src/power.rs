//! Estado de energia e da sessão do Windows usado pelo host:
//!
//! * [`spawn_keep_awake`]: mantém um *power request* do Windows enquanto o Elo
//!   aceita conexões, para o PC não suspender por inatividade (a tela ainda pode
//!   apagar, e a suspensão pedida pelo usuário continua valendo).
//! * [`session_locked`]: a sessão do console está bloqueada (tela de bloqueio)?
//!
//! Fora do Windows tudo vira no-op, só para o `cargo check` no Linux.

#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::Arc;
use std::time::Duration;

/// Motivo de recusa enviado ao controlador quando ninguém pode aprovar o pedido.
pub const NOBODY_TO_APPROVE: &str =
    "Computador bloqueado ou sem ninguém para aprovar. Use a senha fixa ou marque este dispositivo como confiável.";

/// Com que frequência o `keep_awake` do config é relido.
const KEEP_AWAKE_POLL: Duration = Duration::from_secs(30);

/// Inicia uma thread dedicada que liga/desliga o power request conforme
/// `wanted()` (relido a cada 30 s). Vive até o processo encerrar.
pub fn spawn_keep_awake(wanted: Arc<dyn Fn() -> bool + Send + Sync>) {
    let _ = std::thread::Builder::new().name("elo-keep-awake".into()).spawn(move || {
        let mut req = platform::Request::new();
        loop {
            let want = wanted();
            if want != req.active() {
                req.set(want);
            }
            std::thread::sleep(KEEP_AWAKE_POLL);
        }
    });
}

/// A sessão do console (ou a deste processo) está bloqueada?
pub fn session_locked() -> bool {
    platform::session_locked()
}

#[cfg(windows)]
mod platform {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Power::{
        PowerClearRequest, PowerCreateRequest, PowerRequestExecutionRequired, PowerRequestSystemRequired,
        PowerSetRequest,
    };
    use windows::Win32::System::RemoteDesktop::{
        ProcessIdToSessionId, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSQuerySessionInformationW,
        WTSSessionInfoEx, WTSINFOEXW, WTS_CURRENT_SERVER_HANDLE, WTS_SESSIONSTATE_LOCK,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
    };

    /// Power request do Windows ("Elo: aceitando conexões remotas").
    pub struct Request {
        handle: Option<HANDLE>,
        active: bool,
        /// Mantém viva a string apontada pelo REASON_CONTEXT.
        _reason: Vec<u16>,
    }

    impl Request {
        pub fn new() -> Self {
            let mut reason: Vec<u16> = "Elo: aceitando conexões remotas".encode_utf16().chain(std::iter::once(0)).collect();
            let ctx = REASON_CONTEXT {
                Version: 0, // POWER_REQUEST_CONTEXT_VERSION
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 { SimpleReasonString: PWSTR(reason.as_mut_ptr()) },
            };
            // SAFETY: `ctx` e a string apontada vivem durante a chamada; o
            // handle devolvido é nosso e é fechado no Drop.
            let handle = match unsafe { PowerCreateRequest(&ctx) } {
                Ok(h) => Some(h),
                Err(e) => {
                    tracing::warn!(error = %e, "PowerCreateRequest falhou; manter acordado indisponível");
                    None
                }
            };
            Request { handle, active: false, _reason: reason }
        }

        pub fn active(&self) -> bool {
            self.active
        }

        pub fn set(&mut self, on: bool) {
            let Some(h) = self.handle else {
                self.active = on;
                return;
            };
            // SAFETY: `h` é um power request válido que nós criamos.
            unsafe {
                if on {
                    if let Err(e) = PowerSetRequest(h, PowerRequestSystemRequired) {
                        tracing::warn!(error = %e, "não consegui impedir a suspensão por inatividade");
                        return;
                    }
                    // Opcional: em sistemas com Modern Standby mantém o processo rodando.
                    let _ = PowerSetRequest(h, PowerRequestExecutionRequired);
                    tracing::info!("manter acordado LIGADO: o Windows não suspende por inatividade enquanto o Elo aceita conexões");
                } else {
                    let _ = PowerClearRequest(h, PowerRequestSystemRequired);
                    let _ = PowerClearRequest(h, PowerRequestExecutionRequired);
                    tracing::info!("manter acordado DESLIGADO: suspensão por inatividade liberada");
                }
            }
            self.active = on;
        }
    }

    impl Drop for Request {
        fn drop(&mut self) {
            if let Some(h) = self.handle.take() {
                // SAFETY: fechamos o handle que criamos, uma única vez.
                unsafe {
                    let _ = CloseHandle(h);
                }
            }
        }
    }

    /// Sessão deste processo; na sessão 0 (serviço), a do console.
    fn target_session() -> u32 {
        let mut s = 0u32;
        // SAFETY: out-param simples.
        let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut s) }.is_ok();
        if !ok || s == 0 {
            // SAFETY: sem pré-condições.
            s = unsafe { WTSGetActiveConsoleSessionId() };
        }
        s
    }

    pub fn session_locked() -> bool {
        let session = target_session();
        if session == 0xFFFF_FFFF {
            return false;
        }
        let mut buf = PWSTR::null();
        let mut len = 0u32;
        // SAFETY: o Windows aloca `buf` (liberado com WTSFreeMemory); só lemos
        // o WTSINFOEXW se o tamanho devolvido comporta a estrutura.
        unsafe {
            if WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, session, WTSSessionInfoEx, &mut buf, &mut len).is_err()
                || buf.is_null()
            {
                return false;
            }
            let mut locked = false;
            if len as usize >= std::mem::size_of::<WTSINFOEXW>() {
                let info = &*(buf.0 as *const WTSINFOEXW);
                if info.Level == 1 {
                    // Windows 8+/10+: WTS_SESSIONSTATE_LOCK = 0 (no Windows 7 os
                    // valores LOCK/UNLOCK vêm trocados — bug conhecido do SO).
                    locked = info.Data.WTSInfoExLevel1.SessionFlags == WTS_SESSIONSTATE_LOCK as i32;
                }
            }
            WTSFreeMemory(buf.0 as *mut core::ffi::c_void);
            locked
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub struct Request {
        active: bool,
    }
    impl Request {
        pub fn new() -> Self {
            Request { active: false }
        }
        pub fn active(&self) -> bool {
            self.active
        }
        pub fn set(&mut self, on: bool) {
            tracing::debug!(on, "manter acordado (no-op fora do Windows)");
            self.active = on;
        }
    }
    pub fn session_locked() -> bool {
        false
    }
}

/// Espera a decisão de `fut`, mas desiste (recusando com [`NOBODY_TO_APPROVE`])
/// se a sessão for bloqueada no meio da espera.
pub async fn unless_locked(fut: impl std::future::Future<Output = bool>) -> bool {
    tokio::pin!(fut);
    loop {
        tokio::select! {
            r = &mut fut => return r,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                if session_locked() {
                    tracing::info!("sessão bloqueada durante o pedido de aprovação; recusando");
                    elo_client_core::host::set_deny_reason(NOBODY_TO_APPROVE);
                    return false;
                }
            }
        }
    }
}
