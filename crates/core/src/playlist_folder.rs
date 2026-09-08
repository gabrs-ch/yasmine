//! Playlist vinculada a uma pasta.
//!
//! Uma playlist normal só ganha faixa quando alguém arrasta uma por vez.
//! Vinculada a uma pasta, toda faixa que está (ou vier a entrar) dentro dela
//! passa a fazer parte da playlist sozinha — o caso de uso é "essa pasta É a
//! playlist" (um álbum, uma pasta de treino que a pessoa preenche por fora
//! do player).
//!
//! Guardado por (raiz, prefixo relativo), não por caminho absoluto — mesmo
//! raciocínio de `track.rel_path`: mover a raiz inteira de lugar não invalida
//! o vínculo. [`sync_all`] roda depois de cada scan e é idempotente:
//! acrescenta só o que ainda não está na playlist, nunca duplica.

use std::path::Path;

use uuid::Uuid;

use crate::db::{Db, Result};
use crate::model::TrackId;
use crate::playlist;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub root_id: i64,
    /// Caminho da pasta, relativo à raiz, com `/`. Vazio = a raiz inteira.
    pub rel_prefix: String,
}

/// A pasta escolhida não está dentro de nenhuma raiz da biblioteca — não há
/// como expressá-la como (raiz, prefixo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForaDaBiblioteca;

/// Vincula `playlist_id` à pasta `folder`, resolvendo-a contra as raízes
/// conhecidas.
pub fn link(
    db: &Db,
    playlist_id: Uuid,
    folder: &Path,
) -> Result<std::result::Result<(), ForaDaBiblioteca>> {
    let Some((root_id, rel_prefix)) = resolve(db, folder)? else {
        return Ok(Err(ForaDaBiblioteca));
    };
    db.conn().execute(
        "INSERT INTO playlist_folder (playlist_id, root_id, rel_prefix) VALUES (?1, ?2, ?3)
         ON CONFLICT DO NOTHING",
        rusqlite::params![playlist_id.as_bytes().as_slice(), root_id, rel_prefix],
    )?;
    Ok(Ok(()))
}

pub fn unlink(db: &Db, playlist_id: Uuid, root_id: i64, rel_prefix: &str) -> Result<()> {
    db.conn().execute(
        "DELETE FROM playlist_folder WHERE playlist_id = ?1 AND root_id = ?2 AND rel_prefix = ?3",
        rusqlite::params![playlist_id.as_bytes().as_slice(), root_id, rel_prefix],
    )?;
    Ok(())
}

/// Pastas vinculadas a `playlist_id`, para mostrar/desvincular na UI.
pub fn links_for(db: &Db, playlist_id: Uuid) -> Result<Vec<Link>> {
    let mut stmt = db.conn().prepare(
        "SELECT root_id, rel_prefix FROM playlist_folder
         WHERE playlist_id = ?1 ORDER BY rel_prefix",
    )?;
    let rows = stmt.query_map([playlist_id.as_bytes().as_slice()], |row| {
        Ok(Link {
            root_id: row.get(0)?,
            rel_prefix: row.get(1)?,
        })
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

/// Acha em qual raiz da biblioteca `folder` está, e o prefixo relativo a ela.
/// `None` quando a pasta não está sob raiz nenhuma.
fn resolve(db: &Db, folder: &Path) -> Result<Option<(i64, String)>> {
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());

    let mut stmt = db.conn().prepare("SELECT id, path FROM library_root")?;
    let roots: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    for (id, path) in roots {
        let root_path = Path::new(&path)
            .canonicalize()
            .unwrap_or_else(|_| Path::new(&path).to_path_buf());
        if let Ok(rel) = folder.strip_prefix(&root_path) {
            let rel_prefix = rel
                .components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join("/");
            return Ok(Some((id, rel_prefix)));
        }
    }
    Ok(None)
}

/// Sincroniza todos os vínculos: acrescenta às playlists as faixas que
/// deveriam estar lá e ainda não estão. Devolve quantas entraram no total.
///
/// Chamada depois de cada scan (manual ou pelo vigia de pasta) — é assim que
/// um arquivo novo dentro de uma pasta vinculada aparece na playlist sem o
/// usuário fazer nada.
pub fn sync_all(db: &mut Db) -> Result<usize> {
    let mut stmt = db.conn().prepare(
        "SELECT f.playlist_id, f.root_id, f.rel_prefix
         FROM playlist_folder f
         JOIN playlist p ON p.id = f.playlist_id
         WHERE p.deleted = 0",
    )?;
    let links: Vec<(Vec<u8>, i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    let mut total = 0usize;
    for (raw_id, root_id, rel_prefix) in links {
        let Ok(bytes) = <[u8; 16]>::try_from(raw_id.as_slice()) else {
            continue;
        };
        total += sync_one(db, Uuid::from_bytes(bytes), root_id, &rel_prefix)?;
    }
    Ok(total)
}

fn sync_one(db: &mut Db, playlist_id: Uuid, root_id: i64, rel_prefix: &str) -> Result<usize> {
    let mut stmt = db
        .conn()
        .prepare("SELECT id, rel_path FROM track WHERE root_id = ?1")?;
    let rows: Vec<(i64, String)> = stmt
        .query_map(rusqlite::params![root_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    let prefix_slash = format!("{rel_prefix}/");
    let candidates: Vec<TrackId> = rows
        .into_iter()
        .filter(|(_, rel_path)| {
            rel_prefix.is_empty() || rel_path == rel_prefix || rel_path.starts_with(&prefix_slash)
        })
        .map(|(id, _)| TrackId(id))
        .collect();

    if candidates.is_empty() {
        return Ok(0);
    }

    let already: std::collections::HashSet<TrackId> =
        playlist::tracks(db, playlist_id)?.into_iter().collect();
    let new: Vec<TrackId> = candidates
        .into_iter()
        .filter(|id| !already.contains(id))
        .collect();

    playlist::append(db, playlist_id, &new)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::art::ArtCache;
    use crate::scan::scan;
    use crate::testutil::{ambiente, escreve};

    /// Uma pasta com duas subpastas, cada uma com faixas — para vincular só
    /// uma delas e confirmar que a outra fica de fora.
    fn biblioteca(nome: &str) -> (Db, crate::testutil::Ambiente) {
        let env = ambiente(nome);
        for n in 0..3 {
            escreve(
                &env.musica.join(format!("Treino/{n:02}.mp3")),
                &format!("Treino {n:02}"),
                "Artista",
                "Álbum",
                None,
            );
        }
        escreve(
            &env.musica.join("Calma/01.mp3"),
            "Calma 01",
            "Artista",
            "Álbum",
            None,
        );
        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("escanear");
        (db, env)
    }

    #[test]
    fn vincular_traz_so_as_faixas_da_pasta() {
        let (mut db, env) = biblioteca("vinculo");
        let id = playlist::create(&db, "Treino").expect("criar playlist");

        link(&db, id, &env.musica.join("Treino"))
            .expect("resolver")
            .expect("dentro da biblioteca");
        sync_all(&mut db).expect("sincronizar");

        assert_eq!(playlist::tracks(&db, id).expect("faixas").len(), 3);
    }

    /// Rodar de novo sem nada novo na pasta não duplica os itens.
    #[test]
    fn sincronizar_de_novo_nao_duplica() {
        let (mut db, env) = biblioteca("idempotente");
        let id = playlist::create(&db, "Treino").expect("criar playlist");

        link(&db, id, &env.musica.join("Treino"))
            .expect("resolver")
            .expect("dentro da biblioteca");
        sync_all(&mut db).expect("primeira sincronização");
        sync_all(&mut db).expect("segunda sincronização");

        assert_eq!(playlist::tracks(&db, id).expect("faixas").len(), 3);
    }

    /// Faixa nova na pasta vinculada, depois de um rescan, entra sozinha.
    #[test]
    fn faixa_nova_na_pasta_entra_depois_do_proximo_scan() {
        let (mut db, env) = biblioteca("faixa-nova");
        let id = playlist::create(&db, "Treino").expect("criar playlist");
        link(&db, id, &env.musica.join("Treino"))
            .expect("resolver")
            .expect("dentro da biblioteca");
        sync_all(&mut db).expect("sincronizar");
        assert_eq!(playlist::tracks(&db, id).expect("faixas").len(), 3);

        escreve(
            &env.musica.join("Treino/99.mp3"),
            "Treino 99",
            "Artista",
            "Álbum",
            None,
        );
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("reescanear");
        sync_all(&mut db).expect("sincronizar de novo");

        assert_eq!(playlist::tracks(&db, id).expect("faixas").len(), 4);
    }

    #[test]
    fn pasta_fora_da_biblioteca_nao_vincula() {
        let (db, env) = biblioteca("fora");
        let id = playlist::create(&db, "Treino").expect("criar playlist");

        let fora = env.musica.parent().expect("pai").join("outra-pasta-qualquer");
        assert_eq!(link(&db, id, &fora).expect("resolver"), Err(ForaDaBiblioteca));
        assert!(links_for(&db, id).expect("vínculos").is_empty());
    }

    #[test]
    fn desvincular_para_a_sincronizacao_de_trazer_faixa_nova() {
        let (mut db, env) = biblioteca("desvinculo");
        let id = playlist::create(&db, "Treino").expect("criar playlist");
        link(&db, id, &env.musica.join("Treino"))
            .expect("resolver")
            .expect("dentro da biblioteca");
        sync_all(&mut db).expect("sincronizar");

        let vinculo = links_for(&db, id).expect("vínculos").remove(0);
        unlink(&db, id, vinculo.root_id, &vinculo.rel_prefix).expect("desvincular");
        assert!(links_for(&db, id).expect("vínculos").is_empty());

        escreve(
            &env.musica.join("Treino/99.mp3"),
            "Treino 99",
            "Artista",
            "Álbum",
            None,
        );
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("reescanear");
        sync_all(&mut db).expect("sincronizar de novo");

        // Sem vínculo, a faixa nova não entra sozinha.
        assert_eq!(playlist::tracks(&db, id).expect("faixas").len(), 3);
    }
}
