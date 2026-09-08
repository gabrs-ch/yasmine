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

fn main() -> eframe::Result<()> {
    // Pasta opcional na linha de comando: abre e escaneia direto. Serve pra
    // "abrir com" do gerenciador de arquivos e pra subir o app já apontado
    // numa biblioteca.
    let folder = std::env::args().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 660.0])
            .with_min_inner_size([620.0, 380.0])
            .with_title("Player"),
        // glow, não wgpu: o contexto sobe mais rápido e o binário carrega
        // menos dependência. Aparece direto no tempo até a janela existir.
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "player",
        options,
        Box::new(move |cc| match app::App::new(cc, folder) {
            Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
            Err(err) => Err(err.into()),
        }),
    )
}
