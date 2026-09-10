//! O scan roda numa thread própria, com a **sua** conexão SQLite (WAL cuida
//! da concorrência com a conexão dos comandos). Progresso e fim viram eventos
//! Tauri; o front reidrata a lista quando recebe `scan://done`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use player_core::scan::scan_with_progress;
use player_core::{ArtCache, Db};

use crate::loudness;

#[derive(Clone, Serialize)]
struct Progress {
    /// Arquivos processados até agora (o total só é conhecido no meio do
    /// scan, então isto é um contador, não uma fração).
    done: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Done {
    added: usize,
    updated: usize,
    removed: usize,
    unchanged: usize,
    elapsed_s: f64,
}

pub fn spawn(
    app: AppHandle,
    db_path: PathBuf,
    cache: PathBuf,
    root: PathBuf,
    loudness_running: Arc<AtomicBool>,
) {
    let _ = std::thread::Builder::new()
        .name("scan".into())
        .spawn(move || {
            let progress = Arc::new(AtomicUsize::new(0));
            let started = Instant::now();

            // Emissor de progresso: lê o átomo a cada 150 ms até o scan acabar.
            let running = Arc::new(AtomicBool::new(true));
            let emitter = {
                let (app, progress, running) =
                    (app.clone(), Arc::clone(&progress), Arc::clone(&running));
                std::thread::spawn(move || {
                    while running.load(Ordering::Relaxed) {
                        let _ = app.emit(
                            "scan://progress",
                            Progress {
                                done: progress.load(Ordering::Relaxed),
                            },
                        );
                        std::thread::sleep(Duration::from_millis(150));
                    }
                })
            };

            let outcome = Db::open(&db_path)
                .map_err(|e| e.to_string())
                .and_then(|mut db| {
                    let art = ArtCache::new(cache);
                    let report = scan_with_progress(&mut db, &root, &art, &progress)
                        .map_err(|e| e.to_string())?;
                    // Playlists de pasta vinculada: as faixas novas entram sem
                    // outro clique. Barato — só consultas contra os vínculos.
                    let _ = player_core::playlist_folder::sync_all(&mut db);
                    Ok(report)
                });

            running.store(false, Ordering::Relaxed);
            let _ = emitter.join();

            match outcome {
                Ok(r) => {
                    let _ = app.emit(
                        "scan://done",
                        Done {
                            added: r.added,
                            updated: r.updated,
                            removed: r.removed,
                            unchanged: r.unchanged,
                            elapsed_s: started.elapsed().as_secs_f64(),
                        },
                    );
                    // O índice está pronto; o ganho de loudness enche depois,
                    // sem segurar o `scan://done`.
                    loudness::spawn_fill(db_path, loudness_running);
                }
                Err(e) => {
                    let _ = app.emit("scan://error", e);
                }
            }
        });
}
