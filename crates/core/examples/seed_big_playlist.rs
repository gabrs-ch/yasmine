//! Prepara o índice real do app (mesmo caminho que `Paths::resolve` usa) com
//! a biblioteca de 50 mil faixas escaneada e uma playlist "Tudo" com todas
//! elas — para medir a app de verdade com uma playlist grande sem precisar
//! clicar 50 mil vezes.
//!
//! ```text
//! cargo run --release -p player-core --example seed_big_playlist -- \
//!     ~/.local/share/yasmine/library.db ./testdata/lib50k
//! ```

use std::path::PathBuf;

use player_core::library::{self, Sort};
use player_core::{ArtCache, Db, playlist, scan};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let db_path = PathBuf::from(args.next().ok_or("uso: seed_big_playlist <db> <pasta>")?);
    let root = PathBuf::from(args.next().ok_or("uso: seed_big_playlist <db> <pasta>")?);

    let mut db = Db::open(&db_path)?;
    let art = ArtCache::new(root.parent().unwrap_or(&root).join("player-seed-cache"));
    let report = scan(&mut db, &root, &art)?;
    println!("{report:#?}");

    let tracks = library::view(&db, Sort::ArtistAlbum)?;
    let id = playlist::create(&db, "Tudo")?;
    let added = playlist::append(&mut db, id, &tracks)?;
    println!("playlist \"Tudo\" com {added} faixas");

    Ok(())
}
