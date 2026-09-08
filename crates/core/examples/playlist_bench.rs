//! Mede o custo de uma playlist grande: hash sob demanda de 50 mil faixas de
//! uma vez, e o tempo de ler a playlist de volta.
//!
//! O caso realista não é "playlist com 50 mil músicas" — é "o usuário
//! arrasta o álbum inteiro, ou a biblioteca toda, pra uma playlist pela
//! primeira vez", que dispara o hash de tudo que ainda não tinha hash. É
//! esse pico que este exemplo mede.
//!
//! ```text
//! cargo run --release -p player-core --example playlist_bench -- ./testdata/lib50k
//! ```

use std::path::PathBuf;
use std::time::Instant;

use player_core::library::{self, Sort};
use player_core::{ArtCache, Db, playlist, scan};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("uso: playlist_bench <pasta>")?,
    );

    let mut db = Db::open_in_memory()?;
    let art = ArtCache::new(std::env::temp_dir().join("player-playlist-bench-cache"));
    scan(&mut db, &root, &art)?;

    let tracks = library::view(&db, Sort::ArtistAlbum)?;
    println!("{} faixas indexadas", tracks.len());

    let id = playlist::create(&db, "Tudo")?;

    let t = Instant::now();
    let added = playlist::append(&mut db, id, &tracks)?;
    println!(
        "append (com hash de tudo) {:>7.0} ms  ({added} faixas)",
        t.elapsed().as_secs_f64() * 1000.0
    );

    let t = Instant::now();
    let items = playlist::tracks(&db, id)?;
    println!(
        "tracks (já hasheado)      {:>7.1} ms  ({} faixas)",
        t.elapsed().as_secs_f64() * 1000.0,
        items.len()
    );

    // Uma segunda playlist com as mesmas faixas: hash já está gravado, então
    // isto é o caso comum de arrastar mais álbuns depois do primeiro.
    let id2 = playlist::create(&db, "Tudo de novo")?;
    let t = Instant::now();
    playlist::append(&mut db, id2, &tracks)?;
    println!(
        "append (hash já pronto)   {:>7.1} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );

    let t = Instant::now();
    let all = playlist::all(&db)?;
    println!(
        "listar playlists          {:>7.2} ms  ({} playlists)",
        t.elapsed().as_secs_f64() * 1000.0,
        all.len()
    );

    Ok(())
}
