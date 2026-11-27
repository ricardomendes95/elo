//! Ícone na bandeja do sistema (área de notificação), no estilo do AnyDesk.
//!
//! No Windows, roda numa thread dedicada com uma janela "message-only" e um laço
//! `GetMessageW`. O ícone é adicionado via `Shell_NotifyIconW` e responde a
//! clique (abrir) e menu de contexto (Abrir Elo / Sair). Os eventos chegam ao
//! resto do app por um canal `mpsc`.
//!
//! Em sistemas não-Windows (apenas para `cargo check` no Linux), tudo vira um
//! conjunto de stubs no-op: `start` devolve um canal que nunca recebe nada.

/// O que o usuário pediu pela bandeja.
pub enum TrayEvent {
    /// Mostrar/levar a janela principal para frente.
    Open,
    /// Encerrar o aplicativo de verdade.
    Quit,
}

pub use platform::{alert, main_window_visible, post_quit, start, Tray};

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod platform {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::Arc;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
        NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, BringWindowToTop, CreatePopupMenu, FindWindowW, FlashWindowEx, GetForegroundWindow,
        IsWindowVisible, SetWindowPos, FLASHWINFO, FLASHW_ALL, FLASHW_TIMERNOFG, HWND_NOTOPMOST, HWND_TOPMOST,
        SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, ShowWindow, TrackPopupMenuEx, TranslateMessage, HICON,
        HWND_MESSAGE, IDI_APPLICATION, MF_STRING, MSG, SW_RESTORE, SW_SHOW, TPM_LEFTALIGN,
        TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
        WM_RBUTTONUP, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
    };

    use super::TrayEvent;

    /// Mensagem de callback do ícone (os cliques chegam aqui).
    const WM_TRAY: u32 = WM_APP + 1;
    /// Mensagem que o `Drop` posta para encerrar a janela/thread com limpeza.
    const WM_TRAY_QUIT: u32 = WM_APP + 2;
    /// Mensagem de alerta (balão + janela à frente); o lParam é um
    /// `Box<(String, String)>` (título, texto) que a wndproc retoma.
    const WM_TRAY_ALERT: u32 = WM_APP + 3;
    /// Clique no balão de notificação (NIN_BALLOONUSERCLICK = WM_USER + 5).
    const NIN_BALLOONUSERCLICK: u32 = 0x0405;
    /// Id do ícone dentro da janela (único por janela basta).
    const TRAY_UID: u32 = 1;
    /// Ids dos itens do menu de contexto.
    const ID_OPEN: usize = 1;
    const ID_QUIT: usize = 2;

    thread_local! {
        /// Canal de saída, acessado pela `wndproc` na própria thread da bandeja.
        static SENDER: RefCell<Option<Sender<TrayEvent>>> = const { RefCell::new(None) };
        /// Contexto do egui, para acordar o `update()` ao emitir um evento
        /// (a janela pode estar escondida na bandeja, com o loop ocioso).
        static WAKE: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
        /// `HWND` da janela principal (0 enquanto não descoberto). Compartilhado
        /// com a thread da UI, que o preenche; a bandeja controla a janela nativa
        /// diretamente por ele, sem depender do loop/viewport do eframe.
        static MAIN_HWND: RefCell<Option<Arc<AtomicIsize>>> = const { RefCell::new(None) };
        /// Flag de "sair de verdade", compartilhado com o app: a bandeja o liga
        /// antes de mandar o `WM_CLOSE`, e o `handle_close` o lê para deixar fechar.
        static REALLY_QUIT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    }

    /// Lê o `HWND` da janela principal compartilhado (0 se ainda desconhecido).
    fn main_hwnd() -> HWND {
        MAIN_HWND.with(|h| {
            h.borrow()
                .as_ref()
                .map(|a| HWND(a.load(Ordering::SeqCst) as *mut _))
                .unwrap_or_default()
        })
    }

    /// Traz a janela principal à frente via Win32 direto (funciona mesmo com a
    /// janela escondida na bandeja, quando o viewport do eframe não responde).
    unsafe fn show_main_window() {
        let h = main_hwnd();
        if !h.0.is_null() {
            let _ = ShowWindow(h, SW_SHOW);
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
    }

    /// `HWND` da janela principal: o compartilhado ou, se ainda não for
    /// conhecido, o da janela de título "Elo".
    unsafe fn main_or_find(h: isize) -> HWND {
        if h != 0 {
            return HWND(h as *mut _);
        }
        FindWindowW(None, windows::core::w!("Elo")).unwrap_or_default()
    }

    /// Mostra a janela e tenta trazê-la à frente mesmo vindo de segundo plano:
    /// o Windows pode negar o foco (regra de foreground), então ela também sobe
    /// acima das outras (topmost liga/desliga) e pisca na barra de tarefas.
    unsafe fn force_front(h: HWND) {
        if h.0.is_null() {
            return;
        }
        let _ = ShowWindow(h, SW_SHOW);
        let _ = ShowWindow(h, SW_RESTORE);
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
        let _ = SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, flags);
        let _ = SetWindowPos(h, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
        if GetForegroundWindow() != h {
            let fi = FLASHWINFO {
                cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                hwnd: h,
                dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
                uCount: 0,
                dwTimeout: 0,
            };
            let _ = FlashWindowEx(&fi);
        }
    }

    /// Balão de notificação no ícone da bandeja.
    unsafe fn balloon(hwnd: HWND, title: &str, text: &str) {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        let t: Vec<u16> = title.encode_utf16().take(63).collect();
        // A struct é "packed" no x86 (32 bits): monta o array fora e atribui inteiro.
        let mut title_buf = [0u16; 64];
        title_buf[..t.len()].copy_from_slice(&t);
        nid.szInfoTitle = title_buf;
        let b: Vec<u16> = text.encode_utf16().take(255).collect();
        let mut info_buf = [0u16; 256];
        info_buf[..b.len()].copy_from_slice(&b);
        nid.szInfo = info_buf;
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }

    /// Alerta de qualquer thread: com bandeja, posta para a thread dela (balão
    /// + janela à frente); sem bandeja, só traz a janela à frente daqui mesmo.
    pub fn alert(tray_hwnd: isize, main_hwnd: isize, title: &str, text: &str) {
        unsafe {
            if tray_hwnd != 0 {
                let payload = Box::into_raw(Box::new((title.to_string(), text.to_string())));
                let posted =
                    PostMessageW(HWND(tray_hwnd as *mut _), WM_TRAY_ALERT, WPARAM(0), LPARAM(payload as isize));
                if posted.is_ok() {
                    return;
                }
                // Não foi postado: a wndproc nunca vai liberar o Box.
                drop(Box::from_raw(payload));
            }
            force_front(main_or_find(main_hwnd));
        }
    }

    /// A janela principal está visível (não escondida na bandeja)?
    pub fn main_window_visible(main_hwnd: isize) -> bool {
        unsafe {
            let h = main_or_find(main_hwnd);
            !h.0.is_null() && IsWindowVisible(h).as_bool()
        }
    }

    /// Pede à bandeja (de qualquer thread) para encerrar o app como o "Sair".
    pub fn post_quit(tray_hwnd: isize) -> bool {
        tray_hwnd != 0
            && unsafe { PostMessageW(HWND(tray_hwnd as *mut _), WM_COMMAND, WPARAM(ID_QUIT), LPARAM(0)) }.is_ok()
    }

    /// Liga o flag de "sair" e manda `WM_CLOSE` à janela principal via Win32
    /// direto; o `handle_close` vê `really_quit == true` e deixa o app encerrar.
    unsafe fn quit_main_window() {
        REALLY_QUIT.with(|q| {
            if let Some(f) = q.borrow().as_ref() {
                f.store(true, Ordering::SeqCst);
            }
        });
        let h = main_hwnd();
        if !h.0.is_null() {
            let _ = ShowWindow(h, SW_SHOW);
            let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    fn emit(ev: TrayEvent) {
        SENDER.with(|s| {
            if let Some(tx) = s.borrow().as_ref() {
                let _ = tx.send(ev);
            }
        });
        // Força um frame do egui para que `drain_tray` leia o evento mesmo com
        // a janela escondida (senão "Sair"/"Abrir" não teriam efeito).
        WAKE.with(|w| {
            if let Some(ctx) = w.borrow().as_ref() {
                ctx.request_repaint();
            }
        });
    }

    trait EncodeNul {
        /// UTF-16 terminado em nulo, para os `PCWSTR` do Win32.
        fn encodeutf16_nul(&self) -> Vec<u16>;
    }
    impl EncodeNul for str {
        fn encodeutf16_nul(&self) -> Vec<u16> {
            self.encode_utf16().chain(std::iter::once(0)).collect()
        }
    }

    /// Ícone de bandeja, opaco. Remove o ícone e encerra a thread no `Drop`.
    pub struct Tray {
        /// `HWND` da janela message-only, guardado como inteiro para poder
        /// viajar entre threads com segurança (o ponteiro em si não é `Send`).
        hwnd: isize,
    }

    // O `isize` é só um identificador de janela; postar mensagem é thread-safe.
    unsafe impl Send for Tray {}

    impl Tray {
        /// `HWND` da janela da bandeja, para [`alert`]/[`post_quit`] de outras threads.
        pub fn hwnd(&self) -> isize {
            self.hwnd
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            unsafe {
                let hwnd = HWND(self.hwnd as *mut _);
                // Pede à thread da bandeja para destruir a janela: o laço então
                // remove o ícone (NIM_DELETE) e termina.
                let _ = PostMessageW(hwnd, WM_TRAY_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// Procedimento de janela: trata cliques no ícone e o menu de contexto.
    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY => {
                // O evento de mouse vem na parte baixa do lParam.
                match (lparam.0 as u32) & 0xFFFF {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_BALLOONUSERCLICK => {
                        show_main_window();
                        emit(TrayEvent::Open);
                    }
                    WM_RBUTTONUP => show_menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                match (wparam.0 & 0xFFFF) as usize {
                    ID_OPEN => {
                        show_main_window();
                        emit(TrayEvent::Open);
                    }
                    ID_QUIT => {
                        quit_main_window();
                        emit(TrayEvent::Quit);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_TRAY_ALERT => {
                // SAFETY: o lParam veio de `alert`, que fez `Box::into_raw`.
                let payload = Box::from_raw(lparam.0 as *mut (String, String));
                balloon(hwnd, &payload.0, &payload.1);
                force_front(main_or_find(main_hwnd().0 as isize));
                emit(TrayEvent::Open);
                LRESULT(0)
            }
            WM_TRAY_QUIT => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Mostra o menu de contexto na posição do cursor.
    unsafe fn show_menu(hwnd: HWND) {
        let Ok(menu) = CreatePopupMenu() else { return };
        let abrir = "Abrir Elo".encodeutf16_nul();
        let sair = "Sair".encodeutf16_nul();
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, PCWSTR(abrir.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(sair.as_ptr()));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Exigido para o menu fechar corretamente ao clicar fora dele.
        let _ = SetForegroundWindow(hwnd);
        let flags = TPM_RIGHTBUTTON.0 | TPM_LEFTALIGN.0;
        // Sem TPM_RETURNCMD: a escolha chega como WM_COMMAND na wndproc.
        let _ = TrackPopupMenuEx(menu, flags, pt.x, pt.y, hwnd, None);
        let _ = DestroyMenu(menu);
    }

    /// Tenta o ícone embutido do exe (id 1, do elo.ico via winresource); se não
    /// vier, cai para o ícone padrão do Windows. Nunca falha.
    unsafe fn load_icon(hinstance: HINSTANCE) -> HICON {
        // MAKEINTRESOURCEW(1): id de recurso numérico vira um PCWSTR "falso".
        let from_exe = LoadIconW(hinstance, PCWSTR(1 as *const u16)).unwrap_or_default();
        if !from_exe.0.is_null() {
            return from_exe;
        }
        LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
    }

    /// Laço da thread da bandeja: cria a janela, adiciona o ícone e bombeia
    /// mensagens até a janela ser destruída; então remove o ícone.
    unsafe fn run(
        tooltip: String,
        tx: Sender<TrayEvent>,
        hwnd_tx: Sender<isize>,
        wake: egui::Context,
        main_hwnd: Arc<AtomicIsize>,
        really_quit: Arc<AtomicBool>,
    ) {
        SENDER.with(|s| *s.borrow_mut() = Some(tx));
        WAKE.with(|w| *w.borrow_mut() = Some(wake));
        MAIN_HWND.with(|h| *h.borrow_mut() = Some(main_hwnd));
        REALLY_QUIT.with(|q| *q.borrow_mut() = Some(really_quit));

        let hinstance: HINSTANCE = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).unwrap_or_default();
        let class_name = "EloTrayWindow".encodeutf16_nul();
        let window_name = "Elo".encodeutf16_nul();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        // Ignora o retorno: se a classe já existir (start chamado 2x), seguimos.
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_name.as_ptr()),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND_MESSAGE, // janela message-only (sem barra de tarefas / Alt+Tab)
            None,
            hinstance,
            None,
        );
        let hwnd = match hwnd {
            Ok(h) => h,
            Err(_) => {
                let _ = hwnd_tx.send(0);
                return;
            }
        };

        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: load_icon(hinstance),
            ..Default::default()
        };
        // Tooltip "Elo" (szTip tem 128 u16, incluindo o nulo).
        let tip: Vec<u16> = tooltip.encode_utf16().take(127).collect();
        let mut tip_buf = [0u16; 128];
        tip_buf[..tip.len()].copy_from_slice(&tip);
        nid.szTip = tip_buf;
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);

        // Avisa o chamador que a janela está pronta (devolve o HWND como isize).
        let _ = hwnd_tx.send(hwnd.0 as isize);

        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 <= 0 {
                break; // 0 = WM_QUIT, -1 = erro
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Limpeza: remove o ícone da bandeja.
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }

    /// Inicia a bandeja. Devolve o handle (que remove o ícone no `Drop`) e o
    /// receptor dos eventos de clique/menu.
    pub fn start(
        tooltip: &str,
        wake: egui::Context,
        main_hwnd: Arc<AtomicIsize>,
        really_quit: Arc<AtomicBool>,
    ) -> std::io::Result<(Tray, Receiver<TrayEvent>)> {
        let (tx, rx) = channel::<TrayEvent>();
        let (hwnd_tx, hwnd_rx) = channel::<isize>();
        let tip = tooltip.to_string();
        std::thread::Builder::new()
            .name("elo-tray".into())
            .spawn(move || unsafe { run(tip, tx, hwnd_tx, wake, main_hwnd, really_quit) })?;

        // Espera a janela existir para poder encerrá-la depois.
        let hwnd = hwnd_rx
            .recv()
            .map_err(|_| std::io::Error::other("a thread da bandeja terminou cedo"))?;
        if hwnd == 0 {
            return Err(std::io::Error::other("não foi possível criar a janela da bandeja"));
        }
        Ok((Tray { hwnd }, rx))
    }
}

// ---------------------------------------------------------------------------
// Não-Windows (stubs para compilar o resto no Linux)
// ---------------------------------------------------------------------------
#[cfg(not(windows))]
mod platform {
    use std::sync::atomic::{AtomicBool, AtomicIsize};
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::Arc;

    use super::TrayEvent;

    /// Stub: não há bandeja fora do Windows.
    pub struct Tray;

    impl Tray {
        pub fn hwnd(&self) -> isize {
            0
        }
    }

    pub fn alert(_tray_hwnd: isize, _main_hwnd: isize, _title: &str, _text: &str) {}

    pub fn main_window_visible(_main_hwnd: isize) -> bool {
        true
    }

    pub fn post_quit(_tray_hwnd: isize) -> bool {
        false
    }

    /// Stub: devolve um canal que nunca recebe eventos.
    pub fn start(
        _tooltip: &str,
        _wake: egui::Context,
        _main_hwnd: Arc<AtomicIsize>,
        _really_quit: Arc<AtomicBool>,
    ) -> std::io::Result<(Tray, Receiver<TrayEvent>)> {
        let (_tx, rx) = channel::<TrayEvent>();
        Ok((Tray, rx))
    }
}
