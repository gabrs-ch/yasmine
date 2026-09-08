//! Hash de conteúdo das faixas.
//!
//! O `content_hash` é a identidade de uma faixa **entre devices**: `track.id`
//! é autoincrement local, então o mesmo arquivo tem id diferente em cada
//! máquina. Tudo que precisa sobreviver ao sync — item de playlist, contagem
//! de plays, rating — aponta para o hash.
//!
//! # Por que preguiçoso
//!
//! Hashear é ler o arquivo inteiro. Fazer isso no scan multiplicaria o custo
//! da primeira varredura por dez sem que ninguém tivesse pedido sync ainda.
//! Então a coluna nasce `NULL` e só é preenchida quando alguém precisa: uma
//! faixa entrando numa playlist, ou o sync comparando bibliotecas.
//!
//! O hash cobre só o **áudio como bytes do arquivo**, incluindo as tags. Uma
//! edição de metadata muda o hash — o que é aceitável aqui porque a playlist
//! guarda o hash conhecido no momento e o sync reconcilia por conteúdo.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use rayon::prelude::*;

use crate::db::{Db, Error, Result};
use crate::model::{TrackId, TrackKey};

/// Blocos de 128 KiB: grande o bastante para o BLAKE3 render, pequeno o
/// bastante para não pesar na memória com vários arquivos em paralelo.
const CHUNK: usize = 128 * 1024;

fn hash_file(path: &PathBuf) -> Option<TrackKey> {
    let mut file = File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buffer[..n]);
            }
            Err(_) => return None,
        }
    }
    Some(TrackKey(hasher.finalize().into()))
}

/// Garante que `ids` tenham hash calculado e gravado, e devolve o mapa.
///
/// Faixas já hasheadas não são relidas. As que faltam são hasheadas em
/// paralelo e gravadas numa transação só. Ids cujo arquivo sumiu ficam de
/// fora do mapa em vez de derrubar a operação.
pub fn ensure_hashes(db: &mut Db, ids: &[TrackId]) -> Result<HashMap<TrackId, TrackKey>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let values: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|id| rusqlite::types::Value::from(id.0))
        .collect();

    let mut known = HashMap::with_capacity(ids.len());
    let mut missing: Vec<(TrackId, PathBuf)> = Vec::new();
    {
        let mut stmt = db.conn().prepare_cached(
            "SELECT t.id, t.content_hash, r.path, t.rel_path
             FROM track t
             JOIN library_root r ON r.id = t.root_id
             WHERE t.id IN rarray(?1)",
        )?;
        let rows = stmt.query_map([std::rc::Rc::new(values)], |row| {
            Ok((
                TrackId(row.get(0)?),
                row.get::<_, Option<Vec<u8>>>(1)?,
                PathBuf::from(row.get::<_, String>(2)?).join(row.get::<_, String>(3)?),
            ))
        })?;

        for row in rows {
            let (id, hash, path) = row?;
            match hash.and_then(|h| <[u8; 32]>::try_from(h.as_slice()).ok()) {
                Some(hash) => {
                    known.insert(id, TrackKey(hash));
                }
                None => missing.push((id, path)),
            }
        }
    }

    if missing.is_empty() {
        return Ok(known);
    }

    // Ler arquivos é I/O; vale paralelizar mesmo quando são poucos.
    let computed: Vec<(TrackId, TrackKey)> = missing
        .par_iter()
        .filter_map(|(id, path)| hash_file(path).map(|key| (*id, key)))
        .collect();

    let tx = db.conn_mut().transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE track SET content_hash = ?2 WHERE id = ?1")?;
        for (id, key) in &computed {
            stmt.execute(rusqlite::params![id.0, key.0.as_slice()])?;
        }
    }
    tx.commit().map_err(Error::from)?;

    known.extend(computed);
    Ok(known)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::art::ArtCache;
    use crate::scan::scan;
    use crate::testutil::{ambiente, escreve};

    fn biblioteca(nome: &str) -> (Db, crate::testutil::Ambiente) {
        let env = ambiente(nome);
        escreve(&env.musica.join("a/1.mp3"), "Um", "Artista", "Álbum", None);
        escreve(
            &env.musica.join("a/2.mp3"),
            "Dois",
            "Artista",
            "Álbum",
            None,
        );

        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("escanear");
        (db, env)
    }

    fn ids(db: &Db) -> Vec<TrackId> {
        crate::library::view(db, crate::library::Sort::ArtistAlbum).expect("view")
    }

    #[test]
    fn o_scan_nao_hasheia_nada() {
        let (db, _env) = biblioteca("preguicoso");
        let pendentes: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM track WHERE content_hash IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("consultar");
        assert_eq!(pendentes, 2, "o scan hasheou sem precisar");
    }

    #[test]
    fn calcula_e_grava_o_hash() {
        let (mut db, _env) = biblioteca("calcula");
        let ids = ids(&db);

        let mapa = ensure_hashes(&mut db, &ids).expect("hashear");
        assert_eq!(mapa.len(), 2);

        let pendentes: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM track WHERE content_hash IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("consultar");
        assert_eq!(pendentes, 0, "o hash não foi gravado");
    }

    #[test]
    fn segunda_chamada_nao_rele_os_arquivos() {
        let (mut db, env) = biblioteca("cache");
        let ids = ids(&db);
        let primeiro = ensure_hashes(&mut db, &ids).expect("hashear");

        // Apagar os arquivos: se ele fosse reler, falharia.
        std::fs::remove_dir_all(env.musica.join("a")).expect("apagar");
        let segundo = ensure_hashes(&mut db, &ids).expect("hashear de novo");

        assert_eq!(primeiro, segundo);
    }

    #[test]
    fn arquivos_diferentes_tem_hashes_diferentes() {
        let (mut db, _env) = biblioteca("distintos");
        let ids = ids(&db);
        let mapa = ensure_hashes(&mut db, &ids).expect("hashear");

        let chaves: Vec<_> = mapa.values().collect();
        assert_ne!(chaves[0], chaves[1]);
    }

    #[test]
    fn lista_vazia_nao_toca_no_banco() {
        let (mut db, _env) = biblioteca("vazio");
        assert!(ensure_hashes(&mut db, &[]).expect("vazio").is_empty());
    }
}
