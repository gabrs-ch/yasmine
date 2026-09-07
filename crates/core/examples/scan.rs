//! Mede um scan de verdade contra uma pasta de verdade.
//!
//! ```text
//! cargo run --release -p player-core --example scan -- ./testdata/lib50k
//! ```
//!
//! Rodar duas vezes é o teste que importa: a segunda passada tem que ser quase
//! instantânea, porque nenhum arquivo é aberto.

use std::path::PathBuf;
use std::time::Instant;

use player_core::{ArtCache, Db, scan};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().ok_or("uso: scan <pasta> [índice.db]")?);
    let db_path = args.next().map(PathBuf::from);

    let cache = std::env::temp_dir().join("player-bench-cache");
    let mut db = match &db_path {
        Some(path) => Db::open(path)?,
        None => Db::open_in_memory()?,
    };
    let art = ArtCache::new(cache.clone());

    let started = Instant::now();
    let report = scan(&mut db, &root, &art)?;
    let elapsed = started.elapsed();

    let total = report.added + report.updated + report.unchanged;
    println!("{report:#?}");
    println!(
        "{total} faixas em {:.2}s ({:.0} faixas/s)",
        elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64()
    );
    println!("cache de capas: {}", cache.display());

    Ok(())
}
