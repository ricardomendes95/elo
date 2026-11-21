//! Shell (sidebar + banners), Home and Favorites pages.

use eframe::egui;
use egui::{Sense, Vec2};

use crate::hostrun::DIRECT_PORT;
use crate::theme::{self, banner, card, dot, info_card, nav_item, paint_logo, primary_button, section_title};
use crate::{random_code, short_fp, App, Conn, View};
use elo_protocol::format_id;

pub fn draw_shell(app: &mut App, ctx: &egui::Context) {
    let pal = app.pal;
    egui::SidePanel::left("nav")
        .exact_width(212.0)
        .resizable(false)
        .frame(egui::Frame::none().fill(pal.bar).inner_margin(egui::Margin::symmetric(14.0, 18.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(32.0, 24.0), Sense::hover());
                paint_logo(ui.painter(), r.center(), 9.0, &pal);
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Elo").size(22.0).strong().color(pal.text));
            });
            ui.add_space(22.0);
            if nav_item(ui, &pal, app.view == View::Home, "Início") {
                app.view = View::Home;
            }
            if nav_item(ui, &pal, app.view == View::Favorites, "Favoritos") {
                app.view = View::Favorites;
            }
            if nav_item(ui, &pal, app.view == View::Settings, "Configurações") {
                app.view = View::Settings;
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                let status = app.host_status.lock().unwrap().clone();
                let online = status.contains("online") || status.starts_with("pronto");
                ui.label(egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).small().color(pal.muted));
                ui.horizontal(|ui| {
                    dot(ui, if online { pal.ok } else { pal.muted });
                    ui.label(egui::RichText::new(if online { "online" } else { "iniciando…" }).small().color(pal.muted));
                });
                if app.service_mode {
                    ui.label(egui::RichText::new("serviço do Windows ativo").small().color(pal.muted));
                }
            });
        });

    // Someone is connected to THIS computer.
    let mut kick = false;
    let mut open_chat = false;
    if let Some(s) = &app.host_session {
        egui::TopBottomPanel::top("host-session")
            .frame(egui::Frame::none().fill(pal.danger.linear_multiply(0.22)).inner_margin(egui::Margin::symmetric(16.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    dot(ui, pal.danger);
                    let mins = s.since.elapsed().as_secs() / 60;
                    ui.label(egui::RichText::new(format!("{} está acessando este computador · {} min", s.name, mins)).strong().color(pal.text));
                    ui.label(egui::RichText::new(short_fp(&s.fp)).monospace().small().color(pal.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::danger_button(ui, &pal, "Desconectar").clicked() {
                            kick = true;
                        }
                        if ui.button("Chat").clicked() {
                            open_chat = true;
                        }
                    });
                });
            });
    }
    if let Some(s) = &mut app.host_session {
        if kick {
            s.ctl.kick();
        }
        if open_chat {
            s.chat_open = true;
        }
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::symmetric(28.0, 22.0)))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.view {
                View::Home => page_home(app, ui),
                View::Favorites => page_favorites(app, ui),
                View::Settings => crate::ui_settings::page_settings(app, ui),
            });
        });
}

fn address_bar(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    card(ui, &pal, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.mode_lan, false, "Internet");
            ui.selectable_value(&mut app.mode_lan, true, "Rede local");
            ui.separator();
            let hint = if app.mode_lan { "IP do computador (ex.: 192.168.0.10)" } else { "ID ou endereço Elo do computador" };
            let connecting = matches!(app.conn, Conn::Connecting(_));
            let (btn_w, pw_w) = (124.0, 140.0);
            let field_w = (ui.available_width() - pw_w - btn_w - 22.0).max(140.0);
            let field = ui.add_sized([field_w, 34.0], egui::TextEdit::singleline(&mut app.remote_addr).hint_text(hint));
            let pw = ui.add_sized([pw_w, 34.0], egui::TextEdit::singleline(&mut app.remote_password).password(true).hint_text("senha (opcional)"));
            let clicked = ui
                .add_enabled(
                    !connecting,
                    egui::Button::new(egui::RichText::new("Conectar").size(15.0).strong().color(pal.on_accent))
                        .fill(pal.accent)
                        .min_size(egui::vec2(btn_w, 34.0))
                        .rounding(8.0),
                )
                .clicked();
            let enter = (field.lost_focus() || pw.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !connecting && (clicked || enter) && !app.remote_addr.trim().is_empty() {
                app.start_connect(None);
            }
        });
        ui.label(
            egui::RichText::new("Sem senha, o outro computador recebe um pedido para aprovar a conexão.")
                .small()
                .color(pal.muted),
        );
    });
    match &app.conn {
        Conn::Connecting(_) => {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new("conectando… (se pediu aprovação, aguarde o outro lado aceitar)").color(pal.muted));
            });
        }
        Conn::Failed(e) => {
            ui.add_space(8.0);
            ui.colored_label(pal.danger, e.clone());
        }
        _ => {}
    }
}

fn page_home(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    if app.config.open_access {
        banner(ui, pal.danger, "Acesso não supervisionado ligado: qualquer pessoa com o seu endereço entra sem senha e sem aprovação. Desligue em Configurações → Segurança.");
        ui.add_space(12.0);
    }
    ui.heading("Conectar a outro computador");
    ui.add_space(10.0);
    address_bar(app, ui);

    ui.add_space(24.0);
    card(ui, &pal, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            section_title(ui, &pal, if app.service_mode { "Este computador (serviço do Windows)" } else { "Este computador" });
            ui.add_space(6.0);
            let has_dir = !app.config.directory.trim().is_empty();
            if has_dir {
                ui.label(egui::RichText::new(format_id(&app.my_short_id)).size(40.0).strong().monospace().color(pal.accent_text));
                if ui.button("Copiar ID").clicked() {
                    let id = app.my_short_id.clone();
                    ui.output_mut(|o| o.copied_text = id);
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new("endereço completo").small().color(pal.muted));
            }
            let mut node = app.my_node.lock().unwrap().clone();
            ui.add(egui::TextEdit::singleline(&mut node).desired_width(640.0).font(egui::TextStyle::Monospace).horizontal_align(egui::Align::Center));
            ui.add_space(6.0);
            if ui.button("Copiar endereço").clicked() {
                ui.output_mut(|o| o.copied_text = node.clone());
            }
            ui.add_space(14.0);

            // Password row.
            let fixed = !app.config.unattended_password.trim().is_empty() && !app.config.otp;
            if app.service_mode {
                let msg = if fixed {
                    "Senha fixa definida — acesso não supervisionado disponível.".to_string()
                } else if app.config.allow_requests {
                    "Sem senha fixa: conexões pedem a sua aprovação (com esta janela aberta).".to_string()
                } else {
                    "Defina uma senha fixa ou ligue os pedidos de aprovação em Segurança.".to_string()
                };
                ui.label(egui::RichText::new(msg).color(if fixed { pal.accent_text } else { pal.muted }));
            } else if fixed {
                ui.label(egui::RichText::new("Senha fixa definida — acesso não supervisionado").color(pal.accent_text));
            } else {
                let pw = app.session_password.lock().unwrap().clone();
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 330.0).max(0.0) / 2.0);
                    ui.label(egui::RichText::new(if app.config.otp { "Senha de uso único:" } else { "Senha desta sessão:" }).color(pal.muted));
                    ui.label(egui::RichText::new(&pw).size(22.0).strong().monospace().color(pal.accent_text));
                    if ui.button("Nova").on_hover_text("Gerar outra senha").clicked() {
                        *app.session_password.lock().unwrap() = random_code();
                    }
                });
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("Rede local: {}:{}", app.my_lan, DIRECT_PORT)).small().color(pal.muted));
        });
    });

    ui.add_space(22.0);
    ui.columns(3, |c| {
        info_card(&mut c[0], &pal, pal.accent, "Acesso não supervisionado",
            "Defina uma senha fixa em Segurança (e instale o serviço) para acessar esta máquina de qualquer lugar, até com a tela bloqueada.");
        info_card(&mut c[1], &pal, pal.accent2, "Internet sem servidor",
            "Conexão direta por NAT hole punching (P2P), com relays gratuitos de reserva. Criptografia ponta a ponta.");
        info_card(&mut c[2], &pal, pal.warn, "Dispositivos pareados",
            "Ao aprovar um pedido, marque \"confiar\" para que aquele dispositivo entre direto nas próximas vezes.");
    });
}

fn page_favorites(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    ui.heading("Favoritos");
    ui.add_space(12.0);
    card(ui, &pal, |ui| {
        ui.label(egui::RichText::new("Novo favorito").strong().color(pal.text));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.fav_name).desired_width(150.0).hint_text("nome (ex.: PC do trabalho)"));
            ui.selectable_value(&mut app.mode_lan, false, "Internet");
            ui.selectable_value(&mut app.mode_lan, true, "Rede local");
            ui.add(egui::TextEdit::singleline(&mut app.remote_addr).desired_width(230.0).hint_text("ID, endereço ou IP"));
            ui.add(egui::TextEdit::singleline(&mut app.remote_password).password(true).desired_width(120.0).hint_text("senha"));
            if primary_button(ui, &pal, "Salvar").clicked() {
                app.save_favorite();
            }
        });
    });
    ui.add_space(14.0);
    if app.config.favorites.is_empty() {
        ui.label(egui::RichText::new("Nenhum favorito ainda. Salve aqui o seu PC do trabalho para conectar com um clique.").color(pal.muted));
        return;
    }
    let presence = app.presence.lock().unwrap().clone();
    let mut connect: Option<usize> = None;
    let mut remove: Option<usize> = None;
    for (i, fav) in app.config.favorites.iter().enumerate() {
        card(ui, &pal, |ui| {
            ui.horizontal(|ui| {
                let (color, label) = match presence.get(&fav.address) {
                    Some(true) => (pal.ok, "online"),
                    Some(false) => (pal.danger, "offline"),
                    None => (pal.muted, "verificando…"),
                };
                dot(ui, color);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&fav.name).strong().size(15.0).color(pal.text));
                    let kind = if fav.lan { "Rede local" } else { "Internet" };
                    let addr: String = if fav.address.len() > 28 { format!("{}…", &fav.address[..28]) } else { fav.address.clone() };
                    ui.label(egui::RichText::new(format!("{label} · {kind} · {addr}")).small().color(pal.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Remover").clicked() {
                        remove = Some(i);
                    }
                    if primary_button(ui, &pal, "Conectar").clicked() {
                        connect = Some(i);
                    }
                });
            });
        });
        ui.add_space(8.0);
    }
    if let Some(i) = connect {
        let fav = app.config.favorites[i].clone();
        app.view = View::Home;
        app.start_connect_fav(&fav);
    }
    if let Some(i) = remove {
        let mut cfg = app.config.clone();
        cfg.favorites.remove(i);
        app.draft.favorites = cfg.favorites.clone();
        app.save_config(cfg);
    }
}
