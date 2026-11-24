//! Página "Manual": um guia de uso de ponta a ponta do Elo, renderizado em
//! egui. Só depende de `ui` e da `Palette` — nenhuma referência a `App` — para
//! poder ser chamada de qualquer tela. O conteúdo segue os fatos reais do app
//! (ver `docs/interface.md`, `docs/acesso-nao-supervisionado.md`,
//! `docs/seguranca.md`, `docs/build-e-uso.md`).

use eframe::egui;
use egui::{Sense, Vec2};

use crate::theme::Palette;

/// Guia de uso completo. Renderiza dentro de uma área rolável vertical.
pub fn manual_page(ui: &mut egui::Ui, pal: &Palette) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            // Limita a largura do texto para manter as linhas legíveis.
            ui.set_max_width(820.0);

            ui.heading("Manual do Elo");
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(
                    "Como usar o Elo de ponta a ponta: receber uma conexão, acessar outro \
                     computador e manter tudo seguro.",
                )
                .color(pal.muted),
            );
            ui.add_space(16.0);

            secao_1(ui, pal);
            secao_2(ui, pal);
            secao_3(ui, pal);
            secao_4(ui, pal);
            secao_5(ui, pal);
            secao_6(ui, pal);

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(
                    "Dúvidas de segurança ou de instalação do serviço? Confira também as \
                     abas Segurança, Serviço e Sobre em Configurações.",
                )
                .small()
                .color(pal.muted),
            );
            ui.add_space(24.0);
        });
    });
}

// ---------------------------------------------------------------------------
// Seções do manual
// ---------------------------------------------------------------------------

fn secao_1(ui: &mut egui::Ui, pal: &Palette) {
    secao(ui, pal, "1", "O que é o Elo", |ui| {
        paragrafo(
            ui,
            pal,
            "O Elo liga dois computadores pela internet ou pela rede local para que \
             um controle o outro: você vê a tela do outro computador e usa o mouse \
             e o teclado como se estivesse na frente dele. A conexão é direta entre \
             as duas máquinas (ponta a ponta) e criptografada; não passa por um \
             servidor que enxergue a sua tela.",
        );
        ui.add_space(6.0);
        subtitulo(ui, pal, "Os dois papéis, na mesma janela");
        paragrafo(
            ui,
            pal,
            "Ao abrir, o Elo já é os dois ao mesmo tempo — um único programa cobre \
             os dois lados:",
        );
        papel(
            ui,
            pal,
            pal.accent,
            "Este computador (host)",
            "Recebe a conexão. É a máquina que vai ser acessada e controlada. \
             O Elo gera o endereço e a senha dela, detecta o IP da rede local e \
             fica escutando, pronto para aceitar quem conectar.",
        );
        papel(
            ui,
            pal,
            pal.accent2,
            "Remoto (controlador)",
            "Acessa outro computador. É daqui que você digita o endereço do outro \
             e assume o controle da tela dele.",
        );
        paragrafo(
            ui,
            pal,
            "Você não escolhe um modo no início: a mesma janela recebe conexões e, \
             quando você quiser, acessa outra máquina.",
        );
    });
}

fn secao_2(ui: &mut egui::Ui, pal: &Palette) {
    secao(ui, pal, "2", "Este dispositivo", |ui| {
        paragrafo(
            ui,
            pal,
            "No cartão \"Este computador\", na tela Início, ficam os dados que outra \
             pessoa precisa para acessar esta máquina.",
        );
        item(
            ui,
            pal,
            "ID curto (9 dígitos): um número fácil de ditar, no lugar do endereço \
             longo. Ele só aparece quando um servidor de diretório está configurado \
             em Configurações → Conexão; o botão Copiar ID o coloca na área de \
             transferência.",
        );
        item(
            ui,
            pal,
            "Endereço completo: o endereço Elo da máquina (fixo, não muda entre \
             execuções). Use o botão Copiar endereço quando não há ID curto.",
        );
        item(
            ui,
            pal,
            "Rede local: o IP e a porta (9000) para quem está na mesma rede conectar \
             direto, sem internet.",
        );
        ui.add_space(6.0);
        subtitulo(ui, pal, "Senha fixa x senha da sessão");
        paragrafo(
            ui,
            pal,
            "Há dois tipos de senha, e a escolha muda quem consegue entrar:",
        );
        item(
            ui,
            pal,
            "Senha fixa (de conexão): definida em Configurações → Segurança → Senha \
             de conexão. Não muda sozinha, e é o que permite o acesso não \
             supervisionado — quem tem essa senha entra a qualquer hora, sem ninguém \
             aprovar.",
        );
        item(
            ui,
            pal,
            "Senha da sessão: quando não há senha fixa, o Elo mostra uma senha \
             aleatória de 6 dígitos na tela inicial. O botão Nova gera outra. Com a \
             opção Senha de uso único ligada, essa senha troca sozinha a cada acesso \
             bem-sucedido.",
        );
        ui.add_space(6.0);
        subtitulo(ui, pal, "Acesso não supervisionado");
        paragrafo(
            ui,
            pal,
            "É acessar esta máquina quando não há ninguém na frente dela (por exemplo \
             o PC do trabalho, de casa). Basta definir uma senha fixa; para acessar \
             até com a tela bloqueada, instale também o serviço do Windows (seção 5).",
        );
        ui.add_space(6.0);
        subtitulo(ui, pal, "2FA (verificação em duas etapas)");
        paragrafo(
            ui,
            pal,
            "Em Configurações → Segurança → Autenticação em duas etapas, o Elo mostra \
             um QR code (e uma chave) para registrar num app autenticador (Google \
             Authenticator, Microsoft Authenticator, Authy…). Depois de ligado, além \
             da senha, as conexões passam a pedir o código de 6 dígitos do app. O \
             código não é pedido quando você aprova o pedido na hora.",
        );
        ui.add_space(6.0);
        subtitulo(ui, pal, "Dispositivos pareados (confiáveis)");
        paragrafo(
            ui,
            pal,
            "Ao aprovar um pedido de conexão, marque \"Confiar neste dispositivo\" \
             para que ele entre direto nas próximas vezes, sem senha e sem nova \
             aprovação (o 2FA, se ligado, ainda é pedido). A lista de pareados fica \
             em Configurações → Segurança → Dispositivos pareados, onde dá para \
             removê-los a qualquer momento.",
        );
    });
}

fn secao_3(ui: &mut egui::Ui, pal: &Palette) {
    secao(ui, pal, "3", "Como conectar a outro computador", |ui| {
        paragrafo(
            ui,
            pal,
            "Na tela Início, em \"Conectar a outro computador\", escolha entre \
             Internet e Rede local, informe o endereço do outro computador, a senha \
             (se tiver) e clique Conectar.",
        );
        item(
            ui,
            pal,
            "Internet: cole o endereço Elo completo do outro computador, ou o ID \
             curto de 9 dígitos (quando há um diretório configurado).",
        );
        item(
            ui,
            pal,
            "Rede local: digite o IP do outro computador (porta 9000 por padrão; use \
             IP:porta para outra).",
        );
        paragrafo(
            ui,
            pal,
            "Sem senha, o outro computador recebe um pedido para Permitir ou Negar a \
             conexão — é só esperar o outro lado aceitar. Se o destino tiver 2FA, uma \
             janela pede o código de 6 dígitos.",
        );
        ui.add_space(6.0);
        subtitulo(ui, pal, "Favoritos");
        paragrafo(
            ui,
            pal,
            "Na aba Favoritos, salve um computador que você acessa sempre: dê um nome, \
             escolha Internet ou Rede local, informe o endereço e a senha e clique \
             Salvar. Depois é só Conectar com um clique. Cada favorito mostra um ponto \
             de status online / offline / verificando…",
        );
        ui.add_space(6.0);
        subtitulo(ui, pal, "Últimos acessos (recentes)");
        paragrafo(
            ui,
            pal,
            "Logo abaixo da barra de conexão, o Elo lista os computadores acessados \
             recentemente, cada um com um ponto de presença (online ou offline). Em \
             cada item você pode:",
        );
        item(ui, pal, "Conectar — reconectar num clique (com a senha salva, se houver).");
        item(ui, pal, "Renomear — dar um nome mais fácil de reconhecer.");
        item(ui, pal, "Remover — tirar da lista.");
        ui.add_space(6.0);
        subtitulo(ui, pal, "Dispositivos na rede");
        paragrafo(
            ui,
            pal,
            "Quando há outras máquinas com o Elo na mesma rede local, elas aparecem \
             automaticamente em \"Dispositivos na rede\", com nome e IP. Clique \
             Conectar para acessar sem precisar digitar o endereço. A conexão em si \
             continua exigindo senha ou aprovação normalmente.",
        );
    });
}

fn secao_4(ui: &mut egui::Ui, pal: &Palette) {
    secao(ui, pal, "4", "Durante a sessão", |ui| {
        paragrafo(
            ui,
            pal,
            "Com a conexão ativa, a janela mostra a tela do computador remoto, e uma \
             barra de ferramentas no topo reúne os controles.",
        );
        item(
            ui,
            pal,
            "Mouse e teclado: você controla a máquina remota normalmente (se o dono \
             dela permitiu o controle; senão, a sessão é só de visualização).",
        );
        item(
            ui,
            pal,
            "Múltiplos monitores: quando o remoto tem mais de uma tela, um seletor de \
             Monitor aparece na barra para alternar entre elas.",
        );
        item(
            ui,
            pal,
            "Ctrl+Alt+Del: enviado ao remoto pelo botão da barra (exige controle e o \
             serviço do Windows instalado no outro computador).",
        );
        item(
            ui,
            pal,
            "Som: ouça o áudio do computador remoto; o botão Som liga ou coloca no \
             mudo.",
        );
        item(
            ui,
            pal,
            "Chat: um painel lateral para trocar mensagens de texto com a pessoa do \
             outro lado.",
        );
        item(
            ui,
            pal,
            "Arquivos (nos dois sentidos): arraste um arquivo para a tela remota para \
             enviá-lo; quem está no host também pode enviar arquivos para você pelo \
             botão Enviar arquivo. Os arquivos recebidos vão para a pasta Downloads \
             (ou, com o serviço, para C:\\ProgramData\\elo\\recebidos).",
        );
        item(
            ui,
            pal,
            "Área de transferência: o texto copiado de um lado fica disponível para \
             colar no outro, nos dois sentidos.",
        );
        item(
            ui,
            pal,
            "Tela cheia (F11): ocupa a tela inteira; o Esc não sai da tela cheia, é \
             enviado ao computador remoto.",
        );
        ui.add_space(6.0);
        paragrafo(
            ui,
            pal,
            "Para não perder a conexão enquanto faz outra coisa, você pode manter a \
             janela por cima das demais ou deixar o Elo rodando em segundo plano na \
             bandeja do sistema (veja a próxima seção).",
        );
    });
}

fn secao_5(ui: &mut egui::Ui, pal: &Palette) {
    secao(ui, pal, "5", "Rodar em segundo plano", |ui| {
        paragrafo(
            ui,
            pal,
            "Para esta máquina continuar acessível mesmo sem a janela à vista, o Elo \
             pode ficar rodando discretamente.",
        );
        item(
            ui,
            pal,
            "Bandeja do sistema: o Elo mostra um ícone na bandeja, perto do relógio. \
             Se não o encontrar, clique na setinha de \"ícones ocultos\" ao lado do \
             relógio.",
        );
        item(
            ui,
            pal,
            "Fechar para a bandeja: fechar a janela não encerra o programa; ele \
             continua em segundo plano, pronto para receber conexões. Abra-o de novo \
             pelo ícone da bandeja.",
        );
        item(
            ui,
            pal,
            "Iniciar com o sistema: em Configurações → Geral, ligue \"Iniciar o Elo \
             com o sistema\" para ele abrir sozinho quando você entra no Windows.",
        );
        ui.add_space(6.0);
        subtitulo(ui, pal, "Serviço do Windows (acesso com a tela bloqueada)");
        paragrafo(
            ui,
            pal,
            "Para acessar a máquina até quando ela está na tela de bloqueio — e para \
             o Ctrl+Alt+Del remoto — instale o serviço em Configurações → Serviço → \
             Instalar serviço (pede confirmação do Windows / UAC). O serviço inicia \
             junto com o Windows, antes do login, e passa a receber as conexões; a \
             janela do Elo vira o painel do serviço (aprovações e aviso de quem está \
             conectado).",
        );
        banner(
            ui,
            pal.warn,
            "Defina e salve a senha fixa ANTES de instalar o serviço: a identidade e \
             a configuração são copiadas para o serviço na instalação. E, para aprovar \
             pedidos de quem não tem a senha, a janela do Elo precisa estar aberta.",
        );
    });
}

fn secao_6(ui: &mut egui::Ui, pal: &Palette) {
    secao(ui, pal, "6", "Segurança (dicas honestas)", |ui| {
        paragrafo(
            ui,
            pal,
            "A conexão é sempre criptografada de ponta a ponta. O que define quem \
             entra é a sua configuração:",
        );
        item(
            ui,
            pal,
            "Prefira exigir senha, aprovação na hora, ou as duas. Ligue o 2FA para \
             uma camada a mais nas conexões por senha, por acesso aberto ou de \
             dispositivo pareado.",
        );
        item(
            ui,
            pal,
            "O acesso não supervisionado deixa o endereço ser a única proteção: \
             qualquer um que o conheça entra sem senha e sem aprovação. Use só se \
             souber o que está fazendo; a tela inicial mostra uma faixa vermelha de \
             aviso enquanto ele estiver ligado.",
        );
        item(
            ui,
            pal,
            "A descoberta na rede local anuncia o nome do dispositivo e o ID curto \
             para quem está na mesma rede (não atravessa a internet, e não expõe \
             tela, arquivos nem controle — a conexão continua exigindo \
             senha/aprovação/2FA).",
        );
        item(
            ui,
            pal,
            "O executável ainda não é assinado, então o Windows Defender pode marcá-lo \
             como falso positivo e até removê-lo. O Elo traz uma ajuda transparente \
             para adicionar uma exceção (sempre com confirmação de administrador, e \
             com o passo manual à vista); ela resolve só a máquina onde foi feita.",
        );
    });
}

// ---------------------------------------------------------------------------
// Helpers de layout (privados)
// ---------------------------------------------------------------------------

/// Um cartão de seção: numeração em destaque + título, e o corpo logo abaixo.
fn secao(ui: &mut egui::Ui, pal: &Palette, numero: &str, titulo: &str, corpo: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(pal.surface)
        .stroke(egui::Stroke::new(1.0_f32, pal.border))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(numero).size(20.0).strong().monospace().color(pal.accent_text));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(titulo).size(19.0).strong().color(pal.text));
            });
            ui.add_space(8.0);
            corpo(ui);
        });
    ui.add_space(14.0);
}

/// Parágrafo comum, com quebra de linha dentro da largura disponível.
fn paragrafo(ui: &mut egui::Ui, pal: &Palette, texto: &str) {
    ui.label(egui::RichText::new(texto).color(pal.text));
    ui.add_space(4.0);
}

/// Subtítulo em negrito dentro de uma seção.
fn subtitulo(ui: &mut egui::Ui, pal: &Palette, texto: &str) {
    ui.label(egui::RichText::new(texto).strong().size(15.0).color(pal.text));
    ui.add_space(2.0);
}

/// Item de lista com marcador.
fn item(ui: &mut egui::Ui, pal: &Palette, texto: &str) {
    ui.horizontal_top(|ui| {
        ui.add_space(2.0);
        ui.label(egui::RichText::new("•").strong().color(pal.accent_text));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 6.0).max(120.0));
            ui.label(egui::RichText::new(texto).color(pal.text));
        });
    });
    ui.add_space(4.0);
}

/// Cartão de papel (host / remoto): ponto colorido, título e descrição.
fn papel(ui: &mut egui::Ui, pal: &Palette, cor: egui::Color32, titulo: &str, desc: &str) {
    egui::Frame::none()
        .fill(pal.elevated)
        .rounding(10.0)
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, cor);
                ui.label(egui::RichText::new(titulo).strong().size(15.0).color(pal.text));
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(desc).color(pal.muted));
        });
    ui.add_space(8.0);
}

/// Faixa colorida de aviso (igual às demais telas).
fn banner(ui: &mut egui::Ui, cor: egui::Color32, texto: &str) {
    ui.add_space(4.0);
    egui::Frame::none()
        .fill(cor.linear_multiply(0.18))
        .stroke(egui::Stroke::new(1.0_f32, cor))
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(texto).color(cor).strong());
        });
}
