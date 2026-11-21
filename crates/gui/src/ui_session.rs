//! The controller session view: remote screen, toolbar (monitors, Ctrl+Alt+Del,
//! sound, fullscreen, chat, files) and input forwarding.

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
}

impl Session {
    pub fn new(handle: ControllerHandle, events: mpsc::Receiver<ControllerEvent>, label: String) -> Self {
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
        }
    }
}

pub fn drive_session(app: &mut App, ctx: &egui::Context) {
    let pal = app.pal;
    let Conn::Active(session) = &mut app.conn else { return };

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
                            .selected_text(format!("Monitor {}", current + 1))
                            .show_ui(ui, |ui| {
                                for m in &session.monitors {
                                    let label = format!("Monitor {} — {}×{}{}", m.index + 1, m.width, m.height, if m.primary { " (principal)" } else { "" });
                                    if ui.selectable_label(m.current, label).clicked() && !m.current {
                                        select_monitor = Some(m.index);
                                    }
                                }
                            });
                    }
                    if can_control && ui.button("Ctrl+Alt+Del").on_hover_text("Requer o serviço do Windows no outro computador").clicked() {
                        sas = true;
                    }
                    let audio = session.handle.audio();
                    if ui.button(if audio { "Som: ligado" } else { "Som: mudo" }).clicked() {
                        session.handle.set_audio(!audio);
                    }
                    if ui.button("Tela cheia").on_hover_text("F11 alterna; Esc não sai (é enviado ao remoto)").clicked() {
                        session.fullscreen = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                    }
                    if ui.button(if session.chat_open { "Fechar chat" } else { "Chat" }).clicked() {
                        session.chat_open = !session.chat_open;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::danger_button(ui, &pal, "Desconectar").clicked() {
                            disconnect = true;
                        }
                    });
                });
                if let Some((n, t)) = &session.notice {
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
                let send = primary_button(ui, &pal, "Enviar").clicked() || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
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
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "arquivo".into());
                    let id = session.next_file_id;
                    session.next_file_id += 1;
                    session.notice = Some((format!("Enviando {name} ({} KB)…", bytes.len() / 1024), Instant::now()));
                    session.chat.push(format!("[enviado: {name}]"));
                    let h = session.handle.clone();
                    app.rt.spawn(async move {
                        if let Err(e) = h.send_file(id, &name, &bytes).await {
                            tracing::warn!(error = %e, "falha ao enviar arquivo");
                        }
                    });
                }
                Err(e) => session.notice = Some((format!("Não consegui ler o arquivo: {e}"), Instant::now())),
            }
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
        app.rt.spawn(async move {
            let _ = h.select_monitor(i).await;
        });
    }
    if sas {
        let h = h.clone();
        app.rt.spawn(async move {
            let _ = h.send_sas().await;
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
        app.rt.spawn(async move {
            let _ = h.send_chat(&text).await;
        });
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
