//! Paleta e estilo.
//!
//! A direção é software de áudio profissional, não app de streaming: preto
//! quase absoluto, cantos retos, zero sombra, réguas de 1px, densidade alta.
//!
//! O roxo/azul da identidade entra como **acento único**, nunca como
//! gradiente nem como cor de fundo — ele aparece na faixa tocando e na barra
//! de progresso, e em mais nada. Roxo espalhado é justamente o que faz uma
//! interface parecer genérica.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x0A, 0x0A, 0x0C);
pub const PANEL: Color32 = Color32::from_rgb(0x0E, 0x0E, 0x12);
pub const HOVER: Color32 = Color32::from_rgb(0x16, 0x16, 0x1C);
pub const RULE: Color32 = Color32::from_rgb(0x1C, 0x1C, 0x23);
pub const TEXT: Color32 = Color32::from_rgb(0xCB, 0xCB, 0xD4);
pub const DIM: Color32 = Color32::from_rgb(0x6B, 0x6B, 0x78);
pub const FAINT: Color32 = Color32::from_rgb(0x43, 0x43, 0x4E);
/// O acento. Um só, e usado com parcimônia.
pub const ACCENT: Color32 = Color32::from_rgb(0x7C, 0x5C, 0xFF);

/// Altura de uma linha da lista. 22px cabem ~25 faixas numa janela padrão —
/// menos rolagem numa biblioteca de 50 mil.
pub const ROW_HEIGHT: f32 = 22.0;

pub fn body() -> FontId {
    FontId::new(13.0, FontFamily::Proportional)
}

/// Números — duração, número da faixa — em mono, para as colunas alinharem
/// verticalmente sem truque de layout.
pub fn mono() -> FontId {
    FontId::new(12.0, FontFamily::Monospace)
}

pub fn small() -> FontId {
    FontId::new(11.0, FontFamily::Proportional)
}

pub fn apply(ctx: &egui::Context) {
    // Este desenho se compromete com um único visual: mesmo no tema claro do
    // sistema, o player é escuro. Um player de música que muda de cor com o
    // sistema perde a identidade que ele deveria ter.
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.extreme_bg_color = Color32::from_rgb(0x07, 0x07, 0x09);
    visuals.faint_bg_color = PANEL;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = HOVER;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);

    // Cantos retos em tudo. Arredondamento é o tique visual que faz uma
    // interface parecer template.
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::ZERO;
        widget.bg_fill = PANEL;
        widget.weak_bg_fill = PANEL;
        widget.bg_stroke = Stroke::new(1.0, RULE);
        widget.fg_stroke = Stroke::new(1.0, TEXT);
    }
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, RULE);
    visuals.widgets.hovered.bg_fill = HOVER;
    visuals.widgets.hovered.weak_bg_fill = HOVER;
    visuals.widgets.active.bg_fill = HOVER;
    visuals.widgets.active.weak_bg_fill = HOVER;

    // Sem sombra nenhuma.
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.window_corner_radius = CornerRadius::ZERO;
    visuals.menu_corner_radius = CornerRadius::ZERO;

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 22.0;

    style.text_styles = [
        (TextStyle::Body, body()),
        (TextStyle::Button, body()),
        (TextStyle::Monospace, mono()),
        (TextStyle::Small, small()),
        (
            TextStyle::Heading,
            FontId::new(15.0, FontFamily::Proportional),
        ),
    ]
    .into();

    // Registrado nos dois temas: se algo forçar o claro, o visual não quebra.
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}
