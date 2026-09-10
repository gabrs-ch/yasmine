//! Preenche o ganho de loudness (ReplayGain) das faixas que ainda não têm.
//! Decodifica o áudio inteiro de cada faixa — caro, então roda numa thread à
//! parte, disparada depois de cada scan e idempotente (não empilha).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use player_core::Db;

pub fn spawn_fill(db_path: PathBuf, running: Arc<AtomicBool>) {
    if running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("nivelador".into())
        .spawn(move || {
            run(&db_path);
            running.store(false, Ordering::Release);
        });
}

fn run(db_path: &Path) {
    const BATCH: usize = 16;
    let Ok(db) = Db::open(db_path) else { return };
    loop {
        let Ok(batch) = player_core::loudness::pending(&db, BATCH) else {
            return;
        };
        if batch.is_empty() {
            return;
        }
        for item in batch {
            let loudness =
                player_audio::loudness::analyze(&item.path).unwrap_or(player_audio::Loudness {
                    gain_db: 0.0,
                    peak: 1.0,
                });
            let _ = player_core::loudness::set(&db, item.id, loudness.gain_db, loudness.peak);
        }
    }
}
