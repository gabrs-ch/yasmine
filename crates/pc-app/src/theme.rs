//! Paleta e estilo.
//!
//! Direção: Apple Music, só que escuro — cantos arredondados, capa presente
//! em todo lugar (lista, player, sidebar), espaçamento confortável. Isso
//! substitui a direção anterior ("software de áudio profissional", cantos
//! retos em tudo): densidade extrema lia como planilha, não como player.
//!
//! O roxo/azul da identidade continua **acento único** — faixa tocando e
//! barra de progresso — mas arredondamento deixou de ser exceção pontual
//! (só o slider de volume) para ser a regra: botões, linhas, capas, sidebar.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

/// Fonte de ícones — [Lucide](https://lucide.dev) (ISC, licença em
/// `assets/LUCIDE-LICENSE.txt`), embutida como qualquer outro asset do
/// binário. Mesma razão dos ícones que ela substituiu (desenhados à mão,
/// linha por linha): o traço não pode depender de o sistema ter, ou não, o
/// glifo certo. Um arquivo só resolve todo ícone da interface — bem mais
/// consistente (e bonito) que reinventar cada forma em `Painter::line`.
const LUCIDE_TTF: &[u8] = include_bytes!("../assets/lucide.ttf");

/// Fontes de texto — [IBM Plex Sans](https://www.ibm.com/plex/) (OFL,
/// `assets/IBM-PLEX-LICENSE.txt`) no lugar da fonte padrão que o `egui` já
/// traz embutida. A padrão existe pra rodar em qualquer lugar sem asset
/// nenhum — mas "roda em qualquer lugar" e "bonita" são objetivos
/// diferentes, e só dá pra ter os dois embutindo a nossa. Plex é humanista
/// (não a neogrotesca genérica): tem calor e desenho próprio sem abrir mão
/// da legibilidade em tamanho de UI. Regular pro corpo, SemiBold pro título
/// de faixa — peso de verdade, não o texto desenhado duas vezes com
/// deslocamento que fazia esse papel antes.
const PLEX_REGULAR_TTF: &[u8] = include_bytes!("../assets/IBMPlexSans-Regular.ttf");
const PLEX_SEMIBOLD_TTF: &[u8] = include_bytes!("../assets/IBMPlexSans-SemiBold.ttf");
/// Números — duração, faixa — em mono. [JetBrains Mono](https://www.jetbrains.com/lp/mono/)
/// (OFL, `assets/JETBRAINS-MONO-LICENSE.txt`; build "NL", sem ligadura de
/// programação — não faz sentido aqui, é só dígito e `:`), no lugar do mono
/// padrão do `egui` pela mesma razão do Plex: parear uma fonte de corpo
/// desenhada com cuidado com um mono qualquer desfaz o cuidado.
const JETBRAINS_MONO_TTF: &[u8] = include_bytes!("../assets/JetBrainsMonoNL-Regular.ttf");

/// Nome da família de fonte reservada aos ícones. Nunca aparece em texto
/// normal — por isso não entra em `style.text_styles`, só é referenciada
/// direto por quem desenha ícone (`icon()`).
fn icon_family() -> FontFamily {
    FontFamily::Name("lucide".into())
}

pub fn icon(size: f32) -> FontId {
    FontId::new(size, icon_family())
}

fn semibold_family() -> FontFamily {
    FontFamily::Name("plex-semibold".into())
}

/// Título de faixa — na lista e na barra do player. Peso de verdade (fonte
/// SemiBold de verdade), não um truque de desenhar duas vezes.
pub fn strong(size: f32) -> FontId {
    FontId::new(size, semibold_family())
}

/// Glifos usados da fonte de ícones, um por nome do Lucide
/// (lucide.dev/icons) — o caractere é o ponto de código que aquele ícone
/// ocupa na fonte, não tem significado fora dela.
pub mod icon_glyph {
    pub const PLAY: char = '\u{e13c}';
    pub const PAUSE: char = '\u{e12e}';
    pub const SKIP_BACK: char = '\u{e15f}';
    pub const SKIP_FORWARD: char = '\u{e160}';
    pub const SHUFFLE: char = '\u{e15e}';
    pub const REPEAT: char = '\u{e146}';
    pub const REPEAT_ONE: char = '\u{e1fd}';
    pub const PICTURE_IN_PICTURE: char = '\u{e3ae}';
    pub const SEARCH: char = '\u{e151}';
    pub const PLUS: char = '\u{e13d}';
    pub const FOLDER: char = '\u{e0d7}';
    pub const REFRESH: char = '\u{e145}';
    // `minimize`/`maximize` dedicados existem no Lucide (setas de canto
    // pra dentro/fora), mas na prática lêem como "entrar/sair de tela
    // cheia" — a convenção universal de SO pra essas duas ações é mesmo o
    // traço e o quadrado simples, e reconhecível vale mais que original.
    pub const MINUS: char = '\u{e11c}';
    pub const SQUARE: char = '\u{e167}';
    pub const X: char = '\u{e1b2}';
}

pub const BG: Color32 = Color32::from_rgb(0x0A, 0x0A, 0x0C);
pub const PANEL: Color32 = Color32::from_rgb(0x0E, 0x0E, 0x12);
pub const HOVER: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x21);
pub const RULE: Color32 = Color32::from_rgb(0x1C, 0x1C, 0x23);
pub const TEXT: Color32 = Color32::from_rgb(0xCB, 0xCB, 0xD4);
pub const DIM: Color32 = Color32::from_rgb(0x6B, 0x6B, 0x78);
pub const FAINT: Color32 = Color32::from_rgb(0x43, 0x43, 0x4E);
/// O acento. Um só, e usado com parcimônia.
pub const ACCENT: Color32 = Color32::from_rgb(0x7C, 0x5C, 0xFF);
/// Ponta escura do gradiente do acento — a barra de progresso preenchida
/// vai de `ACCENT_DIM` a `ACCENT_BRIGHT` em vez de uma cor chapada. Ainda é
/// UM acento (mesmo matiz, só varia luminosidade), não uma paleta nova.
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x59, 0x42, 0xB8);
/// Ponta clara do gradiente, e cor de hover dos botões primários.
pub const ACCENT_BRIGHT: Color32 = Color32::from_rgb(0x9A, 0x82, 0xFF);

/// Raio padrão de arredondamento — linhas da lista, botões, sidebar, capas
/// pequenas. Único número, usado em todo lugar, para o arredondamento não
/// virar um mosaico de valores diferentes.
pub const RADIUS: u8 = 8;
/// Raio menor, para elementos pequenos (miniatura de capa na lista).
pub const RADIUS_SM: u8 = 4;

/// Altura de uma linha da lista. Maior que a densidade "planilha" de antes —
/// espaço suficiente pra capa respirar, no espírito do Apple Music.
pub const ROW_HEIGHT: f32 = 44.0;

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
    // Família nova, não substitui nem entra como fallback das famílias de
    // texto — só quem chama `icon()` explicitamente enxerga essa fonte.
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "lucide",
        egui::FontData::from_static(LUCIDE_TTF),
        vec![egui::epaint::text::InsertFontFamily {
            family: icon_family(),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));

    // Plex Sans e JetBrains Mono entram com prioridade `Highest` nas
    // famílias padrão (`Proportional`/`Monospace`) — não substituem o que o
    // `egui` já registrou ali, ficam na frente. As fontes padrão continuam
    // de reserva pra glifo que o Plex não cobre (emoji, por exemplo), em vez
    // de sumir ou virar um quadrado.
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "plex-regular",
        egui::FontData::from_static(PLEX_REGULAR_TTF),
        vec![egui::epaint::text::InsertFontFamily {
            family: FontFamily::Proportional,
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "plex-semibold",
        egui::FontData::from_static(PLEX_SEMIBOLD_TTF),
        vec![egui::epaint::text::InsertFontFamily {
            family: semibold_family(),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "jetbrains-mono",
        egui::FontData::from_static(JETBRAINS_MONO_TTF),
        vec![egui::epaint::text::InsertFontFamily {
            family: FontFamily::Monospace,
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));

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

    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(RADIUS);
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

    // Sem sombra nenhuma — arredondado não precisa de sombra pra não parecer
    // chapado; o contraste de fundo já faz esse trabalho.
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    // A janela do SO não arredonda (é o gerenciador de janelas quem manda
    // nisso); menus e popups, sim — é onde o Apple Music arredonda também.
    visuals.window_corner_radius = CornerRadius::ZERO;
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 22.0;

    style.text_styles = [
        (TextStyle::Body, body()),
        (TextStyle::Button, body()),
        (TextStyle::Monospace, mono()),
        (TextStyle::Small, small()),
        // O único uso de Heading é o título da faixa tocando — ganha o
        // mesmo peso de verdade do título da lista (`strong`), não só um
        // tamanho maior.
        (TextStyle::Heading, strong(15.0)),
    ]
    .into();

    // Registrado nos dois temas: se algo forçar o claro, o visual não quebra.
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}
