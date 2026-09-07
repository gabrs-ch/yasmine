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

use player_core::library::{self, Sort};
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

    // O que a UI faz de fato: montar a view, buscar, e pedir só a janela
    // visível. Se algum destes não for instantâneo, a lista trava ao digitar.
    let t = Instant::now();
    let ids = library::view(&db, Sort::ArtistAlbum)?;
    println!(
        "\nview      {:>6.1} ms  ({} ids, {} KB)",
        t.elapsed().as_secs_f64() * 1000.0,
        ids.len(),
        ids.len() * size_of::<player_core::TrackId>() / 1024
    );

    let t = Instant::now();
    let achados = library::search(&db, "azul cor", Sort::ArtistAlbum)?;
    println!(
        "busca     {:>6.1} ms  ({} resultados)",
        t.elapsed().as_secs_f64() * 1000.0,
        achados.len()
    );

    let janela = &ids[ids.len() / 2..(ids.len() / 2 + 40).min(ids.len())];
    let t = Instant::now();
    let linhas = library::rows(&db, janela)?;
    println!(
        "janela    {:>6.1} ms  ({} linhas, do meio da lista)",
        t.elapsed().as_secs_f64() * 1000.0,
        linhas.len()
    );

    Ok(())
}
