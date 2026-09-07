//! Consultas de leitura para a UI.
//!
//! # A lista nunca carrega a biblioteca
//!
//! Uma "view" é um `Vec<TrackId>` — só os ids, na ordem certa. São 400 KB para
//! 50 000 faixas, contra dezenas de MB se cada linha carregasse título,
//! artista e álbum. A UI desenha ~40 linhas por vez e pede só essas com
//! [`rows`].
//!
//! Nada aqui usa `LIMIT/OFFSET` para paginar: `OFFSET n` faz o SQLite
//! percorrer e descartar n linhas, então rolar até o fim de uma lista grande
//! fica progressivamente mais lento. Buscar por id é O(log n) sempre.

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::types::Value;

use crate::db::{Db, Result};
use crate::model::TrackId;

/// Ordem da lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Artista → ano → álbum → disco → faixa. A ordem em que um álbum foi
    /// feito para ser ouvido.
    #[default]
    ArtistAlbum,
    Title,
    /// Mais recentes primeiro, pelo que o scan viu por último.
    RecentlyAdded,
}

impl Sort {
    const fn order_by(self) -> &'static str {
        match self {
            Self::ArtistAlbum => "aa.name_key, al.year, al.title, t.disc_no, t.track_no, t.title",
            Self::Title => "t.title, aa.name_key",
            Self::RecentlyAdded => "t.scanned_at DESC, t.id DESC",
        }
    }
}

/// Uma linha visível da lista. Só o que cabe na tela — o resto do índice fica
/// no banco.
#[derive(Debug, Clone)]
pub struct TrackRow {
    pub id: TrackId,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_no: Option<u32>,
    pub duration_ms: Option<u64>,
    /// Hash da capa, para montar o caminho da miniatura no cache.
    pub art_hash: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
}

/// Todos os ids da biblioteca, na ordem pedida.
pub fn view(db: &Db, sort: Sort) -> Result<Vec<TrackId>> {
    let sql = format!(
        "SELECT t.id FROM track t
         LEFT JOIN artist aa ON aa.id = t.album_artist_id
         LEFT JOIN album  al ON al.id = t.album_id
         ORDER BY {}",
        sort.order_by()
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let ids = stmt.query_map([], |row| row.get::<_, i64>(0).map(TrackId))?;
    Ok(ids.collect::<rusqlite::Result<_>>()?)
}

/// Ids que casam com `query`, na ordem pedida.
///
/// Busca vazia devolve a biblioteca inteira, que é o que a UI espera quando o
/// usuário apaga o campo.
pub fn search(db: &Db, query: &str, sort: Sort) -> Result<Vec<TrackId>> {
    let Some(match_expr) = fts_query(query) else {
        return view(db, sort);
    };

    let sql = format!(
        "SELECT t.id FROM track_fts f
         JOIN track t ON t.id = f.rowid
         LEFT JOIN artist aa ON aa.id = t.album_artist_id
         LEFT JOIN album  al ON al.id = t.album_id
         WHERE track_fts MATCH ?1
         ORDER BY {}",
        sort.order_by()
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let ids = stmt.query_map([&match_expr], |row| row.get::<_, i64>(0).map(TrackId))?;
    Ok(ids.collect::<rusqlite::Result<_>>()?)
}

/// Traduz o que o usuário digitou numa expressão FTS5 segura.
///
/// Cada palavra vira um termo com prefixo (`palavra*`), e todas precisam
/// aparecer. Digitar "leg urb" acha "Legião Urbana" antes de terminar de
/// escrever — que é o comportamento que faz a busca parecer instantânea.
///
/// As aspas são obrigatórias: sem elas, um `-` ou `:` digitado por acaso é
/// lido como operador do FTS5 e a consulta falha na cara do usuário.
fn fts_query(input: &str) -> Option<String> {
    let mut terms = Vec::new();
    for word in input.split_whitespace() {
        // Aspas dentro do termo são escapadas dobrando, como manda o SQLite.
        let escaped = word.replace('"', "\"\"");
        terms.push(format!("\"{escaped}\"*"));
    }
    if terms.is_empty() {
        return None;
    }
    Some(terms.join(" "))
}

/// Dados de exibição para uma janela de ids, na mesma ordem em que foram
/// pedidos.
///
/// Chamada a cada frame com as ~40 linhas visíveis, então é uma consulta só,
/// com `carray`, em vez de uma por linha.
pub fn rows(db: &Db, ids: &[TrackId]) -> Result<Vec<TrackRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let values: Vec<Value> = ids.iter().map(|id| Value::from(id.0)).collect();
    let mut stmt = db.conn().prepare_cached(
        "SELECT t.id, t.title, ar.name, al.title, t.track_no, t.duration_ms, ca.blob_hash
         FROM track t
         LEFT JOIN artist ar    ON ar.id = t.artist_id
         LEFT JOIN album  al    ON al.id = t.album_id
         LEFT JOIN cover_art ca ON ca.id = t.art_id
         WHERE t.id IN rarray(?1)",
    )?;

    let found = stmt.query_map([std::rc::Rc::new(values)], |row| {
        let hash: Option<Vec<u8>> = row.get(6)?;
        Ok(TrackRow {
            id: TrackId(row.get(0)?),
            title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            artist: row.get(2)?,
            album: row.get(3)?,
            track_no: row.get(4)?,
            duration_ms: row.get::<_, Option<i64>>(5)?.map(|d| d as u64),
            art_hash: hash.and_then(|h| <[u8; 32]>::try_from(h.as_slice()).ok()),
        })
    })?;

    let mut by_id: HashMap<i64, TrackRow> = HashMap::with_capacity(ids.len());
    for row in found {
        let row = row?;
        by_id.insert(row.id.0, row);
    }

    // A consulta volta em ordem arbitrária; quem manda na ordem é a view.
    Ok(ids.iter().filter_map(|id| by_id.remove(&id.0)).collect())
}

/// Caminho absoluto de uma faixa, para entregar ao decodificador.
pub fn track_path(db: &Db, id: TrackId) -> Result<Option<PathBuf>> {
    let mut stmt = db.conn().prepare_cached(
        "SELECT r.path, t.rel_path FROM track t
         JOIN library_root r ON r.id = t.root_id
         WHERE t.id = ?1",
    )?;
    let mut found = stmt.query_map([id.0], |row| {
        Ok(PathBuf::from(row.get::<_, String>(0)?).join(row.get::<_, String>(1)?))
    })?;
    found.next().transpose().map_err(Into::into)
}

pub fn stats(db: &Db) -> Result<Stats> {
    Ok(db.conn().query_row(
        "SELECT (SELECT count(*) FROM track),
                (SELECT count(*) FROM album),
                (SELECT count(*) FROM artist)",
        [],
        |row| {
            Ok(Stats {
                tracks: row.get::<_, i64>(0)? as u64,
                albums: row.get::<_, i64>(1)? as u64,
                artists: row.get::<_, i64>(2)? as u64,
            })
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::art::ArtCache;
    use crate::scan::scan;
    use crate::testutil::{ambiente, escreve, png};

    /// Monta uma biblioteca pequena mas com a forma certa: dois artistas, com
    /// acento e caixa diferentes, e uma capa.
    fn biblioteca(nome: &str) -> (Db, crate::testutil::Ambiente) {
        let env = ambiente(nome);
        let capa = png([20, 60, 180]);
        escreve(
            &env.musica.join("legiao/dois/01.mp3"),
            "Eduardo e Mônica",
            "Legião Urbana",
            "Dois",
            Some(&capa),
        );
        escreve(
            &env.musica.join("legiao/dois/02.mp3"),
            "Índios",
            "Legião Urbana",
            "Dois",
            Some(&capa),
        );
        escreve(
            &env.musica.join("abba/gold/01.mp3"),
            "Waterloo",
            "ABBA",
            "Gold",
            None,
        );

        let mut db = Db::open_in_memory().expect("abrir índice");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("escanear");
        (db, env)
    }

    #[test]
    fn view_devolve_tudo_ordenado_por_artista() {
        let (db, _env) = biblioteca("view");
        let ids = view(&db, Sort::ArtistAlbum).expect("montar view");
        assert_eq!(ids.len(), 3);

        let linhas = rows(&db, &ids).expect("buscar linhas");
        // ABBA antes de Legião: a ordenação usa a chave dobrada, não os bytes
        // crus do nome.
        assert_eq!(linhas[0].artist.as_deref(), Some("ABBA"));
        assert_eq!(linhas[1].artist.as_deref(), Some("Legião Urbana"));
    }

    #[test]
    fn busca_por_prefixo_sem_acento() {
        let (db, _env) = biblioteca("busca");
        // Prefixo incompleto e sem acento: é o que a pessoa digita.
        let ids = search(&db, "leg urb", Sort::ArtistAlbum).expect("buscar");
        assert_eq!(ids.len(), 2);

        let ids = search(&db, "monica", Sort::ArtistAlbum).expect("buscar");
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn busca_vazia_devolve_a_biblioteca_inteira() {
        let (db, _env) = biblioteca("vazia");
        assert_eq!(
            search(&db, "   ", Sort::ArtistAlbum).expect("buscar").len(),
            3
        );
    }

    /// A consulta de janela volta em ordem arbitrária do SQLite; quem manda na
    /// ordem é a view, senão a lista embaralha ao rolar.
    #[test]
    fn rows_respeita_a_ordem_dos_ids_pedidos() {
        let (db, _env) = biblioteca("ordem");
        let ids = view(&db, Sort::ArtistAlbum).expect("view");

        let invertido: Vec<_> = ids.iter().copied().rev().collect();
        let linhas = rows(&db, &invertido).expect("linhas");

        let devolvidos: Vec<_> = linhas.iter().map(|r| r.id).collect();
        assert_eq!(devolvidos, invertido);
    }

    #[test]
    fn rows_busca_so_a_janela_pedida() {
        let (db, _env) = biblioteca("janela");
        let ids = view(&db, Sort::ArtistAlbum).expect("view");
        let linhas = rows(&db, &ids[..1]).expect("linhas");
        assert_eq!(linhas.len(), 1);
    }

    #[test]
    fn linha_carrega_o_hash_da_capa_quando_existe() {
        let (db, _env) = biblioteca("capa");
        let ids = view(&db, Sort::ArtistAlbum).expect("view");
        let linhas = rows(&db, &ids).expect("linhas");

        // ABBA veio sem capa; as duas da Legião compartilham a mesma.
        assert!(linhas[0].art_hash.is_none());
        assert_eq!(linhas[1].art_hash, linhas[2].art_hash);
        assert!(linhas[1].art_hash.is_some());
    }

    #[test]
    fn track_path_aponta_para_um_arquivo_existente() {
        let (db, _env) = biblioteca("caminho");
        let ids = view(&db, Sort::ArtistAlbum).expect("view");
        let path = track_path(&db, ids[0])
            .expect("consultar caminho")
            .expect("faixa existe");
        assert!(path.is_file(), "{} não existe", path.display());
    }

    #[test]
    fn stats_conta_faixas_albuns_e_artistas() {
        let (db, _env) = biblioteca("stats");
        let s = stats(&db).expect("stats");
        assert_eq!(s.tracks, 3);
        assert_eq!(s.albums, 2);
        assert_eq!(s.artists, 2);
    }

    #[test]
    fn termos_viram_prefixos_com_e_logico() {
        assert_eq!(fts_query("leg urb").as_deref(), Some("\"leg\"* \"urb\"*"));
    }

    #[test]
    fn busca_vazia_nao_gera_expressao() {
        assert_eq!(fts_query("   "), None);
        assert_eq!(fts_query(""), None);
    }

    /// Sem escape, um `-` ou aspas digitados por acaso viram sintaxe do FTS5 e
    /// a busca explode enquanto o usuário digita.
    #[test]
    fn caracteres_de_sintaxe_nao_escapam_do_termo() {
        assert_eq!(fts_query("a-b").as_deref(), Some("\"a-b\"*"));
        assert_eq!(fts_query("x\"y").as_deref(), Some("\"x\"\"y\"*"));
        assert_eq!(fts_query("OR").as_deref(), Some("\"OR\"*"));
    }
}
