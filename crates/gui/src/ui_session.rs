//! The controller session view: remote screen, toolbar (monitors, Ctrl+Alt+Del,
//! sound, fullscreen, chat, files) and input forwarding.

use std::sync::mpsc as smpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, TextureOptions, Vec2};
use tokio::sync::mpsc;

use elo_client_core::controller::{ControllerEvent, ControllerHandle};
use elo_protocol::{InputEvent, MonitorInfo, MouseButton};

use crate::theme::{self, primary_button};
use crate::{keymap, short_fp, App, Conn};

pub struct Session {
    pub handle: ControllerHandle,
    pub events: mpsc::Receiver<ControllerEvent>,
    pub label: String,
    pub texture: Option<TextureHandle>,
    pub frame_size: (u32, u32),
    pub chat: Vec<String>,
    pub chat_input: String,
    pub chat_open: bool,
    pub prev_mods: egui::Modifiers,
    pub image_rect: Option<Rect>,
    pub monitors: Vec<MonitorInfo>,
    pub last_pointer: Instant,
    pub next_file_id: u32,
    pub fullscreen: bool,
    pub notice: Option<(String, Instant)>,
    /// Resultados das ações assíncronas (chave, resultado) vindos das tasks.
    res_tx: smpsc::Sender<(&'static str, Result<String, String>)>,
    res_rx: smpsc::Receiver<(&'static str, Result<String, String>)>,
    /// Ações em andamento: Ctrl+Alt+Del, troca de monitor.
    busy_sas: bool,
    busy_monitor: bool,
    /// Arquivos sendo enviados agora.
    sending_files: u32,
}

impl Session {
    pub fn new(handle: ControllerHandle, events: mpsc::Receiver<ControllerEvent>, label: String) -> Self {
        let (res_tx, res_rx) = smpsc::channel();
        Session {
            handle,
            events,
            label,
            texture: None,
            frame_size: (0, 0),
            chat: Vec::new(),
            chat_input: String::new(),
            chat_open: false,
            prev_mods: egui::Modifiers::default(),
            image_rect: None,
            monitors: Vec::new(),
            last_pointer: Instant::now(),
            next_file_id: 1,
            fullscreen: false,
            notice: None,
            res_tx,
            res_rx,
            busy_sas: false,
            busy_monitor: false,
            sending_files: 0,
        }
    }
}

pub fn drive_session(app: &mut App, ctx: &egui::Context) {
    let pal = app.pal;
    // Estado "manter no topo": capturado antes do empréstimo da sessão; o botão
    // da barra alterna a cópia local e aplicamos a mudança ao final.
    let on_top_before = app.always_on_top;
    let mut always_on_top = on_top_before;
    let Conn::Active(session) = &mut app.conn else { return };

    // ---- resultados das ações assíncronas ----
    let mut new_toasts: Vec<(String, bool)> = Vec::new();
    while let Ok((key, res)) = session.res_rx.try_recv() {
        match key {
            "sas" => session.busy_sas = false,
            "monitor" => session.busy_monitor = false,
            "file" => session.sending_files = session.sending_files.saturating_sub(1),
            _ => {}
        }
        match res {
            Ok(m) if m.is_empty() => {}
            Ok(m) => new_toasts.push((m, false)),
            Err(e) => new_toasts.push((e, true)),
        }
    }

    // ---- drain events ----
    let mut disconnected = false;
    loop {
        match session.events.try_recv() {
            Ok(ControllerEvent::Frame { width, height, bgra }) => {
                let mut rgba = vec![0u8; bgra.len()];
                for (dst, src) in rgba.chunks_exact_mut(4).zip(bgra.chunks_exact(4)) {
                    dst[0] = src[2];
                    dst[1] = src[1];
                    dst[2] = src[0];
                    dst[3] = 255;
                }
                let image = ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
                session.frame_size = (width, height);
                match &mut session.texture {
                    Some(tex) => tex.set(image, TextureOptions::LINEAR),
                    None => session.texture = Some(ctx.load_texture("remote", image, TextureOptions::LINEAR)),
                }
            }
            Ok(ControllerEvent::ScreenChanged { .. }) => {}
            Ok(ControllerEvent::Monitors(list)) => session.monitors = list,
            Ok(ControllerEvent::Chat(text)) => {
                session.chat.push(format!("host: {text}"));
                session.chat_open = true;
            }
            Ok(ControllerEvent::FileReceived { path }) => session.chat.push(format!("[arquivo recebido: {}]", path.display())),
            // O bridge já aplica a área de transferência sozinho; aqui só avisamos.
            Ok(ControllerEvent::Clipboard(_)) => {
                new_toasts.push(("Área de transferência recebida do computador remoto.".to_string(), false));
            }
            // Só chegam com `ConnectOptions::raw_media` (app Android); o desktop decodifica/toca aqui mesmo.
            Ok(ControllerEvent::FrameH264 { .. } | ControllerEvent::Audio { .. }) => {}
            Ok(ControllerEvent::Disconnected) => {
                disconnected = true;
                break;
            }
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                disconnected = true;
                break;
            }
        }
    }
    if disconnected {
        if session.fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        restore_window_level(app, ctx);
        for (t, e) in new_toasts {
            app.toasts.push((t, Instant::now(), e));
        }
        app.toast_error("A sessão foi encerrada.");
        app.conn = Conn::Failed("A sessão foi encerrada.".into());
        return;
    }

    let can_control = session.handle.permissions.control;
    let mut to_send: Vec<InputEvent> = Vec::new();
    let mut chats: Vec<String> = Vec::new();
    let mut disconnect = false;
    let mut select_monitor: Option<u32> = None;
    let mut sas = false;

    // ---- toolbar ----
    if !session.fullscreen {
        egui::TopBottomPanel::top("bar")
            .frame(egui::Frame::none().fill(pal.bar).inner_margin(egui::Margin::symmetric(14.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    theme::dot(ui, pal.ok);
                    ui.label(egui::RichText::new(&session.label).strong().color(pal.text));
                    let fp: String = session.handle.peer_fingerprint.iter().map(|b| format!("{b:02x}")).collect();
                    ui.label(egui::RichText::new(short_fp(&fp)).monospace().small().color(pal.muted))
                        .on_hover_text("Impressão digital do computador remoto — confira com o que aparece em Configurações → Sobre lá.");
                    if !can_control {
                        ui.colored_label(pal.warn, "somente visualização");
                    }
                    ui.separator();
                    if session.monitors.len() > 1 {
                        let current = session.monitors.iter().find(|m| m.current).map(|m| m.index).unwrap_or(0);
                        egui::ComboBox::from_id_salt("monitor")
                            .selected_text(if session.busy_monitor { "Trocando…".to_string() } else { format!("Monitor {}", current + 1) })
                            .show_ui(ui, |ui| {
                                for m in &session.monitors {
                                    let label = format!("Monitor {} — {}×{}{}", m.index + 1, m.width, m.height, if m.primary { " (principal)" } else { "" });
                                    if ui.selectable_label(m.current, label).clicked() && !m.current && !session.busy_monitor {
                                        select_monitor = Some(m.index);
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Escolhe qual tela do computador remoto aparece aqui");
                        if session.busy_monitor {
                            ui.spinner();
                        }
                    }
                    if !can_control {
                        ui.add_enabled(false, egui::Button::new("Ctrl+Alt+Del")).on_disabled_hover_text("Esta sessão é somente visualização.");
                    } else if session.busy_sas {
                        ui.add_enabled(false, egui::Button::new("Enviando…")).on_disabled_hover_text("Aguarde: o Ctrl+Alt+Del está sendo enviado.");
                        ui.spinner();
                    } else if ui.button("Ctrl+Alt+Del").on_hover_text("Requer o serviço do Windows no outro computador").clicked() {
                        sas = true;
                    }
                    let audio = session.handle.audio();
                    if ui
                        .button(if audio { "Som: ligado" } else { "Som: mudo" })
                        .on_hover_text(if audio { "Clique para silenciar o som do computador remoto" } else { "Clique para ouvir o som do computador remoto" })
                        .clicked()
                    {
                        session.handle.set_audio(!audio);
                        new_toasts.push((if audio { "Som silenciado." } else { "Som ligado." }.to_string(), false));
                    }
                    if ui.button("Tela cheia").on_hover_text("F11 alterna; Esc não sai (é enviado ao remoto)").clicked() {
                        session.fullscreen = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                    }
                    if ui
                        .selectable_label(always_on_top, "Manter no topo")
                        .on_hover_text("Mantém esta janela acima das outras para você navegar sem perder a sessão de vista.")
                        .clicked()
                    {
                        always_on_top = !always_on_top;
                        new_toasts.push((if always_on_top { "Janela mantida no topo." } else { "Janela volta ao normal." }.to_string(), false));
                    }
                    if ui
                        .button(if session.chat_open { "Fechar chat" } else { "Chat" })
                        .on_hover_text("Mostra ou esconde a conversa com o outro computador")
                        .clicked()
                    {
                        session.chat_open = !session.chat_open;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::danger_button(ui, &pal, "Desconectar").on_hover_text("Encerra a sessão com este computador").clicked() {
                            disconnect = true;
                        }
                    });
                });
                if session.sending_files > 0 {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new(format!("Enviando {} arquivo(s)…", session.sending_files)).small().color(pal.accent_text));
                    });
                } else if let Some((n, t)) = &session.notice {
                    if t.elapsed() < Duration::from_secs(4) {
                        ui.label(egui::RichText::new(n).small().color(pal.accent_text));
                    }
                }
            });
    }
    if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
        session.fullscreen = !session.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(session.fullscreen));
    }
    if disconnect {
        if session.fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        restore_window_level(app, ctx);
        for (t, e) in new_toasts {
            app.toasts.push((t, Instant::now(), e));
        }
        app.toast("Sessão encerrada.");
        app.conn = Conn::Idle;
        return;
    }

    // ---- chat ----
    if session.chat_open && !session.fullscreen {
        egui::SidePanel::right("chat")
            .default_width(280.0)
            .frame(egui::Frame::none().fill(pal.bar).inner_margin(egui::Margin::same(12.0)))
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("Chat").strong().size(16.0).color(pal.text));
                ui.separator();
                egui::ScrollArea::vertical().stick_to_bottom(true).max_height(ui.available_height() - 90.0).show(ui, |ui| {
                    for line in &session.chat {
                        ui.label(egui::RichText::new(line).color(pal.text));
                    }
                });
                ui.separator();
                let resp = ui.add(egui::TextEdit::singleline(&mut session.chat_input).desired_width(f32::INFINITY).hint_text("mensagem…"));
                let empty = session.chat_input.trim().is_empty();
                let send = ui
                    .add_enabled_ui(!empty, |ui| primary_button(ui, &pal, "Enviar"))
                    .inner
                    .on_disabled_hover_text("Digite uma mensagem para enviar.")
                    .clicked()
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if send && !session.chat_input.trim().is_empty() {
                    let text = session.chat_input.trim().to_string();
                    session.chat.push(format!("você: {text}"));
                    chats.push(text);
                    session.chat_input.clear();
                }
                ui.label(egui::RichText::new("Arraste arquivos para a tela para enviar.").small().color(pal.muted));
            });
    }

    // ---- remote screen ----
    egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::BLACK)).show(ctx, |ui| {
        if let Some(tex) = &session.texture {
            let (iw, ih) = session.frame_size;
            let avail = ui.available_size();
            let scale = (avail.x / iw.max(1) as f32).min(avail.y / ih.max(1) as f32).max(0.01);
            let size = Vec2::new(iw as f32 * scale, ih as f32 * scale);
            ui.centered_and_justified(|ui| {
                let (rect, _resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                ui.painter().image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                session.image_rect = Some(rect);
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("aguardando a imagem…").color(pal.muted));
            });
        }
    });

    // ---- dropped files → send ----
    let dropped: Vec<egui::DroppedFile> = ctx.input(|i| i.raw.dropped_files.clone());
    for f in dropped {
        if let Some(path) = f.path {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "arquivo".into());
            let id = session.next_file_id;
            session.next_file_id += 1;
            session.sending_files += 1;
            session.notice = Some((format!("Enviando {name}…"), Instant::now()));
            new_toasts.push((format!("Enviando {name}…"), false));
            let h = session.handle.clone();
            let tx = session.res_tx.clone();
            let c = ctx.clone();
            // Leitura e envio fora da thread da UI.
            app.rt.spawn(async move {
                let res = match tokio::fs::read(&path).await {
                    Ok(bytes) => match h.send_file(id, &name, &bytes).await {
                        Ok(()) => Ok(format!("{name} enviado.")),
                        Err(e) => {
                            tracing::warn!(error = %e, "falha ao enviar arquivo");
                            Err(format!("Não consegui enviar {name}: {e}"))
                        }
                    },
                    Err(e) => Err(format!("Não consegui ler o arquivo {name}: {e}")),
                };
                let _ = tx.send(("file", res));
                c.request_repaint();
            });
        }
    }

    // ---- input ----
    let screen = session.frame_size;
    let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
    let rect = session.image_rect;
    if can_control {
        push_mod_edges(session.prev_mods, mods, &mut to_send);
        session.prev_mods = mods;
        for ev in events {
            match ev {
                egui::Event::PointerMoved(pos) => {
                    if let Some((x, y)) = to_host(pos, rect, screen) {
                        to_send.push(InputEvent::MouseMove { x, y });
                    }
                }
                egui::Event::PointerButton { pos, button, pressed, .. } => {
                    if let (Some((x, y)), Some(btn)) = (to_host(pos, rect, screen), map_button(button)) {
                        to_send.push(InputEvent::MouseMove { x, y });
                        to_send.push(InputEvent::MouseButton { button: btn, down: pressed });
                    }
                }
                egui::Event::MouseWheel { delta, .. } => {
                    let (dx, dy) = (delta.x.round() as i32, delta.y.round() as i32);
                    if dx != 0 || dy != 0 {
                        to_send.push(InputEvent::Scroll { dx, dy });
                    }
                }
                egui::Event::Text(t) => {
                    // Non-ASCII characters (ç, accents from dead keys, €…) are
                    // typed as Unicode; ASCII keys already went as VK codes.
                    for c in t.chars().filter(|c| !c.is_ascii()) {
                        to_send.push(InputEvent::Char { code: c as u32 });
                    }
                }
                egui::Event::Key { key, pressed, repeat, .. } => {
                    if key == egui::Key::F11 {
                        continue;
                    }
                    if !repeat {
                        if let Some(code) = keymap::vk(key) {
                            to_send.push(InputEvent::Key { code, down: pressed });
                        }
                    }
                }
                _ => {}
            }
        }
    } else if session.last_pointer.elapsed() > Duration::from_millis(40) {
        // View-only: show our pointer on the host (throttled).
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
            if let Some((x, y)) = to_host(pos, rect, screen) {
                session.last_pointer = Instant::now();
                let h = session.handle.clone();
                app.rt.spawn(async move {
                    let _ = h.send_pointer(x, y).await;
                });
            }
        }
    }

    let h = session.handle.clone();
    if let Some(i) = select_monitor {
        let h = h.clone();
        session.busy_monitor = true;
        let (tx, c) = (session.res_tx.clone(), ctx.clone());
        app.rt.spawn(async move {
            let res = h.select_monitor(i).await.map(|_| format!("Monitor {} selecionado.", i + 1)).map_err(|e| format!("Não consegui trocar de monitor: {e}"));
            let _ = tx.send(("monitor", res));
            c.request_repaint();
        });
    }
    if sas {
        let h = h.clone();
        session.busy_sas = true;
        let (tx, c) = (session.res_tx.clone(), ctx.clone());
        app.rt.spawn(async move {
            let res = h
                .send_sas()
                .await
                .map(|_| "Ctrl+Alt+Del enviado.".to_string())
                .map_err(|e| format!("Não consegui enviar Ctrl+Alt+Del: {e}"));
            let _ = tx.send(("sas", res));
            c.request_repaint();
        });
    }
    if !to_send.is_empty() {
        let h = h.clone();
        app.rt.spawn(async move {
            for ev in to_send {
                let _ = h.send_input(ev).await;
            }
        });
    }
    for text in chats {
        let h = h.clone();
        let (tx, c) = (session.res_tx.clone(), ctx.clone());
        app.rt.spawn(async move {
            if let Err(e) = h.send_chat(&text).await {
                let _ = tx.send(("chat", Err(format!("Não consegui enviar a mensagem: {e}"))));
                c.request_repaint();
            }
        });
    }
    for (t, e) in new_toasts {
        app.toasts.push((t, Instant::now(), e));
    }

    // Aplica a alternância "manter no topo" só quando mudou.
    if always_on_top != on_top_before {
        let level = if always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        app.always_on_top = always_on_top;
    }
}

/// Volta a janela ao nível normal ao sair da sessão (caso estivesse no topo).
fn restore_window_level(app: &mut App, ctx: &egui::Context) {
    if app.always_on_top {
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
        app.always_on_top = false;
    }
}

fn map_button(b: egui::PointerButton) -> Option<MouseButton> {
    match b {
        egui::PointerButton::Primary => Some(MouseButton::Left),
        egui::PointerButton::Secondary => Some(MouseButton::Right),
        egui::PointerButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

/// Window position → host pixel coordinates (relative to the captured monitor).
fn to_host(pos: Pos2, rect: Option<Rect>, screen: (u32, u32)) -> Option<(i32, i32)> {
    let rect = rect?;
    if !rect.contains(pos) || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    let (sw, sh) = screen;
    let nx = ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
    let ny = ((pos.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
    Some(((nx * (sw.max(2) - 1) as f32).round() as i32, (ny * (sh.max(2) - 1) as f32).round() as i32))
}

fn push_mod_edges(prev: egui::Modifiers, now: egui::Modifiers, out: &mut Vec<InputEvent>) {
    let mut edge = |was: bool, is: bool, code: u16| {
        if was != is {
            out.push(InputEvent::Key { code, down: is });
        }
    };
    edge(prev.shift, now.shift, keymap::VK_SHIFT);
    edge(prev.ctrl, now.ctrl, keymap::VK_CONTROL);
    edge(prev.alt, now.alt, keymap::VK_MENU);
}
