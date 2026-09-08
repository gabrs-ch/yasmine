//! Player de música para PC.

// Sem console no Windows quando o binário é aberto pelo explorador.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod art;
mod paths;
mod queue;
mod theme;
mod watcher;

use eframe::egui;

/// A marca do Yasmine — arte fornecida pelo usuário, PNG único. Embutida no
/// binário: é um ícone, não algo que o usuário troca, então não precisa
/// viver solto no disco.
fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/icon-256.png");
    let image = image::load_from_memory(bytes)
        .expect("assets/icon-256.png embutido no binário deveria ser válido")
        .to_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result<()> {
    // Pasta opcional na linha de comando: abre e escaneia direto. Serve pra
    // "abrir com" do gerenciador de arquivos e pra subir o app já apontado
    // numa biblioteca.
    let folder = std::env::args().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 660.0])
            .with_min_inner_size(app::NORMAL_MIN_SIZE)
            .with_title("Yasmine")
            .with_icon(load_icon())
            // Sem decoração do gerenciador de janelas: no Linux, cada
            // ambiente desenha um cabeçalho diferente (cinza claro no
            // XFCE, escuro em outros) que nada no app controla — colado
            // direto num conteúdo quase preto, era a costura mais feia da
            // janela inteira. A barra de comando (`app::App::top_bar`) vira
            // a barra de título também: arrasta, dá duplo clique pra
            // maximizar, tem os próprios botões de minimizar/maximizar/
            // fechar, no mesmo traço do resto da interface.
            .with_decorations(false),
        // glow, não wgpu: o contexto sobe mais rápido e o binário carrega
        // menos dependência. Aparece direto no tempo até a janela existir.
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "yasmine",
        options,
        Box::new(move |cc| match app::App::new(cc, folder) {
            Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
            Err(err) => Err(err.into()),
        }),
    )
}
