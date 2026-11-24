//! Shell (sidebar + banners), Home and Favorites pages.

use eframe::egui;
use egui::{Sense, Vec2};

use crate::hostrun::DIRECT_PORT;
use crate::theme::{self, action_button, banner, card, dot, info_card, nav_item, paint_logo, primary_button, primary_button_if, section_title};
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
    let app_busy_file = app.is_busy("host_file");
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
                        if theme::danger_button(ui, &pal, "Desconectar").on_hover_text("Encerra o acesso de quem está conectado a este computador").clicked() {
                            kick = true;
                        }
                        if ui.button("Chat").on_hover_text("Abre a conversa com quem está conectado").clicked() {
                            open_chat = true;
                        }
                        if host_link.is_some() {
                            let sending = app_busy_file;
                            if action_button(ui, &pal, sending, false, "Enviar arquivo", "Enviando…").on_hover_text("Escolhe um arquivo para enviar ao computador remoto").clicked() {
                                pick_file = true;
                            }
                        }
                    });
                });
            });
    }
    let mut kicked: Option<String> = None;
    if let Some(s) = &mut app.host_session {
        if kick {
            s.ctl.kick();
            kicked = Some(s.name.clone());
        }
        if open_chat {
            s.chat_open = true;
        }
    }
    if let Some(name) = kicked {
        app.toast(format!("Encerrando o acesso de {name}…"));
    }
    if pick_file {
        if let Some(link) = host_link {
            // Diálogo, leitura e envio fora da thread da UI.
            app.run_bg(ctx, "host_file", move || {
                let Some(path) = rfd::FileDialog::new().pick_file() else { return Ok(String::new()) };
                let bytes = std::fs::read(&path).map_err(|e| format!("Não consegui ler o arquivo: {e}"))?;
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "arquivo".into());
                let msg = format!("Enviando {name} ({} KB) ao computador remoto…", bytes.len() / 1024);
                link.send_file(name, bytes);
                Ok(msg)
            });
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
            let empty = app.remote_addr.trim().is_empty();
            let clicked = ui
                .add_enabled(
                    !connecting && !empty,
                    egui::Button::new(egui::RichText::new(if connecting { "Conectando…" } else { "Conectar" }).size(15.0).strong().color(pal.on_accent))
                        .fill(pal.accent)
                        .min_size(egui::vec2(btn_w, 34.0))
                        .rounding(8.0),
                )
                .on_disabled_hover_text(if connecting { "Aguarde: já há uma conexão em andamento." } else { "Informe o ID ou o endereço do computador." })
                .clicked();
            let enter = (field.lost_focus() || pw.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !connecting && (clicked || enter) {
                if empty {
                    app.toast_error("Informe o ID ou o endereço do computador.");
                } else {
                    app.start_connect(None);
                }
            }
        });
        ui.label(
            egui::RichText::new("Sem senha, o outro computador recebe um pedido para aprovar a conexão.")
                .small()
                .color(pal.muted),
        );
    });
    let mut cancel = false;
    match &app.conn {
        Conn::Connecting(_) => {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new("conectando… (se pediu aprovação, aguarde o outro lado aceitar)").color(pal.muted));
                if ui.button("Cancelar").on_hover_text("Desiste desta conexão").clicked() {
                    cancel = true;
                }
            });
        }
        Conn::Failed(e) => {
            ui.add_space(8.0);
            ui.colored_label(pal.danger, e.clone());
        }
        _ => {}
    }
    if cancel {
        app.cancel_connect();
        app.toast("Conexão cancelada.");
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
                let id = app.my_short_id.clone();
                app.copy_button(ui, "my-id", "Copiar ID", &id, "ID copiado", false).on_hover_text("Copia o ID de 9 dígitos deste computador");
                ui.add_space(6.0);
                ui.label(egui::RichText::new("endereço completo").small().color(pal.muted));
            }
            let mut node = app.my_node.lock().unwrap().clone();
            ui.add(egui::TextEdit::singleline(&mut node).desired_width(640.0).font(egui::TextStyle::Monospace).horizontal_align(egui::Align::Center));
            ui.add_space(6.0);
            app.copy_button(ui, "my-addr", "Copiar endereço", &node, "Endereço copiado", false).on_hover_text("Copia o endereço completo para colar no outro computador");
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
                    ui.add_space((ui.available_width() - 430.0).max(0.0) / 2.0);
                    ui.label(egui::RichText::new(if app.config.otp { "Senha de uso único:" } else { "Senha desta sessão:" }).color(pal.muted));
                    ui.label(egui::RichText::new(&pw).size(22.0).strong().monospace().color(pal.accent_text));
                    if ui.button("Nova").on_hover_text("Gerar outra senha").clicked() {
                        *app.session_password.lock().unwrap() = random_code();
                        app.toast("Nova senha gerada.");
                    }
                    let pw2 = pw.clone();
                    app.copy_button(ui, "my-pw", "Copiar", &pw2, "Senha copiada", false).on_hover_text("Copia a senha desta sessão");
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
            app.toast(if v { "O Elo vai continuar na bandeja ao fechar a janela." } else { "Fechar a janela agora encerra o Elo." });
        }
    });
}

/// Pede a instalação elevada numa thread (a chamada espera o UAC) e avisa o resultado.
pub fn run_install(app: &mut App, ctx: &egui::Context) {
    app.run_bg(ctx, "install", || {
        crate::install::elevate_install()
            .map(|()| "Instalando… confirme no UAC. A instalação roda em segundo plano.".to_string())
            .map_err(|e| format!("Não foi possível iniciar a instalação: {e}"))
    });
}

/// Cartão no topo da tela inicial para instalar o Elo no Windows (um clique →
/// UAC). Some quando já instalado: o local e o "Desinstalar" ficam em
/// Configurações → Serviço. Relê `is_installed()` a cada frame.
fn install_card(app: &mut App, ui: &mut egui::Ui) {
    if crate::install::is_installed() {
        return;
    }
    let pal = app.pal;
    let mut do_install = false;
    let installing = app.is_busy("install");
    card(ui, &pal, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new("Instalar no Windows").strong().size(15.0).color(pal.text));
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Copia o Elo para Arquivos de Programas, faz com que ele inicie junto com o Windows e instala \
                 o serviço que permite acessar este computador com a tela bloqueada e usar Ctrl+Alt+Del. \
                 O Windows pedirá a sua confirmação (UAC) uma única vez.",
            )
            .color(pal.muted),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if action_button(ui, &pal, installing, true, "  Instalar  ", "Instalando…").on_hover_text("Instala o Elo no Windows (pede confirmação do UAC)").clicked() {
                do_install = true;
            }
        });
    });
    if do_install {
        run_install(app, ui.ctx());
    }
    ui.add_space(12.0);
}

/// "Conectar" de uma linha de lista: desabilitado enquanto há uma conexão em
/// andamento; na linha do alvo mostra "Conectando…".
fn connect_button(ui: &mut egui::Ui, pal: &theme::Palette, connecting: bool, this: bool) -> bool {
    let label = if connecting && this { "Conectando…" } else { "Conectar" };
    ui.add_enabled(
        !connecting,
        egui::Button::new(egui::RichText::new(label).size(15.0).strong().color(pal.on_accent))
            .fill(pal.accent)
            .min_size(egui::vec2(0.0, 34.0))
            .rounding(8.0),
    )
    .on_disabled_hover_text("Aguarde: já há uma conexão em andamento.")
    .clicked()
}

/// "Dispositivos na rede": devices announced on the LAN. Clicking connects.
fn devices_section(app: &mut App, ui: &mut egui::Ui) {
    /// Quanto tempo uma busca fica "em andamento" (anúncios chegam a cada 2 s).
    const SCAN: std::time::Duration = std::time::Duration::from_secs(4);
    let pal = app.pal;
    let mut devices = app.discovery.as_ref().map(|h| h.devices()).unwrap_or_default();
    // Este computador (mesmo ID anunciado pelo serviço) não aparece na lista.
    devices.retain(|d| d.short_id.is_empty() || d.short_id != app.my_short_id);

    // Fim de uma busca: diz o resultado.
    let scanning = match app.discovery_scan_until {
        Some(t) if std::time::Instant::now() < t => {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            true
        }
        Some(_) => {
            app.discovery_scan_until = None;
            if app.discovery_error.is_none() {
                app.toast(match devices.len() {
                    0 => "Busca concluída: nenhum dispositivo com o Elo na rede.".to_string(),
                    1 => "Busca concluída: 1 dispositivo encontrado.".to_string(),
                    n => format!("Busca concluída: {n} dispositivos encontrados."),
                });
            }
            false
        }
        None => false,
    };

    ui.add_space(22.0);
    let mut refresh = false;
    ui.horizontal(|ui| {
        section_title(ui, &pal, "Dispositivos na rede");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if scanning {
                ui.add_enabled(false, egui::Button::new("Procurando…"));
                ui.spinner();
            } else if ui.button("Atualizar").on_hover_text("Procura de novo os aparelhos com o Elo nesta rede").clicked() {
                refresh = true;
            }
        });
    });
    ui.add_space(8.0);
    let mut connect: Option<usize> = None;
    let connecting = matches!(app.conn, Conn::Connecting(_));
    let cur_addr = app.remote_addr.trim().to_string();
    if let Some(err) = &app.discovery_error {
        card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            ui.colored_label(pal.danger, "Não foi possível procurar dispositivos na rede.");
            ui.label(
                egui::RichText::new(format!(
                    "{err}. A porta UDP 9001 pode estar em uso por outro programa ou bloqueada pelo firewall. Toque em Atualizar para tentar de novo."
                ))
                .small()
                .color(pal.muted),
            );
        });
        ui.add_space(8.0);
    } else if devices.is_empty() {
        card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            if scanning {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new("Procurando aparelhos com o Elo nesta rede…").color(pal.muted));
                });
            } else {
                ui.label(
                    egui::RichText::new(
                        "Nenhum aparelho com o Elo encontrado na rede local. Outros computadores aparecem quando o Elo está aberto \
                         neles; um celular aparece quando \"Compartilhar esta tela\" está ligado no app.",
                    )
                    .color(pal.muted),
                );
            }
        });
        ui.add_space(8.0);
    }
    for (i, d) in devices.iter().enumerate() {
        card(ui, &pal, |ui| {
            ui.horizontal(|ui| {
                dot(ui, if d.accepting { pal.ok } else { pal.muted });
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&d.name).strong().size(15.0).color(pal.text));
                    let sub = if d.accepting {
                        format!("Rede local · {}", d.ip)
                    } else {
                        format!("Rede local · {} · compartilhamento de tela desligado", d.ip)
                    };
                    ui.label(egui::RichText::new(sub).small().color(pal.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if d.accepting {
                        if connect_button(ui, &pal, connecting, cur_addr == d.ip) {
                            connect = Some(i);
                        }
                    } else {
                        ui.add_enabled(false, egui::Button::new("Conectar")).on_disabled_hover_text(
                            "Ligue \"Compartilhar esta tela\" no app do celular para poder acessá-lo.",
                        );
                    }
                });
            });
        });
        ui.add_space(8.0);
    }
    if refresh {
        match &app.discovery {
            // Escutador vivo: só esquece a lista; ela volta com os próximos anúncios.
            Some(h) => h.clear(),
            // Sem escutador (falhou antes): tenta abrir a porta de novo.
            None => {
                let _g = app.rt.enter();
                match elo_client_core::discovery::listen() {
                    Ok(h) => {
                        app.discovery = Some(h);
                        app.discovery_error = None;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "descoberta na LAN indisponível");
                        app.discovery_error = Some(e.to_string());
                        app.toast(format!("Não foi possível procurar na rede: {e}"));
                    }
                }
            }
        }
        tracing::info!("busca de dispositivos na rede iniciada");
        app.discovery_scan_until = Some(std::time::Instant::now() + SCAN);
        ui.ctx().request_repaint();
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
    let connecting = matches!(app.conn, Conn::Connecting(_));
    let cur_addr = app.remote_addr.trim().to_string();
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
                    if primary_button(ui, &pal, "Salvar").on_hover_text("Salva o novo nome").clicked() || enter {
                        commit_rename = true;
                    }
                    if ui.button("Cancelar").on_hover_text("Descarta a edição do nome").clicked() {
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
                        if ui.button("Remover").on_hover_text("Tira este computador da lista de últimos acessos").clicked() {
                            remove = Some(i);
                        }
                        if ui.button("Favoritar").on_hover_text("Guarda este computador nos Favoritos").clicked() {
                            add_fav = Some(i);
                        }
                        if ui.button("Renomear").on_hover_text("Muda o nome exibido").clicked() {
                            start_rename = Some(i);
                        }
                        if connect_button(ui, &pal, connecting, cur_addr == r.address) {
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
            if name.is_empty() {
                app.toast_error("O nome não pode ficar vazio.");
            } else if i < app.config.recents.len() {
                let mut cfg = app.config.clone();
                cfg.recents[i].name = name;
                app.draft.recents = cfg.recents.clone();
                app.save_config(cfg);
                app.toast("Nome atualizado.");
                app.rename = None;
            }
        } else {
            app.rename = None;
        }
    }
    if let Some(i) = remove {
        let mut cfg = app.config.clone();
        cfg.recents.remove(i);
        app.draft.recents = cfg.recents.clone();
        app.save_config(cfg);
        app.rename = None;
        app.toast("Removido dos últimos acessos.");
    }
    if let Some(i) = connect {
        let r = app.config.recents[i].clone();
        app.mode_lan = r.lan;
        app.remote_addr = r.address.clone();
        if r.password.is_empty() {
            // Sem senha salva: preenche o endereço e deixa o usuário digitar a
            // senha (salva ou dinâmica) na barra de conexão.
            app.remote_password.clear();
            app.toast("Digite a senha (se houver) e clique em Conectar.");
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
            if primary_button_if(ui, &pal, !app.remote_addr.trim().is_empty(), "Salvar", "Informe o endereço para salvar o favorito.").clicked() {
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
    let connecting = matches!(app.conn, Conn::Connecting(_));
    let cur_addr = app.remote_addr.trim().to_string();
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
                    if primary_button(ui, &pal, "Salvar").on_hover_text("Salva o novo nome").clicked() || enter {
                        commit_rename = true;
                    }
                    if ui.button("Cancelar").on_hover_text("Descarta a edição do nome").clicked() {
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
                        if ui.button("Remover").on_hover_text("Apaga este favorito").clicked() {
                            remove = Some(i);
                        }
                        if ui.button("Renomear").on_hover_text("Muda o nome exibido").clicked() {
                            start_rename = Some(i);
                        }
                        if connect_button(ui, &pal, connecting, cur_addr == fav.address) {
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
            if name.is_empty() {
                app.toast_error("O nome não pode ficar vazio.");
            } else if i < app.config.favorites.len() {
                let mut cfg = app.config.clone();
                cfg.favorites[i].name = name;
                app.draft.favorites = cfg.favorites.clone();
                app.save_config(cfg);
                app.toast("Nome atualizado.");
                app.rename = None;
            }
        } else {
            app.rename = None;
        }
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
        app.toast("Favorito removido.");
    }
}
