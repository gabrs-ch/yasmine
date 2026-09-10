//! O merge da camada do usuário. Dois devices, quatro algoritmos, um por
//! tabela — todos convergem (aplicar duas vezes dá o mesmo resultado):
//!
//! | Tabela          | Regra                                                    |
//! |-----------------|---------------------------------------------------------|
//! | `playlist`      | LWW por `updated_at`, desempate pelo `origin` (bytes)    |
//! | `playlist_item` | união por `(playlist_id, position)`; túmulo LWW por `deleted_at` |
//! | `play_count`    | G-Counter: `MAX(count)` por `(track_key, device_id)`     |
//! | `track_state`   | LWW **por campo** (`rating` e `resume_pos` independentes)|
//!
//! `deleted` (playlist e item) é respeitado: apagar num device fica apagado,
//! não é reintroduzido pelo outro.

use player_core::Db;
use player_core::db::now_ms;
use rusqlite::{OptionalExtension, params};

use crate::Result;
use crate::protocol::UserLayer;

/// Aplica o snapshot do par no índice local. Devolve quantas playlists foram
/// inseridas ou atualizadas (para o relatório).
pub fn apply(db: &mut Db, layer: &UserLayer, peer: player_core::DeviceId) -> Result<usize> {
    let self_id: Option<Vec<u8>> = db
        .conn()
        .query_row("SELECT id FROM device WHERE is_self = 1", [], |r| r.get(0))
        .optional()?;
    let self_id = self_id.unwrap_or_default();
    let now = now_ms();
    let mut merged = 0usize;

    let tx = db.conn_mut().transaction()?;

    // --- device: nunca toca no próprio is_self ---
    for d in &layer.devices {
        if d.id.as_slice() == self_id.as_slice() {
            continue;
        }
        tx.execute(
            "INSERT INTO device (id, name, is_self, paired_at, last_sync_at)
             VALUES (?1, ?2, 0, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name",
            params![
                d.id.as_slice(),
                d.name,
                d.paired_at.unwrap_or(now),
                d.last_sync_at
            ],
        )?;
    }

    // --- playlist: LWW ---
    for p in &layer.playlists {
        ensure_device(&tx, &p.origin)?;
        let local: Option<(i64, Vec<u8>)> = tx
            .query_row(
                "SELECT updated_at, origin FROM playlist WHERE id = ?1",
                [p.id.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let take = match &local {
            None => true,
            Some((lu, lo)) => {
                p.updated_at > *lu || (p.updated_at == *lu && p.origin.as_slice() > lo.as_slice())
            }
        };
        if take {
            tx.execute(
                "INSERT INTO playlist (id, name, created_at, updated_at, deleted, origin)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name, updated_at = excluded.updated_at,
                    deleted = excluded.deleted, origin = excluded.origin",
                params![
                    p.id.as_slice(),
                    p.name,
                    p.created_at,
                    p.updated_at,
                    i64::from(p.deleted),
                    p.origin.as_slice()
                ],
            )?;
            merged += 1;
        }
    }

    // --- playlist_item: união + túmulo LWW ---
    for it in &layer.items {
        let has_playlist = tx
            .query_row(
                "SELECT 1 FROM playlist WHERE id = ?1",
                [it.playlist_id.as_slice()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !has_playlist {
            continue; // item de uma playlist que o LWW não trouxe
        }
        let local: Option<i64> = tx
            .query_row(
                "SELECT deleted_at FROM playlist_item WHERE playlist_id = ?1 AND position = ?2",
                params![it.playlist_id.as_slice(), it.position],
                |r| r.get(0),
            )
            .optional()?;
        match local {
            None => {
                tx.execute(
                    "INSERT INTO playlist_item
                        (playlist_id, position, track_key, added_at, deleted, deleted_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        it.playlist_id.as_slice(),
                        it.position,
                        it.track_key.as_slice(),
                        it.added_at,
                        i64::from(it.deleted),
                        it.deleted_at
                    ],
                )?;
            }
            Some(local_da) if it.deleted_at > local_da => {
                tx.execute(
                    "UPDATE playlist_item SET deleted = ?3, deleted_at = ?4
                     WHERE playlist_id = ?1 AND position = ?2",
                    params![
                        it.playlist_id.as_slice(),
                        it.position,
                        i64::from(it.deleted),
                        it.deleted_at
                    ],
                )?;
            }
            Some(_) => {}
        }
    }

    // --- play_count: G-Counter ---
    for pc in &layer.play_counts {
        ensure_device(&tx, &pc.device_id)?;
        tx.execute(
            "INSERT INTO play_count (track_key, device_id, count) VALUES (?1, ?2, ?3)
             ON CONFLICT(track_key, device_id)
             DO UPDATE SET count = MAX(play_count.count, excluded.count)",
            params![pc.track_key.as_slice(), pc.device_id.as_slice(), pc.count],
        )?;
    }

    // --- track_state: LWW por campo ---
    for ts in &layer.track_states {
        tx.execute(
            "INSERT INTO track_state
                (track_key, last_played_at, rating, rating_updated_at,
                 resume_pos_ms, resume_updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(track_key) DO UPDATE SET
                last_played_at = NULLIF(MAX(
                    COALESCE(track_state.last_played_at, 0),
                    COALESCE(excluded.last_played_at, 0)), 0),
                rating = CASE WHEN excluded.rating_updated_at > track_state.rating_updated_at
                         THEN excluded.rating ELSE track_state.rating END,
                rating_updated_at = MAX(track_state.rating_updated_at, excluded.rating_updated_at),
                resume_pos_ms = CASE WHEN excluded.resume_updated_at > track_state.resume_updated_at
                                THEN excluded.resume_pos_ms ELSE track_state.resume_pos_ms END,
                resume_updated_at = MAX(track_state.resume_updated_at, excluded.resume_updated_at)",
            params![
                ts.track_key.as_slice(),
                ts.last_played_at,
                ts.rating,
                ts.rating_updated_at,
                ts.resume_pos_ms,
                ts.resume_updated_at
            ],
        )?;
    }

    // Carimba o par como pareado agora.
    tx.execute(
        "INSERT INTO device (id, name, is_self, paired_at, last_sync_at)
         VALUES (?1, ?2, 0, ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET
            paired_at = COALESCE(device.paired_at, ?3), last_sync_at = ?3",
        params![peer.as_bytes().as_slice(), "dispositivo pareado", now],
    )?;

    tx.commit()?;
    Ok(merged)
}

/// Lê a camada do usuário inteira como linhas cruas. É o que o host manda.
pub fn snapshot(db: &Db) -> Result<UserLayer> {
    use crate::protocol::{DeviceRow, ItemRow, PlayCountRow, PlaylistRow, TrackStateRow};
    let conn = db.conn();
    let mut layer = UserLayer::default();

    let mut q = conn.prepare("SELECT id, name, is_self, paired_at, last_sync_at FROM device")?;
    let rows = q.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })?;
    for row in rows {
        let (id, name, is_self, paired_at, last_sync_at) = row?;
        if let Some(id) = arr32(&id) {
            layer.devices.push(DeviceRow {
                id,
                name,
                is_self: is_self != 0,
                paired_at,
                last_sync_at,
            });
        }
    }

    let mut q =
        conn.prepare("SELECT id, name, created_at, updated_at, deleted, origin FROM playlist")?;
    let rows = q.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Vec<u8>>(5)?,
        ))
    })?;
    for row in rows {
        let (id, name, created_at, updated_at, deleted, origin) = row?;
        if let (Some(id), Some(origin)) = (arr16(&id), arr32(&origin)) {
            layer.playlists.push(PlaylistRow {
                id,
                name,
                created_at,
                updated_at,
                deleted: deleted != 0,
                origin,
            });
        }
    }

    let mut q = conn.prepare(
        "SELECT playlist_id, position, track_key, added_at, deleted, deleted_at FROM playlist_item",
    )?;
    let rows = q.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Vec<u8>>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    for row in rows {
        let (playlist_id, position, track_key, added_at, deleted, deleted_at) = row?;
        if let (Some(playlist_id), Some(track_key)) = (arr16(&playlist_id), arr32(&track_key)) {
            layer.items.push(ItemRow {
                playlist_id,
                position,
                track_key,
                added_at,
                deleted: deleted != 0,
                deleted_at,
            });
        }
    }

    let mut q = conn.prepare("SELECT track_key, device_id, count FROM play_count")?;
    let rows = q.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (track_key, device_id, count) = row?;
        if let (Some(track_key), Some(device_id)) = (arr32(&track_key), arr32(&device_id)) {
            layer.play_counts.push(PlayCountRow {
                track_key,
                device_id,
                count,
            });
        }
    }

    let mut q = conn.prepare(
        "SELECT track_key, last_played_at, rating, rating_updated_at, resume_pos_ms, resume_updated_at
         FROM track_state",
    )?;
    let rows = q.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    for row in rows {
        let (
            track_key,
            last_played_at,
            rating,
            rating_updated_at,
            resume_pos_ms,
            resume_updated_at,
        ) = row?;
        if let Some(track_key) = arr32(&track_key) {
            layer.track_states.push(TrackStateRow {
                track_key,
                last_played_at,
                rating: rating.map(|v| v as i32),
                rating_updated_at,
                resume_pos_ms,
                resume_updated_at,
            });
        }
    }

    Ok(layer)
}

fn ensure_device(tx: &rusqlite::Transaction<'_>, id: &[u8; 32]) -> Result<()> {
    tx.execute(
        "INSERT INTO device (id, name, is_self) VALUES (?1, '(desconhecido)', 0)
         ON CONFLICT(id) DO NOTHING",
        [id.as_slice()],
    )?;
    Ok(())
}

fn arr32(v: &[u8]) -> Option<[u8; 32]> {
    <[u8; 32]>::try_from(v).ok()
}
fn arr16(v: &[u8]) -> Option<[u8; 16]> {
    <[u8; 16]>::try_from(v).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PlayCountRow, PlaylistRow};
    use player_core::Db;

    fn db() -> Db {
        Db::open_in_memory().expect("db")
    }

    fn self_id(db: &Db) -> [u8; 32] {
        let v: Vec<u8> = db
            .conn()
            .query_row("SELECT id FROM device WHERE is_self = 1", [], |r| r.get(0))
            .expect("self");
        arr32(&v).expect("32")
    }

    #[test]
    fn playlist_lww_mantem_a_mais_nova() {
        let mut db = db();
        let me = crate::identity::Identity::load_or_create(&mut db, "A").expect("id");
        let pid = [0x11u8; 16];

        let older = UserLayer {
            playlists: vec![PlaylistRow {
                id: pid,
                name: "v1".into(),
                created_at: 1,
                updated_at: 10,
                deleted: false,
                origin: me.public(),
            }],
            ..Default::default()
        };
        let newer = UserLayer {
            playlists: vec![PlaylistRow {
                id: pid,
                name: "v2".into(),
                created_at: 1,
                updated_at: 20,
                deleted: false,
                origin: me.public(),
            }],
            ..Default::default()
        };
        apply(&mut db, &newer, player_core::DeviceId([9u8; 32])).expect("newer");
        apply(&mut db, &older, player_core::DeviceId([9u8; 32])).expect("older");

        let name: String = db
            .conn()
            .query_row("SELECT name FROM playlist WHERE id = ?1", [&pid[..]], |r| {
                r.get(0)
            })
            .expect("nome");
        assert_eq!(name, "v2", "o merge deixou a versão velha ganhar");
    }

    #[test]
    fn play_count_soma_por_max_e_nao_perde_play() {
        let mut db = db();
        let _ = crate::identity::Identity::load_or_create(&mut db, "A").expect("id");
        let me = self_id(&db);
        let other = [0x22u8; 32];
        let tk = [0x33u8; 32];

        // PC tocou 3x (peer), celular tocou 2x (nós).
        db.conn()
            .execute(
                "INSERT INTO play_count (track_key, device_id, count) VALUES (?1, ?2, 2)",
                params![&tk[..], &me[..]],
            )
            .expect("local");
        let layer = UserLayer {
            play_counts: vec![PlayCountRow {
                track_key: tk,
                device_id: other,
                count: 3,
            }],
            devices: vec![crate::protocol::DeviceRow {
                id: other,
                name: "PC".into(),
                is_self: false,
                paired_at: Some(1),
                last_sync_at: None,
            }],
            ..Default::default()
        };
        apply(&mut db, &layer, player_core::DeviceId(other)).expect("merge");
        // aplicar de novo não muda nada
        apply(&mut db, &layer, player_core::DeviceId(other)).expect("merge idempotente");

        let total: i64 = db
            .conn()
            .query_row(
                "SELECT SUM(count) FROM play_count WHERE track_key = ?1",
                [&tk[..]],
                |r| r.get(0),
            )
            .expect("total");
        assert_eq!(total, 5, "esperado 2 + 3");
    }

    #[test]
    fn item_apagado_de_um_lado_nao_volta() {
        let mut db = db();
        let me = crate::identity::Identity::load_or_create(&mut db, "A").expect("id");
        let pid = [0x44u8; 16];
        let tk = [0x55u8; 32];

        let base = UserLayer {
            playlists: vec![PlaylistRow {
                id: pid,
                name: "P".into(),
                created_at: 1,
                updated_at: 1,
                deleted: false,
                origin: me.public(),
            }],
            items: vec![crate::protocol::ItemRow {
                playlist_id: pid,
                position: "a0".into(),
                track_key: tk,
                added_at: 1,
                deleted: false,
                deleted_at: 0,
            }],
            ..Default::default()
        };
        apply(&mut db, &base, player_core::DeviceId([9u8; 32])).expect("base");

        // Nós apagamos o item localmente (túmulo com deleted_at alto).
        db.conn()
            .execute(
                "UPDATE playlist_item SET deleted = 1, deleted_at = 100
                 WHERE playlist_id = ?1 AND position = 'a0'",
                [&pid[..]],
            )
            .expect("apagar");

        // O par manda de novo a versão viva (deleted_at 0) — não pode ressuscitar.
        apply(&mut db, &base, player_core::DeviceId([9u8; 32])).expect("re-merge");
        let vivo: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM playlist_item
                 WHERE playlist_id = ?1 AND position = 'a0' AND deleted = 0",
                [&pid[..]],
                |r| r.get(0),
            )
            .expect("consultar");
        assert_eq!(vivo, 0, "o item apagado voltou no sync");
    }
}
