//! Player de música para PC — casca Tauri.
//!
//! A UI é web (`ui/`, React) servida pelo webview do SO; o Rust é o back:
//! índice (`player-core`), áudio (`player-audio`) e a ponte de comandos.
//! Migração de `egui` documentada em `../../design/` e no plano do branch
//! `tauri-ui`.

// Sem console no Windows quando aberto pelo explorador.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod art_protocol;
mod commands;
mod dto;
mod hexhash;
mod paths;
mod scan;
mod state;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Manager;

use state::AppState;

fn main() {
    let app_state = AppState::load().unwrap_or_else(|err| {
        eprintln!("Yasmine: {err}");
        std::process::exit(1);
    });
    let cache_dir = app_state.paths.cache.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(app_state))
        .register_uri_scheme_protocol("art", art_protocol::handler(cache_dir))
        .setup(|app| {
            // Um argumento de pasta na linha de comando aponta a biblioteca
            // já na subida (atalho, `cargo tauri dev -- <pasta>`, e a base do
            // "abrir com" da Fase 4). Arquivos soltos: Fase 4.
            if let Some(dir) = std::env::args_os()
                .skip(1)
                .map(PathBuf::from)
                .find(|p| p.is_dir())
            {
                let handle = app.handle().clone();
                app.state::<Mutex<AppState>>()
                    .lock()
                    .expect("estado do app")
                    .set_root(&handle, dir);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::library_stats,
            commands::current_root,
            commands::list_playlists,
            commands::list_artists,
            commands::open_source,
            commands::track_rows,
            commands::pick_folder,
            commands::rescan,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o Yasmine");
}
