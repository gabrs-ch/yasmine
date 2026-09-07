//! Varredura da pasta e indexação incremental.
//!
//! # O caminho rápido é não abrir o arquivo
//!
//! A parte cara de um scan não é o SQLite — é abrir 50 000 arquivos e fazer
//! parse de tag. Por isso a primeira coisa que acontece com cada arquivo é a
//! comparação de `(tamanho, mtime)` com o que o índice já tem: bateu, a faixa
//! é dada como inalterada e nem é aberta. Um rescan de biblioteca intacta vira
//! um lote de `stat`.
//!
//! # Ordem do trabalho
//!
//! 1. Travessia paralela (`jwalk`) coletando caminho, tamanho e mtime.
//! 2. Divisão em inalterados / a processar, contra o estado do índice.
//! 3. Parse de tag em paralelo (`rayon`), com a capa já deduplicada por hash
//!    dentro do worker — o blob morre ali e não chega a acumular na memória.
//! 4. Uma única transação escreve tudo.
//!
//! O passo 3 é o único que escala com o tamanho da biblioteca, e é o único
//! paralelo. O passo 4 é serial de propósito: uma transação com statements
//! preparados é mais rápida que várias em paralelo disputando o mesmo arquivo.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::{Accessor, ItemKey};
use rayon::prelude::*;
use rusqlite::Transaction;

use crate::art::{ArtCache, ArtRef, KnownArt};
use crate::db::{Db, Error, Result};
use crate::norm::{album_key, fold_key};

/// Extensões consideradas áudio. Casa com o que o `symphonia` decodifica —
/// não adianta indexar o que não vai tocar.
pub const AUDIO_EXT: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wav", "wave", "aiff", "aif",
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    /// Arquivos que nem foram abertos: `(tamanho, mtime)` bateu com o índice.
    pub unchanged: usize,
    /// Capas distintas decodificadas. Comparado com `added`, mostra o quanto a
    /// deduplicação economizou.
    pub art_decoded: usize,
}

/// `rel_path → (id, tamanho, mtime)`. É o estado do índice carregado de uma
/// vez só: ~4 MB para 50k faixas, e é o que permite decidir "mudou?" sem uma
/// consulta ao banco por arquivo.
type KnownTracks = HashMap<String, (i64, u64, i64)>;

/// Um arquivo visto na travessia, antes de qualquer parse.
struct Found {
    rel_path: String,
    file_size: u64,
    mtime_ns: i64,
}

/// Uma faixa depois do parse de tag.
struct Parsed {
    rel_path: String,
    file_size: u64,
    mtime_ns: i64,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    genre: Option<String>,
    disc_no: Option<u32>,
    track_no: Option<u32>,
    year: Option<i32>,
    duration_ms: Option<u64>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    bit_depth: Option<u8>,
    codec: Option<String>,
    bitrate_kbps: Option<u32>,
    art: Option<ArtRef>,
}

/// Varre `root` e atualiza o índice.
pub fn scan(db: &mut Db, root: &Path, art: &ArtCache) -> Result<ScanReport> {
    scan_with_progress(db, root, art, &AtomicUsize::new(0))
}

/// Igual a [`scan`], mas incrementa `progress` a cada arquivo processado, para
/// a UI conseguir mostrar andamento sem lock.
pub fn scan_with_progress(
    db: &mut Db,
    root: &Path,
    art: &ArtCache,
    progress: &AtomicUsize,
) -> Result<ScanReport> {
    let root_id = ensure_root(db, root)?;
    let known = load_known_tracks(db, root_id)?;
    art.preload(load_known_art(db)?);
    let art_before = art.len();

    let found = walk(root);

    // Divide contra o índice antes de abrir qualquer arquivo.
    let mut seen: HashSet<&str> = HashSet::with_capacity(found.len());
    let mut to_parse = Vec::new();
    let mut unchanged = 0usize;

    for file in &found {
        seen.insert(file.rel_path.as_str());
        match known.get(&file.rel_path) {
            Some(&(_, size, mtime)) if size == file.file_size && mtime == file.mtime_ns => {
                unchanged += 1;
            }
            _ => to_parse.push(file),
        }
    }

    let parsed: Vec<Parsed> = to_parse
        .par_iter()
        .filter_map(|file| {
            let out = parse(root, file, art);
            progress.fetch_add(1, Ordering::Relaxed);
            out
        })
        .collect();

    // Faixas que sumiram do disco.
    let removed: Vec<i64> = known
        .iter()
        .filter(|(path, _)| !seen.contains(path.as_str()))
        .map(|(_, &(id, _, _))| id)
        .collect();

    let mut report = ScanReport {
        unchanged,
        removed: removed.len(),
        ..ScanReport::default()
    };

    let tx = db.conn_mut().transaction()?;
    {
        let mut writer = Writer::new(&tx)?;
        for track in &parsed {
            let existing = known.get(&track.rel_path).map(|&(id, _, _)| id);
            writer.upsert(root_id, track, existing)?;
            if existing.is_some() {
                report.updated += 1;
            } else {
                report.added += 1;
            }
        }
        for id in removed {
            writer.delete(id)?;
        }
    }
    tx.commit()?;

    report.art_decoded = art.len() - art_before;
    Ok(report)
}

// -----------------------------------------------------------------------------
// Travessia
// -----------------------------------------------------------------------------

fn walk(root: &Path) -> Vec<Found> {
    jwalk::WalkDir::new(root)
        .skip_hidden(true)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| AUDIO_EXT.contains(&e.to_ascii_lowercase().as_str()))
        })
        .filter_map(|entry| {
            let path = entry.path();
            let meta = entry.metadata().ok()?;
            Some(Found {
                rel_path: rel_path(root, &path)?,
                file_size: meta.len(),
                mtime_ns: mtime_ns(&meta),
            })
        })
        .collect()
}

/// Caminho relativo à raiz, sempre com `/`.
///
/// Normalizar o separador deixa o índice portátil: o mesmo arquivo de
/// biblioteca abre no Linux e no Windows, e o sync não vê dois caminhos
/// diferentes para a mesma faixa.
fn rel_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(
        rel.components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or(0)
}

// -----------------------------------------------------------------------------
// Parse
// -----------------------------------------------------------------------------

fn parse(root: &Path, file: &Found, art_cache: &ArtCache) -> Option<Parsed> {
    let path = root.join(&file.rel_path);
    // Arquivo ilegível ou corrompido é pulado, não aborta o scan: uma faixa
    // ruim no meio de 50 mil não pode custar a biblioteca inteira.
    let tagged = lofty::read_from_path(&path).ok()?;

    let props = tagged.properties();
    let codec = format!("{:?}", tagged.file_type());
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let mut parsed = Parsed {
        rel_path: file.rel_path.clone(),
        file_size: file.file_size,
        mtime_ns: file.mtime_ns,
        title: None,
        artist: None,
        album: None,
        album_artist: None,
        genre: None,
        disc_no: None,
        track_no: None,
        year: None,
        duration_ms: u64::try_from(props.duration().as_millis()).ok(),
        sample_rate: props.sample_rate(),
        channels: props.channels().map(u16::from),
        bit_depth: props.bit_depth(),
        codec: Some(codec),
        bitrate_kbps: props.audio_bitrate(),
        art: None,
    };

    if let Some(tag) = tag {
        parsed.title = tag.title().map(std::borrow::Cow::into_owned);
        parsed.artist = tag.artist().map(std::borrow::Cow::into_owned);
        parsed.album = tag.album().map(std::borrow::Cow::into_owned);
        parsed.genre = tag.genre().map(std::borrow::Cow::into_owned);
        parsed.album_artist = tag.get_string(&ItemKey::AlbumArtist).map(str::to_owned);
        parsed.track_no = tag.track();
        parsed.disc_no = tag.disk();
        parsed.year = tag.year().and_then(|y| i32::try_from(y).ok());

        // A capa é registrada aqui dentro, no worker: o blob é hasheado,
        // deduplicado e descartado antes de sair desta função. É o que impede
        // 50 mil blobs de arte de coexistirem na memória.
        if let Some(picture) = tag.pictures().first() {
            parsed.art = art_cache.store(picture.data());
        }
    }

    // Sem título na tag, o nome do arquivo é melhor que nada.
    if parsed.title.is_none() {
        parsed.title = Path::new(&file.rel_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_owned);
    }

    Some(parsed)
}

// -----------------------------------------------------------------------------
// Escrita
// -----------------------------------------------------------------------------

fn ensure_root(db: &Db, root: &Path) -> Result<i64> {
    let path = root.to_string_lossy();
    let now = now_ms();
    db.conn().execute(
        "INSERT INTO library_root (path, added_at) VALUES (?1, ?2)
         ON CONFLICT (path) DO NOTHING",
        (&path, now),
    )?;
    Ok(db.conn().query_row(
        "SELECT id FROM library_root WHERE path = ?1",
        [&path],
        |row| row.get(0),
    )?)
}

fn load_known_tracks(db: &Db, root_id: i64) -> Result<KnownTracks> {
    let mut stmt = db
        .conn()
        .prepare("SELECT id, rel_path, file_size, mtime_ns FROM track WHERE root_id = ?1")?;
    let rows = stmt.query_map([root_id], |row| {
        Ok((
            row.get::<_, String>(1)?,
            (row.get(0)?, row.get::<_, i64>(2)? as u64, row.get(3)?),
        ))
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Error::from)
}

fn load_known_art(db: &Db) -> Result<Vec<KnownArt>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT blob_hash, width, height FROM cover_art")?;
    let rows = stmt.query_map([], |row| {
        let hash: Vec<u8> = row.get(0)?;
        let width: Option<u32> = row.get(1)?;
        let height: Option<u32> = row.get(2)?;
        Ok((hash, width.unwrap_or(0), height.unwrap_or(0)))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (hash, width, height) = row?;
        if let Ok(hash) = <[u8; 32]>::try_from(hash.as_slice()) {
            out.push((hash, (width, height)));
        }
    }
    Ok(out)
}

/// Escritor com statements preparados e caches de id.
///
/// Preparar uma vez e reusar 50 000 vezes é a diferença entre o SQLite gastar
/// o tempo parseando SQL e gastar escrevendo. Os caches evitam um `SELECT` por
/// faixa para achar artista e álbum.
struct Writer<'tx> {
    tx: &'tx Transaction<'tx>,
    artists: HashMap<String, i64>,
    albums: HashMap<String, i64>,
    arts: HashMap<[u8; 32], i64>,
}

impl<'tx> Writer<'tx> {
    fn new(tx: &'tx Transaction<'tx>) -> Result<Self> {
        let mut artists = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT name_key, id FROM artist")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            for row in rows {
                let (key, id) = row?;
                artists.insert(key, id);
            }
        }

        let mut albums = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT key, id FROM album")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            for row in rows {
                let (key, id) = row?;
                albums.insert(key, id);
            }
        }

        let mut arts = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT blob_hash, id FROM cover_art")?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get(1)?)))?;
            for row in rows {
                let (hash, id) = row?;
                if let Ok(hash) = <[u8; 32]>::try_from(hash.as_slice()) {
                    arts.insert(hash, id);
                }
            }
        }

        Ok(Self {
            tx,
            artists,
            albums,
            arts,
        })
    }

    fn artist_id(&mut self, name: &str) -> Result<i64> {
        let key = fold_key(name);
        if let Some(&id) = self.artists.get(&key) {
            return Ok(id);
        }
        self.tx.execute(
            "INSERT INTO artist (name, name_key) VALUES (?1, ?2)
             ON CONFLICT (name_key) DO NOTHING",
            (name, &key),
        )?;
        let id: i64 =
            self.tx
                .query_row("SELECT id FROM artist WHERE name_key = ?1", [&key], |r| {
                    r.get(0)
                })?;
        self.artists.insert(key, id);
        Ok(id)
    }

    fn art_id(&mut self, art: &ArtRef) -> Result<i64> {
        if let Some(&id) = self.arts.get(&art.hash) {
            return Ok(id);
        }
        self.tx.execute(
            "INSERT INTO cover_art (blob_hash, mime, width, height) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (blob_hash) DO NOTHING",
            (art.hash.as_slice(), "image/jpeg", art.width, art.height),
        )?;
        let id: i64 = self.tx.query_row(
            "SELECT id FROM cover_art WHERE blob_hash = ?1",
            [art.hash.as_slice()],
            |r| r.get(0),
        )?;
        self.arts.insert(art.hash, id);
        Ok(id)
    }

    fn album_id(
        &mut self,
        title: &str,
        album_artist: Option<&str>,
        album_artist_id: Option<i64>,
        year: Option<i32>,
        art_id: Option<i64>,
    ) -> Result<i64> {
        let key = album_key(album_artist, title);
        if let Some(&id) = self.albums.get(&key) {
            return Ok(id);
        }
        self.tx.execute(
            "INSERT INTO album (title, album_artist_id, year, art_id, key)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (key) DO NOTHING",
            (title, album_artist_id, year, art_id, &key),
        )?;
        let id: i64 = self
            .tx
            .query_row("SELECT id FROM album WHERE key = ?1", [&key], |r| r.get(0))?;
        self.albums.insert(key, id);
        Ok(id)
    }

    fn upsert(&mut self, root_id: i64, track: &Parsed, existing: Option<i64>) -> Result<()> {
        let artist_id = match track.artist.as_deref() {
            Some(name) => Some(self.artist_id(name)?),
            None => None,
        };
        // Sem tag de álbum-artista, o artista da faixa assume: é o que faz um
        // álbum de um artista só não virar um álbum por faixa.
        let album_artist_name = track.album_artist.as_deref().or(track.artist.as_deref());
        let album_artist_id = match album_artist_name {
            Some(name) => Some(self.artist_id(name)?),
            None => None,
        };
        let art_id = match &track.art {
            Some(art) => Some(self.art_id(art)?),
            None => None,
        };
        let album_id = match track.album.as_deref() {
            Some(title) => Some(self.album_id(
                title,
                album_artist_name,
                album_artist_id,
                track.year,
                art_id,
            )?),
            None => None,
        };

        let now = now_ms();
        let id = if let Some(id) = existing {
            self.tx.execute(
                "UPDATE track SET
                     file_size = ?2, mtime_ns = ?3, content_hash = NULL,
                     title = ?4, album_id = ?5, artist_id = ?6, album_artist_id = ?7,
                     disc_no = ?8, track_no = ?9, year = ?10, genre = ?11, art_id = ?12,
                     duration_ms = ?13, sample_rate = ?14, channels = ?15, bit_depth = ?16,
                     codec = ?17, bitrate_kbps = ?18, scanned_at = ?19
                 WHERE id = ?1",
                rusqlite::params![
                    id,
                    track.file_size as i64,
                    track.mtime_ns,
                    track.title,
                    album_id,
                    artist_id,
                    album_artist_id,
                    track.disc_no,
                    track.track_no,
                    track.year,
                    track.genre,
                    art_id,
                    track.duration_ms.map(|d| d as i64),
                    track.sample_rate,
                    track.channels,
                    track.bit_depth,
                    track.codec,
                    track.bitrate_kbps,
                    now,
                ],
            )?;
            self.tx
                .execute("DELETE FROM track_fts WHERE rowid = ?1", [id])?;
            id
        } else {
            self.tx.execute(
                "INSERT INTO track (
                     root_id, rel_path, file_size, mtime_ns,
                     title, album_id, artist_id, album_artist_id,
                     disc_no, track_no, year, genre, art_id,
                     duration_ms, sample_rate, channels, bit_depth,
                     codec, bitrate_kbps, scanned_at
                 ) VALUES (
                     ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
                 )",
                rusqlite::params![
                    root_id,
                    track.rel_path,
                    track.file_size as i64,
                    track.mtime_ns,
                    track.title,
                    album_id,
                    artist_id,
                    album_artist_id,
                    track.disc_no,
                    track.track_no,
                    track.year,
                    track.genre,
                    art_id,
                    track.duration_ms.map(|d| d as i64),
                    track.sample_rate,
                    track.channels,
                    track.bit_depth,
                    track.codec,
                    track.bitrate_kbps,
                    now,
                ],
            )?;
            self.tx.last_insert_rowid()
        };

        self.tx.execute(
            "INSERT INTO track_fts (rowid, title, artist, album, album_artist)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                id,
                track.title,
                track.artist,
                track.album,
                track.album_artist,
            ],
        )?;

        Ok(())
    }

    fn delete(&mut self, id: i64) -> Result<()> {
        self.tx
            .execute("DELETE FROM track_fts WHERE rowid = ?1", [id])?;
        self.tx.execute("DELETE FROM track WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    use crate::testutil::{ambiente, conta, escreve, mp3_silencioso, png};

    #[test]
    fn indexa_faixas_e_extrai_metadata() {
        let env = ambiente("indexa");
        escreve(
            &env.musica
                .join("Legião Urbana/Dois/01 - Eduardo e Mônica.mp3"),
            "Eduardo e Mônica",
            "Legião Urbana",
            "Dois",
            None,
        );
        escreve(
            &env.musica.join("Legião Urbana/Dois/02 - Índios.mp3"),
            "Índios",
            "Legião Urbana",
            "Dois",
            None,
        );

        let mut db = Db::open_in_memory().expect("abrir índice");
        let art = ArtCache::new(env.cache.clone());
        let report = scan(&mut db, &env.musica, &art).expect("escanear");

        assert_eq!(report.added, 2);
        assert_eq!(report.updated, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(conta(&db, "SELECT count(*) FROM track"), 2);
        // Duas faixas, um artista, um álbum: a deduplicação por chave dobrada
        // funcionou.
        assert_eq!(conta(&db, "SELECT count(*) FROM artist"), 1);
        assert_eq!(conta(&db, "SELECT count(*) FROM album"), 1);

        let titulo: String = db
            .conn()
            .query_row(
                "SELECT title FROM track ORDER BY rel_path LIMIT 1",
                [],
                |r| r.get(0),
            )
            .expect("ler título");
        assert_eq!(titulo, "Eduardo e Mônica");
    }

    /// O caminho rápido: nada mudou, então nenhum arquivo é aberto.
    #[test]
    fn rescan_sem_mudanca_nao_reprocessa_nada() {
        let env = ambiente("rescan");
        escreve(
            &env.musica.join("a/b/1.mp3"),
            "Um",
            "Artista",
            "Álbum",
            None,
        );
        escreve(
            &env.musica.join("a/b/2.mp3"),
            "Dois",
            "Artista",
            "Álbum",
            None,
        );

        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("primeiro scan");

        let report = scan(&mut db, &env.musica, &art).expect("segundo scan");
        assert_eq!(report.unchanged, 2);
        assert_eq!(report.added, 0);
        assert_eq!(report.updated, 0);
        assert_eq!(conta(&db, "SELECT count(*) FROM track"), 2);
    }

    #[test]
    fn arquivo_apagado_sai_do_indice_e_da_busca() {
        let env = ambiente("apagado");
        let alvo = env.musica.join("a/1.mp3");
        escreve(&alvo, "Um", "Artista", "Álbum", None);
        escreve(
            &env.musica.join("a/2.mp3"),
            "Dois",
            "Artista",
            "Álbum",
            None,
        );

        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("primeiro scan");

        fs::remove_file(&alvo).expect("apagar arquivo");
        let report = scan(&mut db, &env.musica, &art).expect("segundo scan");

        assert_eq!(report.removed, 1);
        assert_eq!(conta(&db, "SELECT count(*) FROM track"), 1);
        // O índice de busca não pode ficar com a faixa fantasma.
        assert_eq!(conta(&db, "SELECT count(*) FROM track_fts"), 1);
    }

    #[test]
    fn arquivo_editado_e_reindexado_sem_duplicar() {
        let env = ambiente("editado");
        let alvo = env.musica.join("a/1.mp3");
        escreve(&alvo, "Título Antigo", "Artista", "Álbum", None);

        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("primeiro scan");

        escreve(
            &alvo,
            "Título Novo Bem Mais Comprido",
            "Artista",
            "Álbum",
            None,
        );
        let report = scan(&mut db, &env.musica, &art).expect("segundo scan");

        assert_eq!(report.updated, 1);
        assert_eq!(report.added, 0);
        assert_eq!(conta(&db, "SELECT count(*) FROM track"), 1);
        assert_eq!(
            conta(&db, "SELECT count(*) FROM track_fts"),
            1,
            "reindexar duplicou a linha na busca"
        );
    }

    /// O ponto da deduplicação: um álbum inteiro compartilha uma capa, e ela é
    /// decodificada uma única vez.
    #[test]
    fn capa_do_album_e_decodificada_uma_vez_so() {
        let env = ambiente("capa");
        let capa = png([10, 40, 200]);
        for n in 1..=5 {
            escreve(
                &env.musica.join(format!("a/{n}.mp3")),
                &format!("Faixa {n}"),
                "Artista",
                "Álbum",
                Some(&capa),
            );
        }

        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        let report = scan(&mut db, &env.musica, &art).expect("escanear");

        assert_eq!(report.added, 5);
        assert_eq!(report.art_decoded, 1, "a mesma capa foi decodificada 5x");
        assert_eq!(conta(&db, "SELECT count(*) FROM cover_art"), 1);
        assert_eq!(
            conta(&db, "SELECT count(*) FROM track WHERE art_id IS NOT NULL"),
            5,
            "todas as faixas apontam para a capa"
        );
    }

    #[test]
    fn busca_encontra_sem_digitar_acento() {
        let env = ambiente("busca");
        escreve(
            &env.musica.join("a/1.mp3"),
            "Eduardo e Mônica",
            "Legião Urbana",
            "Dois",
            None,
        );

        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("escanear");

        for termo in ["monica", "legiao", "Mônica"] {
            let hits: i64 = db
                .conn()
                .query_row(
                    "SELECT count(*) FROM track_fts WHERE track_fts MATCH ?1",
                    [termo],
                    |r| r.get(0),
                )
                .expect("consultar busca");
            assert_eq!(hits, 1, "busca por {termo:?} não achou");
        }
    }

    #[test]
    fn arquivo_sem_tag_ainda_entra_com_o_nome_do_arquivo() {
        let env = ambiente("sem-tag");
        let alvo = env.musica.join("solta/Faixa Sem Tag.mp3");
        fs::create_dir_all(alvo.parent().expect("pai")).expect("criar dir");
        fs::write(&alvo, mp3_silencioso()).expect("gravar");

        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        let report = scan(&mut db, &env.musica, &art).expect("escanear");

        assert_eq!(report.added, 1);
        let titulo: String = db
            .conn()
            .query_row("SELECT title FROM track", [], |r| r.get(0))
            .expect("ler título");
        assert_eq!(titulo, "Faixa Sem Tag");
    }
}
