//! Shell (sidebar + banners), Home and Favorites pages.

use eframe::egui;
use egui::{Sense, Vec2};

use crate::hostrun::DIRECT_PORT;
use crate::theme::{self, banner, card, dot, info_card, nav_item, paint_logo, primary_button, section_title};
use crate::{random_code, short_fp, App, Conn, RenameTarget, View};
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
            if nav_item(ui, &pal, app.view == View::Manual, "Manual") {
                app.view = View::Manual;
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
    let mut pick_file = false;
    let host_link = app.host_link.lock().unwrap().clone();
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
                        if host_link.is_some() && ui.button("Enviar arquivo").clicked() {
                            pick_file = true;
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
    if pick_file {
        if let Some(link) = host_link {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "arquivo".into());
                        app.toast(format!("Enviando {name} ({} KB) ao computador remoto…", bytes.len() / 1024));
                        link.send_file(name, bytes);
                    }
                    Err(e) => app.toast(format!("Não consegui ler o arquivo: {e}")),
                }
            }
        }
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(pal.bg).inner_margin(egui::Margin::symmetric(28.0, 22.0)))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.view {
                View::Home => page_home(app, ui),
                View::Favorites => page_favorites(app, ui),
                View::Settings => crate::ui_settings::page_settings(app, ui),
                View::Manual => crate::ui_manual::manual_page(ui, &pal),
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
    if cfg!(windows) {
        install_card(app, ui);
    }
    if app.config.open_access {
        banner(ui, pal.danger, "Acesso não supervisionado ligado: qualquer pessoa com o seu endereço entra sem senha e sem aprovação. Desligue em Configurações → Segurança.");
        ui.add_space(12.0);
    }
    ui.heading("Conectar a outro computador");
    ui.add_space(10.0);
    address_bar(app, ui);

    devices_section(app, ui);
    recents_section(app, ui);

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

    ui.add_space(22.0);
    card(ui, &pal, |ui| {
        let mut v = app.config.close_to_tray;
        if ui
            .checkbox(&mut v, "Continuar na bandeja ao fechar a janela")
            .on_hover_text("Fechar a janela só esconde o Elo: a conexão e o acesso a este computador continuam ativos. Use o ícone na bandeja para reabrir ou sair.")
            .changed()
        {
            let mut cfg = app.config.clone();
            cfg.close_to_tray = v;
            app.draft.close_to_tray = v;
            app.save_config(cfg);
        }
    });
}

/// Cartão no topo da tela inicial: instalar o Elo no Windows (um clique → UAC)
/// ou, se já instalado, um aviso discreto com a opção de desinstalar. A cada
/// frame relê `is_installed()`, então o cartão troca sozinho assim que a
/// instalação/desinstalação termina.
fn install_card(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    let mut do_install = false;
    let mut do_uninstall = false;
    if crate::install::is_installed() {
        egui::Frame::none()
            .fill(pal.ok.linear_multiply(0.14))
            .stroke(egui::Stroke::new(1.0_f32, pal.ok))
            .rounding(12.0)
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    dot(ui, pal.ok);
                    ui.label(
                        egui::RichText::new(format!("Elo instalado em {}", crate::install::INSTALL_DIR_DISPLAY))
                            .color(pal.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Desinstalar").clicked() {
                            do_uninstall = true;
                        }
                    });
                });
            });
    } else {
        card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Instalar no Windows").strong().size(15.0).color(pal.text));
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Copia o Elo para Arquivos de Programas e faz com que ele inicie junto com o Windows, \
                     em segundo plano, com permissão de administrador. O Windows pedirá a sua confirmação (UAC).",
                )
                .color(pal.muted),
            );
            ui.add_space(10.0);
            if primary_button(ui, &pal, "  Instalar  ").clicked() {
                do_install = true;
            }
        });
    }
    if do_install {
        match crate::install::elevate_install() {
            Ok(()) => app.toast("Instalando… confirme no UAC. A instalação roda em segundo plano."),
            Err(e) => app.toast(format!("Não foi possível iniciar a instalação: {e}")),
        }
    }
    if do_uninstall {
        match crate::install::elevate_uninstall() {
            Ok(()) => app.toast("Desinstalando… confirme no UAC."),
            Err(e) => app.toast(format!("Não foi possível iniciar a desinstalação: {e}")),
        }
    }
    ui.add_space(12.0);
}

/// "Dispositivos na rede": devices announced on the LAN. Clicking connects.
fn devices_section(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    let devices = app.discovery.as_ref().map(|h| h.devices()).unwrap_or_default();
    ui.add_space(22.0);
    let mut refresh = false;
    ui.horizontal(|ui| {
        section_title(ui, &pal, "Dispositivos na rede");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Atualizar").clicked() {
                refresh = true;
            }
        });
    });
    ui.add_space(8.0);
    let mut connect: Option<usize> = None;
    if devices.is_empty() {
        card(ui, &pal, |ui| {
            ui.label(
                egui::RichText::new(
                    "Nenhum computador com o Elo encontrado na rede local. Toque em Atualizar para procurar de novo.",
                )
                .color(pal.muted),
            );
        });
        ui.add_space(8.0);
    }
    for (i, d) in devices.iter().enumerate() {
        card(ui, &pal, |ui| {
            ui.horizontal(|ui| {
                dot(ui, pal.ok);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&d.name).strong().size(15.0).color(pal.text));
                    ui.label(egui::RichText::new(format!("Rede local · {}", d.ip)).small().color(pal.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, &pal, "Conectar").clicked() {
                        connect = Some(i);
                    }
                });
            });
        });
        ui.add_space(8.0);
    }
    if refresh {
        // Solta o handle antigo (limpa o mapa) e recria o escutador: começa
        // vazio e volta a se preencher com os anúncios dos próximos instantes.
        let _g = app.rt.enter();
        app.discovery = elo_client_core::discovery::listen().ok();
    }
    if let Some(i) = connect {
        app.mode_lan = true;
        app.remote_addr = devices[i].ip.clone();
        app.start_connect(None);
    }
}

/// "Últimos acessos": recently connected computers, with online dot, rename,
/// remove and one-click reconnect.
fn recents_section(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    if app.config.recents.is_empty() {
        return;
    }
    ui.add_space(22.0);
    section_title(ui, &pal, "Últimos acessos");
    ui.add_space(8.0);
    let presence = app.presence.lock().unwrap().clone();
    let mut connect: Option<usize> = None;
    let mut remove: Option<usize> = None;
    let mut add_fav: Option<usize> = None;
    let mut start_rename: Option<usize> = None;
    let mut commit_rename = false;
    let mut cancel_rename = false;
    for (i, r) in app.config.recents.iter().enumerate() {
        card(ui, &pal, |ui| {
            ui.horizontal(|ui| {
                let color = match presence.get(&r.address) {
                    Some(true) => pal.ok,
                    _ => pal.muted,
                };
                dot(ui, color);
                if app.rename == Some(RenameTarget::Recent(i)) {
                    let resp = ui.add(egui::TextEdit::singleline(&mut app.rename_buf).desired_width(180.0).hint_text("nome"));
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if primary_button(ui, &pal, "Salvar").clicked() || enter {
                        commit_rename = true;
                    }
                    if ui.button("Cancelar").clicked() {
                        cancel_rename = true;
                    }
                } else {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&r.name).strong().size(15.0).color(pal.text));
                        let kind = if r.lan { "Rede local" } else { "Internet" };
                        let addr: String = if r.address.len() > 28 { format!("{}…", &r.address[..28]) } else { r.address.clone() };
                        ui.label(egui::RichText::new(format!("{kind} · {addr}")).small().color(pal.muted));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Remover").clicked() {
                            remove = Some(i);
                        }
                        if ui.button("Favoritar").clicked() {
                            add_fav = Some(i);
                        }
                        if ui.button("Renomear").clicked() {
                            start_rename = Some(i);
                        }
                        if primary_button(ui, &pal, "Conectar").clicked() {
                            connect = Some(i);
                        }
                    });
                }
            });
        });
        ui.add_space(8.0);
    }
    if let Some(i) = add_fav {
        let r = app.config.recents[i].clone();
        if app.add_favorite_unique(r.name, r.address, r.lan, r.password) {
            app.toast("Adicionado aos favoritos.");
        } else {
            app.toast("Já está nos favoritos.");
        }
    }
    if let Some(i) = start_rename {
        app.rename = Some(RenameTarget::Recent(i));
        app.rename_buf = app.config.recents[i].name.clone();
    }
    if cancel_rename {
        app.rename = None;
    }
    if commit_rename {
        if let Some(RenameTarget::Recent(i)) = app.rename {
            let name = app.rename_buf.trim().to_string();
            if !name.is_empty() && i < app.config.recents.len() {
                let mut cfg = app.config.clone();
                cfg.recents[i].name = name;
                app.draft.recents = cfg.recents.clone();
                app.save_config(cfg);
            }
        }
        app.rename = None;
    }
    if let Some(i) = remove {
        let mut cfg = app.config.clone();
        cfg.recents.remove(i);
        app.draft.recents = cfg.recents.clone();
        app.save_config(cfg);
        app.rename = None;
    }
    if let Some(i) = connect {
        let r = app.config.recents[i].clone();
        app.mode_lan = r.lan;
        app.remote_addr = r.address.clone();
        if r.password.is_empty() {
            // Sem senha salva: preenche o endereço e deixa o usuário digitar a
            // senha (salva ou dinâmica) na barra de conexão.
            app.remote_password.clear();
        } else {
            app.remote_password = r.password.clone();
            app.start_connect(None);
        }
    }
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
    let mut start_rename: Option<usize> = None;
    let mut commit_rename = false;
    let mut cancel_rename = false;
    for (i, fav) in app.config.favorites.iter().enumerate() {
        card(ui, &pal, |ui| {
            ui.horizontal(|ui| {
                let (color, label) = match presence.get(&fav.address) {
                    Some(true) => (pal.ok, "online"),
                    Some(false) => (pal.danger, "offline"),
                    None => (pal.muted, "verificando…"),
                };
                dot(ui, color);
                if app.rename == Some(RenameTarget::Favorite(i)) {
                    let resp = ui.add(egui::TextEdit::singleline(&mut app.rename_buf).desired_width(180.0).hint_text("nome"));
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if primary_button(ui, &pal, "Salvar").clicked() || enter {
                        commit_rename = true;
                    }
                    if ui.button("Cancelar").clicked() {
                        cancel_rename = true;
                    }
                } else {
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
                        if ui.button("Renomear").clicked() {
                            start_rename = Some(i);
                        }
                        if primary_button(ui, &pal, "Conectar").clicked() {
                            connect = Some(i);
                        }
                    });
                }
            });
        });
        ui.add_space(8.0);
    }
    if let Some(i) = start_rename {
        app.rename = Some(RenameTarget::Favorite(i));
        app.rename_buf = app.config.favorites[i].name.clone();
    }
    if cancel_rename {
        app.rename = None;
    }
    if commit_rename {
        if let Some(RenameTarget::Favorite(i)) = app.rename {
            let name = app.rename_buf.trim().to_string();
            if !name.is_empty() && i < app.config.favorites.len() {
                let mut cfg = app.config.clone();
                cfg.favorites[i].name = name;
                app.draft.favorites = cfg.favorites.clone();
                app.save_config(cfg);
            }
        }
        app.rename = None;
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
        app.rename = None;
    }
}
