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
mod loudness;
mod paths;
mod playback;
mod queue;
mod scan;
mod state;
mod sync_host;
mod watcher;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Manager;

use state::AppState;

fn main() {
    linux_webkit_workarounds();

    let app_state = AppState::load().unwrap_or_else(|err| {
        eprintln!("Yasmine: {err}");
        std::process::exit(1);
    });
    let cache_dir = app_state.paths.cache.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(app_state))
        .manage(sync_host::SyncHost::default())
        .register_uri_scheme_protocol("art", art_protocol::handler(cache_dir))
        .setup(|app| {
            let handle = app.handle().clone();

            // Linha de comando: uma pasta aponta a biblioteca já na subida
            // (atalho, `cargo tauri dev -- <pasta>`); um ou mais arquivos são
            // o "abrir com" do gerenciador — aponta a biblioteca pra pasta
            // deles e toca depois que o scan indexar (`flush_pending_play`).
            let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
            let dir = args.iter().find(|p| p.is_dir()).cloned();
            let files: Vec<PathBuf> = args.iter().filter(|p| p.is_file()).cloned().collect();
            {
                let state = app.state::<Mutex<AppState>>();
                let mut st = state.lock().expect("estado do app");
                if let Some(d) = dir {
                    st.set_root(&handle, d);
                } else if let Some(parent) = files.first().and_then(|f| f.parent()) {
                    let parent = parent.to_path_buf();
                    st.pending_play = files;
                    st.set_root(&handle, parent);
                }
            }

            // Loop que bombeia os eventos do motor de áudio e transmite o
            // `playback://state`.
            playback::spawn(handle);
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
            commands::play_at,
            commands::play_pause,
            commands::next_track,
            commands::prev_track,
            commands::seek,
            commands::set_volume,
            commands::set_shuffle,
            commands::cycle_repeat,
            commands::playback_snapshot,
            commands::flush_pending_play,
            commands::playlist_create,
            commands::playlist_rename,
            commands::playlist_delete,
            commands::playlist_add_tracks,
            commands::playlist_remove_at,
            commands::playlist_move,
            commands::playlist_set_image,
            commands::playlist_clear_image,
            commands::library_set_image,
            commands::library_clear_image,
            commands::library_image,
            commands::track_set_album_art,
            commands::playlist_links,
            commands::playlist_link_folder,
            commands::playlist_unlink_folder,
            commands::artist_set_image,
            commands::artist_clear_image,
            commands::sync_start,
            commands::sync_stop,
            commands::sync_info,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o Yasmine");
}

/// O renderizador DMABUF do WebKitGTK 2.4x quebra em muitas combinações de
/// mesa/driver — e mais ainda dentro de um AppImage, onde libs empacotadas
/// convivem com o `libGL`/`libEGL`/`libgbm` do host (`WebKitWebProcess`
/// aborta com SIGABRT já na subida, visto em Fedora/Nobara). Desligá-lo cai
/// num caminho de composição estável; a diferença de desempenho é
/// imperceptível numa UI de player. Também desliga a aceleração quando não
/// há GPU utilizável (VM).
fn linux_webkit_workarounds() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: primeira linha do `main`, antes de qualquer thread, GTK ou
        // FFI — não há leitura concorrente do ambiente.
        #[allow(unsafe_code)]
        unsafe {
            if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
                std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            }
        }
    }
}
