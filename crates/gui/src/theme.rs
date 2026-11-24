//! Visual system: palette, egui styling and the shared widgets (buttons,
//! toggles, setting rows, cards, logo). Every screen uses these so buttons,
//! inputs and cards look the same everywhere.

use eframe::egui;
use egui::{Color32, Pos2, Sense, Vec2};

/// A semantic color palette (one per theme): a blue-tinted "low-light" base
/// (not pure black), layered surfaces, a light accent tone for contrast, and
/// high-contrast text.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub bar: Color32,
    pub surface: Color32,
    pub elevated: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub accent2: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub ok: Color32,
}

pub fn palette(light: bool) -> Palette {
    if light {
        Palette {
            bg: Color32::from_rgb(0xF5, 0xF7, 0xFA),
            bar: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            elevated: Color32::from_rgb(0xF0, 0xF2, 0xF6),
            hover: Color32::from_rgb(0xE7, 0xEB, 0xF1),
            border: Color32::from_rgb(0xDD, 0xE2, 0xEA),
            text: Color32::from_rgb(0x1A, 0x20, 0x2C),
            muted: Color32::from_rgb(0x64, 0x70, 0x80),
            accent: Color32::from_rgb(0x12, 0xB3, 0xA6),
            accent_text: Color32::from_rgb(0x0C, 0x8F, 0x84),
            on_accent: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            accent2: Color32::from_rgb(0x4F, 0x55, 0xE0),
            warn: Color32::from_rgb(0xC8, 0x7A, 0x12),
            danger: Color32::from_rgb(0xD1, 0x44, 0x4A),
            ok: Color32::from_rgb(0x1F, 0xA2, 0x5A),
        }
    } else {
        Palette {
            bg: Color32::from_rgb(0x0F, 0x14, 0x1B),
            bar: Color32::from_rgb(0x12, 0x18, 0x21),
            surface: Color32::from_rgb(0x18, 0x1F, 0x29),
            elevated: Color32::from_rgb(0x20, 0x29, 0x35),
            hover: Color32::from_rgb(0x27, 0x32, 0x40),
            border: Color32::from_rgb(0x2C, 0x37, 0x45),
            text: Color32::from_rgb(0xE7, 0xEC, 0xF2),
            muted: Color32::from_rgb(0x98, 0xA4, 0xB3),
            accent: Color32::from_rgb(0x2D, 0xD4, 0xBF),
            accent_text: Color32::from_rgb(0x5E, 0xEA, 0xD4),
            on_accent: Color32::from_rgb(0x05, 0x24, 0x20),
            accent2: Color32::from_rgb(0x8B, 0x97, 0xF8),
            warn: Color32::from_rgb(0xEC, 0xB4, 0x4E),
            danger: Color32::from_rgb(0xF2, 0x6D, 0x72),
            ok: Color32::from_rgb(0x4A, 0xDE, 0x80),
        }
    }
}

/// Fully style egui so every default button/input/panel looks consistent.
pub fn apply_theme(ctx: &egui::Context, pal: &Palette, light: bool) {
    use egui::{Rounding, Stroke};
    let mut v = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
    let r = Rounding::same(8.0);

    v.override_text_color = Some(pal.text);
    v.panel_fill = pal.bg;
    v.window_fill = pal.surface;
    v.window_stroke = Stroke::new(1.0_f32, pal.border);
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(10.0);
    v.faint_bg_color = pal.surface;
    v.extreme_bg_color = pal.elevated;
    v.selection.bg_fill = pal.accent.linear_multiply(0.30);
    v.selection.stroke = Stroke::new(1.0_f32, pal.accent);
    v.hyperlink_color = pal.accent_text;

    v.widgets.noninteractive.bg_fill = pal.surface;
    v.widgets.noninteractive.weak_bg_fill = pal.surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, pal.text);
    v.widgets.noninteractive.rounding = r;

    v.widgets.inactive.bg_fill = pal.elevated;
    v.widgets.inactive.weak_bg_fill = pal.elevated;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, pal.text);
    v.widgets.inactive.rounding = r;

    v.widgets.hovered.bg_fill = pal.hover;
    v.widgets.hovered.weak_bg_fill = pal.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, pal.text);
    v.widgets.hovered.rounding = r;
    v.widgets.hovered.expansion = 1.0;

    v.widgets.active.bg_fill = pal.accent.linear_multiply(0.85);
    v.widgets.active.weak_bg_fill = pal.hover;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, pal.text);
    v.widgets.active.rounding = r;

    v.widgets.open = v.widgets.inactive;
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 34.0;
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(21.0)),
        (TextStyle::Body, FontId::proportional(14.5)),
        (TextStyle::Button, FontId::proportional(14.5)),
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Monospace, FontId::monospace(13.5)),
    ]
    .into();
    ctx.set_style(style);
}

/// Filled accent button.
pub fn primary_button(ui: &mut egui::Ui, pal: &Palette, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(15.0).strong().color(pal.on_accent))
            .fill(pal.accent)
            .min_size(egui::vec2(0.0, 34.0))
            .rounding(8.0),
    )
}

/// Botão de ação demorada: enquanto `busy`, fica desabilitado com `busy_label`
/// (gerúndio) e um spinner ao lado. `primary` usa o estilo de destaque.
/// O retorno só tem `clicked()` verdadeiro quando não está ocupado.
pub fn action_button(ui: &mut egui::Ui, pal: &Palette, busy: bool, primary: bool, label: &str, busy_label: &str) -> egui::Response {
    if busy {
        let r = ui.add_enabled(false, egui::Button::new(busy_label).min_size(egui::vec2(0.0, 34.0)).rounding(8.0));
        ui.spinner();
        return r.on_disabled_hover_text("Aguarde, isso ainda está em andamento.");
    }
    if primary {
        primary_button(ui, pal, label)
    } else {
        ui.add(egui::Button::new(label).min_size(egui::vec2(0.0, 34.0)).rounding(8.0))
    }
}

/// Botão de destaque que pode ficar desabilitado (com a explicação no hover).
pub fn primary_button_if(ui: &mut egui::Ui, pal: &Palette, enabled: bool, label: &str, why_disabled: &str) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(label).size(15.0).strong().color(pal.on_accent))
            .fill(pal.accent)
            .min_size(egui::vec2(0.0, 34.0))
            .rounding(8.0),
    )
    .on_disabled_hover_text(why_disabled)
}

/// Filled red button for destructive actions.
pub fn danger_button(ui: &mut egui::Ui, pal: &Palette, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).strong().color(Color32::WHITE))
            .fill(pal.danger)
            .min_size(egui::vec2(0.0, 32.0))
            .rounding(8.0),
    )
}

/// Pill toggle switch.
pub fn toggle_switch(ui: &mut egui::Ui, pal: &Palette, on: &mut bool) -> egui::Response {
    let size = Vec2::new(42.0, 24.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let radius = rect.height() / 2.0;
    let bg = if *on {
        pal.accent
    } else if ui.visuals().dark_mode {
        pal.elevated
    } else {
        pal.border
    };
    let p = ui.painter();
    p.rect_filled(rect, radius, bg);
    let cx = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    p.circle_filled(egui::pos2(cx, rect.center().y), radius - 3.0, Color32::WHITE);
    resp
}

/// Title + description on the left, a toggle on the right. Returns true if changed.
pub fn toggle_row(ui: &mut egui::Ui, pal: &Palette, title: &str, desc: &str, on: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 60.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(w);
            ui.label(egui::RichText::new(title).strong().color(pal.text));
            ui.label(egui::RichText::new(desc).small().color(pal.muted));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed = toggle_switch(ui, pal, on).changed();
        });
    });
    changed
}

/// Title + description on the left, any widget(s) on the right.
pub fn setting_row(ui: &mut egui::Ui, pal: &Palette, title: &str, desc: &str, right_w: f32, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let w = (ui.available_width() - right_w - 10.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(w);
            ui.label(egui::RichText::new(title).strong().color(pal.text));
            if !desc.is_empty() {
                ui.label(egui::RichText::new(desc).small().color(pal.muted));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// Uppercase section label with a one-line description.
pub fn settings_section(ui: &mut egui::Ui, pal: &Palette, label: &str, desc: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(label.to_uppercase()).size(12.0).strong().color(pal.muted));
    if !desc.is_empty() {
        ui.label(egui::RichText::new(desc).small().color(pal.muted));
    }
    ui.add_space(8.0);
}

/// A bordered card container.
pub fn card<R>(ui: &mut egui::Ui, pal: &Palette, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(pal.surface)
        .stroke(egui::Stroke::new(1.0_f32, pal.border))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, add)
        .inner
}

/// Card with a title, followed by spacing.
pub fn settings_group(ui: &mut egui::Ui, pal: &Palette, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    card(ui, pal, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(title).strong().size(15.0).color(pal.text));
        ui.add_space(8.0);
        add(ui);
    });
    ui.add_space(12.0);
}

/// Soft info card with an accent dot.
pub fn info_card(ui: &mut egui::Ui, pal: &Palette, accent: Color32, title: &str, body: &str) {
    egui::Frame::none()
        .fill(pal.surface)
        .stroke(egui::Stroke::new(1.0_f32, pal.border))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_min_height(104.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, accent);
                ui.label(egui::RichText::new(title).strong().size(15.0).color(pal.text));
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new(body).color(pal.muted));
        });
}

/// A colored banner (warnings / status).
pub fn banner(ui: &mut egui::Ui, color: Color32, text: &str) {
    egui::Frame::none()
        .fill(color.linear_multiply(0.18))
        .stroke(egui::Stroke::new(1.0_f32, color))
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(text).color(color).strong());
        });
}

/// Uppercase muted label.
pub fn section_title(ui: &mut egui::Ui, pal: &Palette, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(12.0).strong().color(pal.muted));
}

/// Sidebar / tab navigation item.
pub fn nav_item(ui: &mut egui::Ui, pal: &Palette, selected: bool, label: &str) -> bool {
    let text = egui::RichText::new(label).size(15.0).color(if selected { pal.accent_text } else { pal.text });
    ui.add_sized([ui.available_width(), 38.0], egui::SelectableLabel::new(selected, text)).clicked()
}

/// The Elo mark: two interlocking links (an "elo" = a chain link / bond).
pub fn paint_logo(painter: &egui::Painter, center: Pos2, r: f32, pal: &Palette) {
    let w = r * 0.36;
    let dx = r * 0.58;
    painter.circle_stroke(center - Vec2::new(dx, 0.0), r, egui::Stroke::new(w, pal.accent2));
    painter.circle_stroke(center + Vec2::new(dx, 0.0), r, egui::Stroke::new(w, pal.accent));
}

/// A round avatar with initials.
pub fn avatar(ui: &mut egui::Ui, pal: &Palette, name: &str, size: f32) {
    let initial: String = name.chars().next().unwrap_or('?').to_uppercase().collect();
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, pal.accent);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(size * 0.45),
        pal.on_accent,
    );
}

/// A small colored status dot.
pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 4.0, color);
}
