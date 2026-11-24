//! Settings page: General, Security, Connection, Service, Logs, About.
//! Edits go to `app.draft`; "Salvar" persists. 2FA enrollment and removing a
//! paired device apply immediately.

use eframe::egui;
use egui::{Color32, Rect, Sense, Vec2};

use crate::config;
use crate::hostrun::DIRECT_PORT;
#[cfg(windows)]
use crate::theme::action_button;
use crate::theme::{avatar, primary_button_if, card, nav_item, primary_button, setting_row, settings_group, settings_section, toggle_row, toggle_switch};
use crate::{logging, open_folder, short_fp, App, Enroll, SettingsTab};

pub fn page_settings(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    ui.heading("Configurações");
    ui.label(egui::RichText::new("Acesso, segurança, rede e preferências.").color(pal.muted));
    ui.add_space(12.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(180.0);
            for (tab, label) in [
                (SettingsTab::General, "Geral"),
                (SettingsTab::Security, "Segurança"),
                (SettingsTab::Connection, "Conexão"),
                (SettingsTab::Service, "Serviço"),
                (SettingsTab::Logs, "Logs"),
                (SettingsTab::About, "Sobre"),
            ] {
                if nav_item(ui, &pal, app.settings_tab == tab, label) {
                    app.settings_tab = tab;
                }
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_max_width(760.0);
            match app.settings_tab {
                SettingsTab::General => general(app, ui),
                SettingsTab::Security => security(app, ui),
                SettingsTab::Connection => connection(app, ui),
                SettingsTab::Service => service_tab(app, ui),
                SettingsTab::Logs => logs(app, ui),
                SettingsTab::About => about(app, ui),
            }
        });
    });
}

fn is_dirty(app: &App) -> bool {
    serde_json::to_string(&app.draft).ok() != serde_json::to_string(&app.config).ok()
}

fn save_row(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    let dirty = is_dirty(app);
    ui.horizontal(|ui| {
        if primary_button_if(ui, &pal, dirty, "  Salvar  ", "Não há alterações para salvar.").on_hover_text("Grava as alterações").clicked() {
            let cfg = app.draft.clone();
            app.save_config(cfg);
            app.toast("Configurações salvas.");
        }
        if ui
            .add_enabled(dirty, egui::Button::new("Desfazer").min_size(egui::vec2(0.0, 34.0)))
            .on_hover_text("Volta ao que está salvo")
            .on_disabled_hover_text("Não há alterações para desfazer.")
            .clicked()
        {
            app.draft = app.config.clone();
            app.toast("Alterações descartadas.");
        }
        if dirty {
            ui.label(egui::RichText::new("alterações não salvas").small().color(pal.warn));
        }
    });
}

fn general(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    settings_group(ui, &pal, "Conta", |ui| {
        ui.horizontal(|ui| {
            let user = config::current_username();
            avatar(ui, &pal, &user, 48.0);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&user).size(17.0).strong().color(pal.text));
                ui.label(egui::RichText::new(format!("conta deste computador · {}", config::computer_name())).small().color(pal.muted));
            });
        });
        ui.add_space(10.0);
        setting_row(ui, &pal, "Nome exibido", "Como você aparece para quem aprova a sua conexão.", 230.0, |ui| {
            ui.add(egui::TextEdit::singleline(&mut app.draft.display_name).desired_width(220.0).hint_text(config::current_username()));
        });
    });
    settings_group(ui, &pal, "Aparência", |ui| {
        setting_row(ui, &pal, "Tema", "Aplicado na hora; salve para manter.", 200.0, |ui| {
            ui.selectable_value(&mut app.draft.theme, "light".to_string(), "Claro");
            ui.selectable_value(&mut app.draft.theme, "dark".to_string(), "Escuro");
        });
    });
    settings_group(ui, &pal, "Inicialização", |ui| {
        toggle_row(ui, &pal, "Iniciar o Elo com o sistema", "Abre o Elo quando você entra no Windows.", &mut app.draft.autostart);
    });
    save_row(app, ui);
}

fn draw_qr(ui: &mut egui::Ui, data: &str, size: f32) {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        ui.label("(não foi possível gerar o QR)");
        return;
    };
    let w = code.width();
    let colors = code.to_colors();
    let quiet = 2usize;
    let cell = size / (w + quiet * 2) as f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, Color32::WHITE);
    for y in 0..w {
        for x in 0..w {
            if colors[y * w + x] == qrcode::Color::Dark {
                let min = rect.min + egui::vec2((x + quiet) as f32 * cell, (y + quiet) as f32 * cell);
                p.rect_filled(Rect::from_min_size(min, Vec2::splat(cell.ceil())), 0.0, Color32::BLACK);
            }
        }
    }
}

fn twofa(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    let mut on = app.config.twofa || app.enroll.is_some();
    let was = on;
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 60.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(w);
            ui.label(egui::RichText::new("Autenticação em duas etapas (2FA)").strong().color(pal.text));
            ui.label(egui::RichText::new("Também pede um código do app autenticador em conexões por senha, sem supervisão ou de dispositivo pareado. Não é pedido quando você aprova.").small().color(pal.muted));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            toggle_switch(ui, &pal, &mut on);
        });
    });
    if on && !was {
        let secret = elo_client_core::totp::generate_secret();
        let uri = elo_client_core::totp::otpauth_uri(&secret, &config::computer_name(), "Elo");
        app.enroll = Some(Enroll { secret, uri, code: String::new(), error: None });
    }
    if !on && was {
        app.enroll = None;
        if app.config.twofa {
            let mut cfg = app.config.clone();
            cfg.twofa = false;
            cfg.totp_secret.clear();
            app.draft.twofa = false;
            app.draft.totp_secret.clear();
            app.save_config(cfg);
            app.toast("2FA desligado.");
        }
    }
    let mut finish: Option<bool> = None;
    let mut copy_key: Option<String> = None;
    if let Some(en) = &mut app.enroll {
        ui.add_space(8.0);
        egui::Frame::none().fill(pal.elevated).rounding(10.0).inner_margin(egui::Margin::same(12.0)).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                draw_qr(ui, &en.uri, 170.0);
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("1. Escaneie o QR no app autenticador (Google/Microsoft Authenticator, Authy…).").color(pal.text));
                    ui.label(egui::RichText::new("Ou digite a chave:").small().color(pal.muted));
                    let mut key = elo_client_core::totp::secret_to_base32(&en.secret);
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut key).font(egui::TextStyle::Monospace).desired_width(330.0));
                        copy_key = Some(key.clone());
                    });
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("2. Digite o código de 6 dígitos que o app mostra:").color(pal.text));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut en.code).desired_width(110.0).hint_text("000000"));
                        if primary_button(ui, &pal, "Ativar").on_hover_text("Confere o código e liga o 2FA").clicked() {
                            finish = Some(true);
                        }
                        if ui.button("Cancelar").on_hover_text("Desiste de ligar o 2FA").clicked() {
                            finish = Some(false);
                        }
                    });
                    if let Some(e) = &en.error {
                        ui.colored_label(pal.danger, e);
                    }
                });
            });
        });
    } else if app.config.twofa {
        ui.label(egui::RichText::new("2FA ativo.").small().color(pal.accent_text));
    }
    if let Some(k) = copy_key {
        // O botão fica logo abaixo do campo, fora do empréstimo de `app.enroll`.
        ui.horizontal(|ui| {
            app.copy_button(ui, "totp-key", "Copiar chave", &k, "Chave 2FA copiada", true).on_hover_text("Copia a chave para digitar no app autenticador");
        });
    }
    match finish {
        Some(true) => {
            let en = app.enroll.as_mut().unwrap();
            if elo_client_core::totp::verify(&en.secret, &en.code, elo_client_core::totp::now()) {
                let b32 = elo_client_core::totp::secret_to_base32(&en.secret);
                let mut cfg = app.config.clone();
                cfg.twofa = true;
                cfg.totp_secret = b32.clone();
                app.draft.twofa = true;
                app.draft.totp_secret = b32;
                app.enroll = None;
                app.save_config(cfg);
                app.toast("2FA ativado.");
            } else {
                en.error = Some("Código incorreto — confira o relógio do celular e tente de novo.".into());
                app.toast_error("Código 2FA incorreto. Confira o relógio do celular e tente de novo.");
            }
        }
        Some(false) => {
            app.enroll = None;
            app.toast("Ativação do 2FA cancelada.");
        }
        None => {}
    }
}

fn security(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    card(ui, &pal, |ui| {
        ui.set_width(ui.available_width());
        settings_section(ui, &pal, "Acesso", "Quem pode conectar neste computador, e com qual verificação.");
        toggle_row(ui, &pal, "Acesso não supervisionado",
            "Sem aprovação e sem senha: qualquer pessoa que saiba o endereço entra. Use só se souber o que está fazendo.",
            &mut app.draft.open_access);
        if app.draft.open_access {
            ui.colored_label(pal.danger, "Atenção: com isto ligado, o seu endereço vira a única proteção.");
        }
        ui.separator();
        setting_row(ui, &pal, "Senha de conexão",
            "Quem tiver esta senha entra sem aprovação. Tentativas erradas são limitadas.", 230.0, |ui| {
            ui.add(egui::TextEdit::singleline(&mut app.draft.unattended_password).password(true).desired_width(220.0).hint_text("não definida"));
        });
        ui.separator();
        toggle_row(ui, &pal, "Senha de uso único",
            "Sem senha fixa (ou com isto ligado), vale a senha desta sessão, que muda após cada acesso.",
            &mut app.draft.otp);
        ui.separator();
        toggle_row(ui, &pal, "Pedidos de aprovação",
            "Quem não tem a senha pode pedir para entrar; você permite ou nega na hora.",
            &mut app.draft.allow_requests);
        ui.separator();
        twofa(app, ui);

        ui.add_space(14.0);
        settings_section(ui, &pal, "Dispositivos pareados", "Entram sem senha e sem aprovação (o 2FA ainda é pedido, se ligado). Pareie marcando \"confiar\" ao aprovar um pedido.");
        if app.config.trusted.is_empty() {
            ui.label(egui::RichText::new("Nenhum dispositivo pareado.").color(pal.muted));
        } else {
            let mut remove: Option<usize> = None;
            for (i, d) in app.config.trusted.iter().enumerate() {
                ui.horizontal(|ui| {
                    avatar(ui, &pal, &d.name, 30.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&d.name).strong().color(pal.text));
                        ui.label(egui::RichText::new(format!("{} · pareado em {}", short_fp(&d.fingerprint), d.added)).small().monospace().color(pal.muted));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Remover").on_hover_text("Este dispositivo volta a precisar de senha ou aprovação").clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                let mut cfg = app.config.clone();
                cfg.trusted.remove(i);
                app.draft.trusted = cfg.trusted.clone();
                app.save_config(cfg);
                app.toast("Dispositivo removido.");
            }
        }

        ui.add_space(14.0);
        settings_section(ui, &pal, "Controle", "O que a pessoa conectada pode fazer neste computador.");
        toggle_row(ui, &pal, "Permitir controlar mouse e teclado",
            "Desligado: a pessoa só vê a tela.", &mut app.draft.allow_control);
        ui.separator();
        toggle_row(ui, &pal, "Mostrar o ponteiro de quem está vendo",
            "Quando a pessoa não pode controlar, o cursor dela aparece na sua tela com o nome.", &mut app.draft.show_pointer);
        ui.separator();
        toggle_row(ui, &pal, "Área de transferência compartilhada",
            "Texto copiado de um lado fica disponível do outro.", &mut app.draft.allow_clipboard);
        ui.separator();
        toggle_row(ui, &pal, "Transmitir o som deste computador",
            "A pessoa conectada ouve o áudio daqui.", &mut app.draft.allow_audio);
        ui.separator();
        toggle_row(ui, &pal, "Receber arquivos",
            "Arquivos arrastados para a sessão vão para a sua pasta Downloads (com o serviço instalado: C:\\ProgramData\\elo\\recebidos).", &mut app.draft.allow_files);
        #[cfg(windows)]
        {
            ui.separator();
            setting_row(ui, &pal, "Rodar como administrador (UAC)",
                "Permite controlar janelas elevadas (Gerenciador de Tarefas, instaladores). Reinicia o Elo. Com o serviço instalado isso já vale.",
                190.0, |ui| {
                let restarting = app.is_busy("restart_admin");
                if action_button(ui, &pal, restarting, false, "Reiniciar como admin", "Reiniciando…")
                    .on_hover_text("Reabre o Elo com permissão de administrador (pede confirmação do UAC)")
                    .clicked()
                {
                    // Só fecha esta janela se o Windows aceitou (UAC confirmado).
                    let ctx = ui.ctx().clone();
                    app.run_bg(&ctx, "restart_admin", || match crate::service::elevate("--restarted") {
                        Ok(()) => std::process::exit(0),
                        Err(e) => Err(format!("Não reiniciou como administrador: {e}")),
                    });
                }
            });
        }
    });
    ui.add_space(12.0);
    save_row(app, ui);
}

fn connection(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    settings_group(ui, &pal, "Internet", |ui| {
        ui.label(egui::RichText::new("Conexão P2P automática (iroh): ligação direta por NAT hole punching, com relays gratuitos de reserva. Nada a configurar.").color(pal.muted));
    });
    settings_group(ui, &pal, "ID curto (diretório)", |ui| {
        ui.label(egui::RichText::new("Para usar um número de 9 dígitos em vez do endereço longo, informe um servidor de diretório (host:porta) — suba o seu com `elo-client relay`. O vídeo continua P2P; o diretório só traduz o número. Reinicie o Elo após mudar.").color(pal.muted));
        ui.add_space(6.0);
        setting_row(ui, &pal, "Servidor", "", 270.0, |ui| {
            ui.add(egui::TextEdit::singleline(&mut app.draft.directory).desired_width(260.0).hint_text("ex.: meu-vps.com:9000"));
        });
    });
    settings_group(ui, &pal, "Rede local", |ui| {
        ui.label(egui::RichText::new(format!("Aceita conexões diretas na porta TCP {DIRECT_PORT} ({}:{DIRECT_PORT}).", app.my_lan)).color(pal.muted));
    });
    save_row(app, ui);
}

fn service_tab(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    #[cfg(windows)]
    {
        let app_installed = crate::install::is_installed();
        settings_group(ui, &pal, "Instalação", |ui| {
            ui.horizontal(|ui| {
                crate::theme::dot(ui, if app_installed { pal.ok } else { pal.muted });
                let text = if app_installed {
                    format!("Elo instalado em {}", crate::install::INSTALL_DIR_DISPLAY)
                } else {
                    "Elo não está instalado (rodando direto deste arquivo).".to_string()
                };
                ui.label(egui::RichText::new(text).color(pal.text));
            });
            ui.add_space(8.0);
            if app_installed {
                ui.horizontal(|ui| {
                    let busy = app.is_busy("uninstall");
                    if action_button(ui, &pal, busy, false, "Desinstalar o Elo", "Desinstalando…")
                        .on_hover_text("Remove o Elo, o serviço e os dados deste computador (pede confirmação do UAC)")
                        .clicked()
                    {
                        let ctx = ui.ctx().clone();
                        app.run_bg(&ctx, "uninstall", || {
                            crate::install::elevate_uninstall()
                                .map(|()| "Desinstalando… confirme no UAC.".to_string())
                                .map_err(|e| format!("Não foi possível iniciar a desinstalação: {e}"))
                        });
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    let busy = app.is_busy("install");
                    if action_button(ui, &pal, busy, true, "Instalar o Elo", "Instalando…")
                        .on_hover_text("Instala o Elo no Windows (pede confirmação do UAC)")
                        .clicked()
                    {
                        crate::ui_home::run_install(app, ui.ctx());
                    }
                });
            }
        });
        let installed = crate::service::is_installed();
        settings_group(ui, &pal, "Serviço do Windows", |ui| {
            ui.label(egui::RichText::new("Com o serviço, o Elo inicia junto com o Windows (antes do login), aceita conexões com a tela bloqueada e permite Ctrl+Alt+Del remoto. Esta janela vira o painel do serviço (aprovações, aviso de quem está conectado).").color(pal.muted));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                crate::theme::dot(ui, if installed { pal.ok } else { pal.muted });
                ui.label(egui::RichText::new(if installed { "Instalado" } else { "Não instalado" }).strong().color(pal.text));
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if installed {
                    if ui.button("Logs do serviço").on_hover_text("Abre a pasta de logs do serviço no Explorador de Arquivos").clicked() {
                        match open_folder(&config::service_dir().join("logs")) {
                            Ok(()) => app.toast("Abrindo a pasta de logs do serviço…"),
                            Err(e) => app.toast_error(e),
                        }
                    }
                } else if app_installed {
                    // Instalação antiga, sem serviço: reinstalar junta tudo.
                    let busy = app.is_busy("repair");
                    if action_button(ui, &pal, busy, true, "Reparar instalação", "Reparando…")
                        .on_hover_text("Reinstala o Elo junto com o serviço (pede confirmação do UAC)")
                        .clicked()
                    {
                        let ctx = ui.ctx().clone();
                        app.run_bg(&ctx, "repair", || {
                            crate::install::elevate_install()
                                .map(|()| "Confirme no Windows (UAC). O Elo será reinstalado com o serviço.".to_string())
                                .map_err(|e| format!("Não foi possível iniciar a instalação: {e}"))
                        });
                    }
                } else {
                    ui.label(egui::RichText::new("O serviço é instalado junto com o Elo (botão Instalar acima).").color(pal.muted));
                }
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Ao instalar, a identidade e a senha deste computador são copiadas para o serviço — salve a senha fixa antes. Desinstalar o Elo remove o serviço junto.").small().color(pal.muted));
        });
    }
    #[cfg(not(windows))]
    {
        ui.label(egui::RichText::new("O serviço de sistema está disponível apenas no Windows.").color(pal.muted));
        let _ = app;
    }
}

fn logs(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    card(ui, &pal, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Registro de atividades").strong().color(pal.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Limpar visualização").on_hover_text("Esvazia só a lista abaixo; os arquivos de log continuam na pasta").clicked() {
                    logging::clear_view();
                    app.toast("Visualização dos logs limpa.");
                }
                let text = logging::recent().join("\n");
                if text.is_empty() {
                    if ui.add_enabled(false, egui::Button::new("Copiar")).on_disabled_hover_text("Não há registros para copiar.").clicked() {}
                } else {
                    app.copy_button(ui, "logs", "Copiar", &text, "Logs copiados", false).on_hover_text("Copia o registro de atividades para a área de transferência");
                }
                if ui.button("Abrir pasta").on_hover_text("Abre a pasta dos arquivos de log").clicked() {
                    match open_folder(&logging::log_dir()) {
                        Ok(()) => app.toast("Abrindo a pasta de logs…"),
                        Err(e) => app.toast_error(e),
                    }
                }
            });
        });
        ui.label(egui::RichText::new("Arquivos diários guardados por 14 dias. Ao relatar um problema, envie o log dos dois computadores.").small().color(pal.muted));
        ui.add_space(6.0);
        egui::Frame::none().fill(pal.elevated).rounding(8.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
            egui::ScrollArea::both().max_height(430.0).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                let lines = logging::recent();
                if lines.is_empty() {
                    ui.label(egui::RichText::new("(vazio)").color(pal.muted));
                }
                for l in lines {
                    let color = if l.contains(" WARN ") { pal.warn } else if l.contains(" ERROR ") { pal.danger } else { pal.text };
                    ui.label(egui::RichText::new(l).monospace().size(12.0).color(color));
                }
            });
        });
    });
}

fn about(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    settings_group(ui, &pal, "Elo", |ui| {
        ui.label(egui::RichText::new(format!("Versão {}", env!("CARGO_PKG_VERSION"))).color(pal.muted));
        ui.label(egui::RichText::new("Acesso remoto seguro, ponta a ponta (Noise · ChaCha20-Poly1305) e P2P. Software original.").color(pal.muted));
        ui.add_space(6.0);
        let dir = if app.service_mode { config::service_dir() } else { config::config_dir() };
        let kp = crate::hostrun::load_or_create_keypair(&dir);
        let fp: String = kp.public.iter().map(|b| format!("{b:02x}")).collect();
        ui.label(egui::RichText::new(format!("Impressão digital deste computador: {}", short_fp(&fp))).monospace().small().color(pal.muted));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            app.copy_button(ui, "fp", "Copiar impressão digital", &fp, "Impressão digital copiada", false)
                .on_hover_text("Copia a impressão digital completa deste computador");
            if ui.button("Abrir pasta de dados").on_hover_text("Abre a pasta com a configuração do Elo").clicked() {
                match open_folder(&config::config_dir()) {
                    Ok(()) => app.toast("Abrindo a pasta de dados…"),
                    Err(e) => app.toast_error(e),
                }
            }
            if ui.button("Abrir pasta de logs").on_hover_text("Abre a pasta dos arquivos de log").clicked() {
                match open_folder(&logging::log_dir()) {
                    Ok(()) => app.toast("Abrindo a pasta de logs…"),
                    Err(e) => app.toast_error(e),
                }
            }
        });
    });

}
