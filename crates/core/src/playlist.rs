//! Playlists.
//!
//! # Duas coisas que parecem detalhe e não são
//!
//! **O item aponta para o hash, não para o id da faixa.** `track.id` é
//! autoincrement local: o mesmo arquivo tem id diferente em cada máquina, e
//! uma playlist guardada por id não sobreviveria ao sync. Por isso um item
//! pode existir sem faixa local — veio de outro device e o arquivo ainda não
//! chegou. Ele aparece como buraco, não some.
//!
//! **A posição é uma string, não um número.** Ver [`crate::fracidx`]: arrastar
//! um item escreve uma linha em vez de renumerar a playlist inteira, e dois
//! devices reordenando ao mesmo tempo não colidem.
//!
//! # Apagar é marcar
//!
//! Apagar uma playlist não apaga a linha, marca `deleted`. Sem esse túmulo, o
//! outro device reintroduziria a playlist apagada no próximo encontro — ele
//! tem uma linha que este não tem, e "não tenho" é indistinguível de "apaguei".

use uuid::Uuid;

use crate::db::{Db, Error, Result, now_ms};
use crate::fracidx;
use crate::model::{DeviceId, TrackId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: Uuid,
    pub name: String,
    /// Itens na playlist, incluindo os que ainda não têm arquivo local.
    pub items: usize,
    pub updated_at: i64,
}

/// Um item da playlist. `track` é `None` quando o arquivo não está aqui.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub position: String,
    pub track: Option<TrackId>,
}

/// O identificador deste device, criado na primeira chamada.
///
/// **Provisório.** Na Fase 4 este id passa a ser a chave pública estática do
/// Noise, para que parear e identificar sejam a mesma operação. Até lá é um
/// valor aleatório, que já serve para carimbar quem escreveu cada playlist.
pub fn self_device(db: &Db) -> Result<DeviceId> {
    let existing: Option<Vec<u8>> = db
        .conn()
        .query_row("SELECT id FROM device WHERE is_self = 1", [], |row| {
            row.get(0)
        })
        .ok();

    if let Some(bytes) = existing
        && let Ok(id) = <[u8; 32]>::try_from(bytes.as_slice())
    {
        return Ok(DeviceId(id));
    }

    let mut id = [0u8; 32];
    id[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    id[16..].copy_from_slice(Uuid::new_v4().as_bytes());

    db.conn().execute(
        "INSERT INTO device (id, name, is_self, paired_at) VALUES (?1, ?2, 1, ?3)",
        rusqlite::params![id.as_slice(), "este dispositivo", now_ms()],
    )?;
    Ok(DeviceId(id))
}

pub fn create(db: &Db, name: &str) -> Result<Uuid> {
    let device = self_device(db)?;
    // UUIDv7 é ordenável no tempo e gerável offline: dois devices criando
    // playlists sem se ver não colidem nem precisam de servidor.
    let id = Uuid::now_v7();
    let now = now_ms();

    db.conn().execute(
        "INSERT INTO playlist (id, name, created_at, updated_at, deleted, origin)
         VALUES (?1, ?2, ?3, ?3, 0, ?4)",
        rusqlite::params![id.as_bytes().as_slice(), name, now, device.0.as_slice()],
    )?;
    Ok(id)
}

pub fn rename(db: &Db, id: Uuid, name: &str) -> Result<()> {
    let device = self_device(db)?;
    db.conn().execute(
        "UPDATE playlist SET name = ?1, updated_at = ?2, origin = ?3 WHERE id = ?4",
        rusqlite::params![
            name,
            now_ms(),
            device.0.as_slice(),
            id.as_bytes().as_slice()
        ],
    )?;
    Ok(())
}

/// Marca a playlist como apagada. Os itens ficam, para o túmulo poder viajar.
pub fn delete(db: &Db, id: Uuid) -> Result<()> {
    let device = self_device(db)?;
    db.conn().execute(
        "UPDATE playlist SET deleted = 1, updated_at = ?1, origin = ?2 WHERE id = ?3",
        rusqlite::params![now_ms(), device.0.as_slice(), id.as_bytes().as_slice()],
    )?;
    Ok(())
}

/// Carimba quem mexeu e quando. É o que o merge de LWW do sync vai comparar,
/// então toda alteração de conteúdo passa por aqui.
fn touch(db: &Db, id: Uuid) -> Result<()> {
    let device = self_device(db)?;
    db.conn().execute(
        "UPDATE playlist SET updated_at = ?1, origin = ?2 WHERE id = ?3",
        rusqlite::params![now_ms(), device.0.as_slice(), id.as_bytes().as_slice()],
    )?;
    Ok(())
}

/// Playlists vivas, mais recentes primeiro.
pub fn all(db: &Db) -> Result<Vec<Playlist>> {
    let mut stmt = db.conn().prepare(
        "SELECT p.id, p.name, p.updated_at,
                (SELECT count(*) FROM playlist_item i WHERE i.playlist_id = p.id)
         FROM playlist p
         WHERE p.deleted = 0
         ORDER BY p.updated_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: Vec<u8> = row.get(0)?;
        Ok(Playlist {
            id: <[u8; 16]>::try_from(id.as_slice()).map_or_else(|_| Uuid::nil(), Uuid::from_bytes),
            name: row.get(1)?,
            updated_at: row.get(2)?,
            items: row.get::<_, i64>(3)? as usize,
        })
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Error::from)
}

/// Itens na ordem, com a faixa local resolvida quando existe.
pub fn items(db: &Db, id: Uuid) -> Result<Vec<Item>> {
    let mut stmt = db.conn().prepare_cached(
        // Subconsulta em vez de JOIN: dois arquivos idênticos na biblioteca
        // compartilham o hash, e um JOIN duplicaria o item.
        "SELECT i.position,
                (SELECT t.id FROM track t WHERE t.content_hash = i.track_key LIMIT 1)
         FROM playlist_item i
         WHERE i.playlist_id = ?1
         ORDER BY i.position",
    )?;
    let rows = stmt.query_map([id.as_bytes().as_slice()], |row| {
        Ok(Item {
            position: row.get(0)?,
            track: row.get::<_, Option<i64>>(1)?.map(TrackId),
        })
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Error::from)
}

/// Só as faixas que existem localmente, na ordem da playlist.
pub fn tracks(db: &Db, id: Uuid) -> Result<Vec<TrackId>> {
    Ok(items(db, id)?.into_iter().filter_map(|i| i.track).collect())
}

/// Acrescenta faixas ao fim. Devolve quantas entraram.
///
/// Precisa de `&mut Db` porque hashear é o que dá identidade à faixa, e o hash
/// é gravado no índice.
pub fn append(db: &mut Db, id: Uuid, tracks: &[TrackId]) -> Result<usize> {
    if tracks.is_empty() {
        return Ok(0);
    }
    let hashes = crate::hash::ensure_hashes(db, tracks)?;

    let last: Option<String> = db
        .conn()
        .query_row(
            "SELECT position FROM playlist_item WHERE playlist_id = ?1
             ORDER BY position DESC LIMIT 1",
            [id.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .ok();

    let positions = fracidx::append_many(last.as_deref(), tracks.len());
    let now = now_ms();
    let mut added = 0usize;

    let tx = db.conn().unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO playlist_item (playlist_id, position, track_key, added_at)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (track, position) in tracks.iter().zip(&positions) {
            // Faixa cujo arquivo sumiu no meio do caminho não entra, mas não
            // impede as outras.
            let Some(key) = hashes.get(track) else {
                continue;
            };
            stmt.execute(rusqlite::params![
                id.as_bytes().as_slice(),
                position,
                key.0.as_slice(),
                now
            ])?;
            added += 1;
        }
    }
    tx.commit()?;

    if added > 0 {
        touch(db, id)?;
    }
    Ok(added)
}

pub fn remove(db: &Db, id: Uuid, position: &str) -> Result<()> {
    db.conn().execute(
        "DELETE FROM playlist_item WHERE playlist_id = ?1 AND position = ?2",
        rusqlite::params![id.as_bytes().as_slice(), position],
    )?;
    touch(db, id)
}

/// Move o item que está em `position` para a posição `to` da lista.
///
/// Escreve **uma** linha: é o índice fracionário fazendo o trabalho.
pub fn move_item(db: &Db, id: Uuid, position: &str, to: usize) -> Result<()> {
    let mut ordered: Vec<String> = items(db, id)?.into_iter().map(|i| i.position).collect();
    let Some(from) = ordered.iter().position(|p| p == position) else {
        return Ok(());
    };
    ordered.remove(from);

    let to = to.min(ordered.len());
    let after = to
        .checked_sub(1)
        .and_then(|i| ordered.get(i))
        .map(String::as_str);
    let before = ordered.get(to).map(String::as_str);
    let novo = fracidx::between(after, before);

    db.conn().execute(
        "UPDATE playlist_item SET position = ?3 WHERE playlist_id = ?1 AND position = ?2",
        rusqlite::params![id.as_bytes().as_slice(), position, novo],
    )?;
    touch(db, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::art::ArtCache;
    use crate::library::{Sort, view};
    use crate::scan::scan;
    use crate::testutil::{ambiente, escreve};

    fn biblioteca(nome: &str, faixas: usize) -> (Db, crate::testutil::Ambiente) {
        let env = ambiente(nome);
        for n in 0..faixas {
            escreve(
                &env.musica.join(format!("a/{n:02}.mp3")),
                &format!("Faixa {n:02}"),
                "Artista",
                "Álbum",
                None,
            );
        }
        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("escanear");
        (db, env)
    }

    #[test]
    fn cria_lista_e_apaga() {
        let (db, _env) = biblioteca("crud", 2);
        let id = create(&db, "Corrida").expect("criar");

        let listas = all(&db).expect("listar");
        assert_eq!(listas.len(), 1);
        assert_eq!(listas[0].name, "Corrida");
        assert_eq!(listas[0].items, 0);

        rename(&db, id, "Academia").expect("renomear");
        assert_eq!(all(&db).expect("listar")[0].name, "Academia");

        delete(&db, id).expect("apagar");
        assert!(all(&db).expect("listar").is_empty());
    }

    /// Apagar tem que deixar túmulo: sem a linha marcada, o sync não sabe
    /// distinguir "apaguei" de "nunca tive".
    #[test]
    fn apagar_deixa_tumulo_em_vez_de_sumir_com_a_linha() {
        let (db, _env) = biblioteca("tumulo", 2);
        let id = create(&db, "Some").expect("criar");
        delete(&db, id).expect("apagar");

        let linhas: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM playlist WHERE deleted = 1", [], |r| {
                r.get(0)
            })
            .expect("consultar");
        assert_eq!(linhas, 1);
    }

    #[test]
    fn acrescenta_faixas_na_ordem() {
        let (mut db, _env) = biblioteca("ordem", 5);
        let faixas = view(&db, Sort::ArtistAlbum).expect("view");
        let id = create(&db, "Mix").expect("criar");

        assert_eq!(append(&mut db, id, &faixas).expect("acrescentar"), 5);
        assert_eq!(tracks(&db, id).expect("faixas"), faixas);
        assert_eq!(all(&db).expect("listar")[0].items, 5);
    }

    #[test]
    fn acrescentar_hasheia_as_faixas() {
        let (mut db, _env) = biblioteca("hash", 3);
        let faixas = view(&db, Sort::ArtistAlbum).expect("view");
        let id = create(&db, "Mix").expect("criar");
        append(&mut db, id, &faixas).expect("acrescentar");

        let pendentes: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM track WHERE content_hash IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("consultar");
        assert_eq!(pendentes, 0);
    }

    #[test]
    fn a_mesma_faixa_pode_entrar_duas_vezes() {
        let (mut db, _env) = biblioteca("repetida", 2);
        let faixas = view(&db, Sort::ArtistAlbum).expect("view");
        let id = create(&db, "Mix").expect("criar");

        append(&mut db, id, &faixas[..1]).expect("primeira");
        append(&mut db, id, &faixas[..1]).expect("de novo");
        assert_eq!(tracks(&db, id).expect("faixas").len(), 2);
    }

    #[test]
    fn remove_um_item_sem_mexer_nos_outros() {
        let (mut db, _env) = biblioteca("remove", 4);
        let faixas = view(&db, Sort::ArtistAlbum).expect("view");
        let id = create(&db, "Mix").expect("criar");
        append(&mut db, id, &faixas).expect("acrescentar");

        let itens = items(&db, id).expect("itens");
        remove(&db, id, &itens[1].position).expect("remover");

        let restantes = tracks(&db, id).expect("faixas");
        assert_eq!(restantes, vec![faixas[0], faixas[2], faixas[3]]);
    }

    #[test]
    fn mover_para_o_comeco_e_para_o_fim() {
        let (mut db, _env) = biblioteca("mover", 4);
        let faixas = view(&db, Sort::ArtistAlbum).expect("view");
        let id = create(&db, "Mix").expect("criar");
        append(&mut db, id, &faixas).expect("acrescentar");

        let itens = items(&db, id).expect("itens");
        move_item(&db, id, &itens[3].position, 0).expect("mover pro começo");
        assert_eq!(
            tracks(&db, id).expect("faixas"),
            vec![faixas[3], faixas[0], faixas[1], faixas[2]]
        );

        // Mover o primeiro de volta para o fim desfaz o movimento anterior.
        let itens = items(&db, id).expect("itens");
        move_item(&db, id, &itens[0].position, 3).expect("mover pro fim");
        assert_eq!(tracks(&db, id).expect("faixas"), faixas);
    }

    /// Mover escreve uma linha, não renumera a lista: é o ponto do índice
    /// fracionário.
    #[test]
    fn mover_altera_apenas_a_posicao_do_item_movido() {
        let (mut db, _env) = biblioteca("uma-linha", 6);
        let faixas = view(&db, Sort::ArtistAlbum).expect("view");
        let id = create(&db, "Mix").expect("criar");
        append(&mut db, id, &faixas).expect("acrescentar");

        let antes: Vec<String> = items(&db, id)
            .expect("itens")
            .into_iter()
            .map(|i| i.position)
            .collect();
        move_item(&db, id, &antes[5], 2).expect("mover");
        let depois: Vec<String> = items(&db, id)
            .expect("itens")
            .into_iter()
            .map(|i| i.position)
            .collect();

        let mudadas = antes.iter().filter(|p| !depois.contains(p)).count();
        assert_eq!(mudadas, 1, "renumerou mais de um item");
    }

    /// Item cujo arquivo não existe aqui vira buraco, não some: ele pertence à
    /// playlist e o arquivo pode chegar depois pelo sync.
    #[test]
    fn item_sem_arquivo_local_continua_na_lista() {
        let (mut db, _env) = biblioteca("buraco", 3);
        let faixas = view(&db, Sort::ArtistAlbum).expect("view");
        let id = create(&db, "Mix").expect("criar");
        append(&mut db, id, &faixas).expect("acrescentar");

        // Some com a faixa do meio do índice, como se o arquivo não estivesse
        // neste device.
        db.conn()
            .execute("DELETE FROM track WHERE id = ?1", [faixas[1].0])
            .expect("apagar faixa");

        let itens = items(&db, id).expect("itens");
        assert_eq!(itens.len(), 3, "o item sumiu junto com a faixa");
        assert!(itens[1].track.is_none());
        assert_eq!(tracks(&db, id).expect("tocáveis").len(), 2);
    }

    #[test]
    fn o_device_proprio_e_criado_uma_vez_so() {
        let (db, _env) = biblioteca("device", 1);
        let a = self_device(&db).expect("primeiro");
        let b = self_device(&db).expect("segundo");
        assert_eq!(a, b);

        let linhas: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM device", [], |r| r.get(0))
            .expect("consultar");
        assert_eq!(linhas, 1);
    }
}
