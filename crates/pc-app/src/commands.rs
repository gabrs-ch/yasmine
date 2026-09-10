//! A ponte IPC. Cada comando trava o `AppState`, chama `player-core` e
//! devolve DTO. Sem lógica de UI aqui — só tradução.

// A assinatura de `#[tauri::command]` dita: `State`/`AppHandle`/args
// desserializados entram por valor. Não é cópia cara (`State` é um wrapper de
// referência), e o macro não aceita `&`.
#![allow(clippy::needless_pass_by_value)]

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use player_core::{library, playlist, playlist_folder};

use crate::dto::{
    self, ArtistDto, LinkDto, OpenResult, PlaylistDto, SortArg, SourceArg, StatsDto, TrackRowDto,
};
use crate::hexhash;
use crate::playback::{self, PlaybackDto};
use crate::scan;
use crate::state::{AppState, META_LIBRARY_IMAGE, Source};

type St<'a> = State<'a, Mutex<AppState>>;

const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];

fn parse_uuid(s: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s).map_err(|_| "uuid inválido".to_string())
}

#[tauri::command]
pub fn library_stats(state: St<'_>) -> Result<StatsDto, String> {
    let st = state.lock().expect("estado do app");
    library::stats(&st.db)
        .map(StatsDto::from)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn current_root(state: St<'_>) -> Option<String> {
    state
        .lock()
        .expect("estado do app")
        .root
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn list_playlists(state: St<'_>) -> Result<Vec<PlaylistDto>, String> {
    let st = state.lock().expect("estado do app");
    let pls = playlist::all(&st.db).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(pls.len());
    for pl in &pls {
        let covers = playlist::cover_hashes(&st.db, pl.id).unwrap_or_default();
        let linked = !playlist_folder::links_for(&st.db, pl.id)
            .unwrap_or_default()
            .is_empty();
        out.push(dto::playlist_dto(pl, &covers, linked));
    }
    Ok(out)
}

#[tauri::command]
pub fn list_artists(state: St<'_>) -> Result<Vec<ArtistDto>, String> {
    let st = state.lock().expect("estado do app");
    library::artists(&st.db)
        .map(|v| v.into_iter().map(ArtistDto::from).collect())
        .map_err(|e| e.to_string())
}

/// Abre uma fonte: monta a view (guardada no estado), devolve o que a lista e
/// o hero precisam de imediato.
#[tauri::command]
pub fn open_source(
    state: St<'_>,
    source: SourceArg,
    sort: SortArg,
    query: String,
) -> Result<OpenResult, String> {
    let mut st = state.lock().expect("estado do app");
    let src = Source::from_arg(&source)?;
    let sort_core: player_core::Sort = sort.into();

    let mut positions: Vec<String> = Vec::new();
    let ids: Vec<player_core::TrackId> = match src {
        Source::Library if query.trim().is_empty() => {
            library::view(&st.db, sort_core).map_err(|e| e.to_string())?
        }
        Source::Library => library::search(&st.db, &query, sort_core).map_err(|e| e.to_string())?,
        Source::Artist(a) => library::by_artist(&st.db, a, sort_core).map_err(|e| e.to_string())?,
        Source::Playlist(id) => {
            // Itens (não só faixas): guarda a `position` de cada um, na ordem,
            // pra "remover"/"mover" saberem o alvo. Item sem arquivo local
            // não entra na lista.
            let items = playlist::items(&st.db, id).map_err(|e| e.to_string())?;
            let mut v = Vec::with_capacity(items.len());
            for it in items {
                if let Some(t) = it.track {
                    v.push(t);
                    positions.push(it.position);
                }
            }
            v
        }
    };

    st.source = src;
    st.sort = sort;
    st.query = query;
    st.view = ids;
    st.view_positions = positions;
    let total = st.view.len();

    let (kind, title, subtitle, hero_art): (&'static str, String, String, Option<String>) =
        match src {
            Source::Library => {
                let s = library::stats(&st.db).map_err(|e| e.to_string())?;
                (
                    "library",
                    "Your Library".to_string(),
                    format!("{} tracks · {} albums", s.tracks, s.albums),
                    None,
                )
            }
            Source::Playlist(id) => {
                let pls = playlist::all(&st.db).map_err(|e| e.to_string())?;
                let pl = pls.iter().find(|p| p.id == id);
                let name = pl.map(|p| p.name.clone()).unwrap_or_default();
                let hero = pl
                    .and_then(|p| p.image_hash.as_ref().map(hexhash::encode))
                    .or_else(|| {
                        playlist::cover_hashes(&st.db, id)
                            .ok()
                            .and_then(|c| c.first().map(hexhash::encode))
                    });
                ("playlist", name, tracks_label(total), hero)
            }
            Source::Artist(a) => {
                let name = library::artists(&st.db)
                    .ok()
                    .and_then(|v| v.into_iter().find(|x| x.id == a).map(|x| x.name))
                    .unwrap_or_default();
                let hero = library::rows(&st.db, &st.view[..total.min(1)])
                    .ok()
                    .and_then(|mut r| r.pop())
                    .and_then(|r| r.art_hash)
                    .as_ref()
                    .map(hexhash::encode);
                ("artist", name, tracks_label(total), hero)
            }
        };

    Ok(OpenResult {
        total,
        kind,
        title,
        subtitle,
        hero_art,
    })
}

fn tracks_label(n: usize) -> String {
    if n == 1 {
        "1 track".to_string()
    } else {
        format!("{n} tracks")
    }
}

/// Uma janela da view atual, resolvida em linhas de exibição.
#[tauri::command]
pub fn track_rows(state: St<'_>, start: usize, count: usize) -> Result<Vec<TrackRowDto>, String> {
    let st = state.lock().expect("estado do app");
    let end = start.saturating_add(count).min(st.view.len());
    let ids: &[player_core::TrackId] = if start < end {
        &st.view[start..end]
    } else {
        &[]
    };
    library::rows(&st.db, ids)
        .map(|rows| rows.into_iter().map(TrackRowDto::from).collect())
        .map_err(|e| e.to_string())
}

/// Abre o seletor nativo de pasta; se o usuário escolher, aponta a biblioteca
/// pra lá e dispara o scan.
#[tauri::command]
pub async fn pick_folder(app: AppHandle, state: St<'_>) -> Result<Option<String>, String> {
    let Some(fp) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path: PathBuf = fp.into_path().map_err(|e| e.to_string())?;
    state
        .lock()
        .expect("estado do app")
        .set_root(&app, path.clone());
    Ok(Some(path.to_string_lossy().into_owned()))
}

// ---- playback ---------------------------------------------------------------

/// Toca a faixa no índice `index` da view atual (substitui a fila por ela).
#[tauri::command]
pub fn play_at(state: St<'_>, index: usize) -> Result<PlaybackDto, String> {
    let mut st = state.lock().expect("estado do app");
    st.play_at(index)?;
    Ok(playback::snapshot(&st))
}

#[tauri::command]
pub fn play_pause(state: St<'_>) -> Result<PlaybackDto, String> {
    let mut st = state.lock().expect("estado do app");
    st.toggle_play()?;
    Ok(playback::snapshot(&st))
}

#[tauri::command]
pub fn next_track(state: St<'_>) -> Result<PlaybackDto, String> {
    let mut st = state.lock().expect("estado do app");
    st.next_track()?;
    Ok(playback::snapshot(&st))
}

#[tauri::command]
pub fn prev_track(state: St<'_>) -> Result<PlaybackDto, String> {
    let mut st = state.lock().expect("estado do app");
    st.prev_track()?;
    Ok(playback::snapshot(&st))
}

#[tauri::command]
pub fn seek(state: St<'_>, ms: u64) -> PlaybackDto {
    let st = state.lock().expect("estado do app");
    st.engine.seek(std::time::Duration::from_millis(ms));
    playback::snapshot(&st)
}

#[tauri::command]
pub fn set_volume(state: St<'_>, volume: f32) {
    state.lock().expect("estado do app").set_volume(volume);
}

#[tauri::command]
pub fn set_shuffle(state: St<'_>, on: bool) -> PlaybackDto {
    let mut st = state.lock().expect("estado do app");
    st.queue.set_shuffle(on);
    st.queue_next();
    playback::snapshot(&st)
}

/// Cicla o modo de repetição: desligado → tudo → uma → desligado.
#[tauri::command]
pub fn cycle_repeat(state: St<'_>) -> PlaybackDto {
    let mut st = state.lock().expect("estado do app");
    let next = st.queue.repeat().next();
    st.queue.set_repeat(next);
    st.queue_next();
    playback::snapshot(&st)
}

/// Estado de reprodução agora — pra hidratar o front na subida.
#[tauri::command]
pub fn playback_snapshot(state: St<'_>) -> PlaybackDto {
    playback::snapshot(&state.lock().expect("estado do app"))
}

/// Arquivos de "abrir com" esperando o scan — o front chama isto no
/// `scan://done`. Sem nada pendente, no-op.
#[tauri::command]
pub fn flush_pending_play(state: St<'_>) -> Result<Option<PlaybackDto>, String> {
    let mut st = state.lock().expect("estado do app");
    if st.pending_play.is_empty() {
        return Ok(None);
    }
    let files = std::mem::take(&mut st.pending_play);
    st.play_files(&files)?;
    Ok(Some(playback::snapshot(&st)))
}

/// Reindexação manual da pasta atual.
#[tauri::command]
pub fn rescan(app: AppHandle, state: St<'_>) -> Result<(), String> {
    let (db_path, cache, root, guard) = {
        let st = state.lock().expect("estado do app");
        let root = st.root.clone().ok_or("nenhuma pasta escolhida")?;
        (
            st.paths.db.clone(),
            st.paths.cache.clone(),
            root,
            std::sync::Arc::clone(&st.loudness_running),
        )
    };
    scan::spawn(app, db_path, cache, root, guard);
    Ok(())
}

// ---- playlists: escrita -----------------------------------------------------

#[tauri::command]
pub fn playlist_create(state: St<'_>, name: Option<String>) -> Result<PlaylistDto, String> {
    let st = state.lock().expect("estado do app");
    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "New playlist".to_string());
    let id = playlist::create(&st.db, &name).map_err(|e| e.to_string())?;
    let pls = playlist::all(&st.db).map_err(|e| e.to_string())?;
    let pl = pls
        .iter()
        .find(|p| p.id == id)
        .ok_or("playlist recém-criada sumiu")?;
    Ok(dto::playlist_dto(pl, &[], false))
}

#[tauri::command]
pub fn playlist_rename(state: St<'_>, id: String, name: String) -> Result<(), String> {
    let st = state.lock().expect("estado do app");
    playlist::rename(&st.db, parse_uuid(&id)?, name.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn playlist_delete(state: St<'_>, id: String) -> Result<(), String> {
    let st = state.lock().expect("estado do app");
    playlist::delete(&st.db, parse_uuid(&id)?).map_err(|e| e.to_string())
}

/// Adiciona faixas ao fim de uma playlist. Devolve quantas entraram.
#[tauri::command]
pub fn playlist_add_tracks(state: St<'_>, id: String, tracks: Vec<i64>) -> Result<usize, String> {
    let uuid = parse_uuid(&id)?;
    let ids: Vec<player_core::TrackId> = tracks.into_iter().map(player_core::TrackId).collect();
    let mut st = state.lock().expect("estado do app");
    playlist::append(&mut st.db, uuid, &ids).map_err(|e| e.to_string())
}

/// Remove o item no índice `index` da view atual (que tem de ser uma playlist).
#[tauri::command]
pub fn playlist_remove_at(state: St<'_>, index: usize) -> Result<(), String> {
    let st = state.lock().expect("estado do app");
    let Source::Playlist(id) = st.source else {
        return Err("a fonte atual não é uma playlist".into());
    };
    let pos = st
        .view_positions
        .get(index)
        .ok_or("índice fora da playlist")?
        .clone();
    playlist::remove(&st.db, id, &pos).map_err(|e| e.to_string())
}

/// Move o item de `from` pra o índice `to` na view atual (playlist).
#[tauri::command]
pub fn playlist_move(state: St<'_>, from: usize, to: usize) -> Result<(), String> {
    let st = state.lock().expect("estado do app");
    let Source::Playlist(id) = st.source else {
        return Err("a fonte atual não é uma playlist".into());
    };
    let pos = st
        .view_positions
        .get(from)
        .ok_or("índice fora da playlist")?
        .clone();
    playlist::move_item(&st.db, id, &pos, to).map_err(|e| e.to_string())
}

// ---- imagens (playlist / biblioteca / capa de álbum) ----------------------

#[tauri::command]
pub async fn playlist_set_image(app: AppHandle, state: St<'_>, id: String) -> Result<(), String> {
    let uuid = parse_uuid(&id)?;
    let Some(fp) = app
        .dialog()
        .file()
        .add_filter("Image", IMAGE_EXT)
        .blocking_pick_file()
    else {
        return Ok(());
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let st = state.lock().expect("estado do app");
    let hash = st.image_into_cache(&path).ok_or("imagem inválida")?;
    playlist::set_image(&st.db, uuid, Some(&hash)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn playlist_clear_image(state: St<'_>, id: String) -> Result<(), String> {
    let st = state.lock().expect("estado do app");
    playlist::set_image(&st.db, parse_uuid(&id)?, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn library_set_image(app: AppHandle, state: St<'_>) -> Result<Option<String>, String> {
    let Some(fp) = app
        .dialog()
        .file()
        .add_filter("Image", IMAGE_EXT)
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let mut st = state.lock().expect("estado do app");
    let hash = st.image_into_cache(&path).ok_or("imagem inválida")?;
    st.db
        .conn()
        .execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (META_LIBRARY_IMAGE, hexhash::encode(&hash)),
        )
        .map_err(|e| e.to_string())?;
    st.library_image = Some(hash);
    Ok(Some(hexhash::encode(&hash)))
}

#[tauri::command]
pub fn library_clear_image(state: St<'_>) -> Result<(), String> {
    let mut st = state.lock().expect("estado do app");
    st.db
        .conn()
        .execute("DELETE FROM meta WHERE key = ?1", [META_LIBRARY_IMAGE])
        .map_err(|e| e.to_string())?;
    st.library_image = None;
    Ok(())
}

/// Hash hex da foto da biblioteca (pra a linha "Your Library" da sidebar).
#[tauri::command]
pub fn library_image(state: St<'_>) -> Option<String> {
    state
        .lock()
        .expect("estado do app")
        .library_image
        .as_ref()
        .map(hexhash::encode)
}

#[tauri::command]
pub async fn track_set_album_art(app: AppHandle, state: St<'_>, id: i64) -> Result<(), String> {
    let Some(fp) = app
        .dialog()
        .file()
        .add_filter("Image", IMAGE_EXT)
        .blocking_pick_file()
    else {
        return Ok(());
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let st = state.lock().expect("estado do app");
    player_core::art::set_album_art(&st.db, &st.art, player_core::TrackId(id), &bytes)
        .map_err(|e| e.to_string())?
        .ok_or("imagem inválida".into())
        .map(|_| ())
}

// ---- pasta vinculada ------------------------------------------------------

#[tauri::command]
pub fn playlist_links(state: St<'_>, id: String) -> Result<Vec<LinkDto>, String> {
    let st = state.lock().expect("estado do app");
    let links = playlist_folder::links_for(&st.db, parse_uuid(&id)?).map_err(|e| e.to_string())?;
    Ok(links
        .into_iter()
        .map(|l| LinkDto {
            label: if l.rel_prefix.is_empty() {
                "Unlink whole folder".to_string()
            } else {
                format!("Unlink \"{}\"", l.rel_prefix)
            },
            root_id: l.root_id,
            rel_prefix: l.rel_prefix,
        })
        .collect())
}

#[tauri::command]
pub async fn playlist_link_folder(app: AppHandle, state: St<'_>, id: String) -> Result<(), String> {
    let uuid = parse_uuid(&id)?;
    let Some(fp) = app.dialog().file().blocking_pick_folder() else {
        return Ok(());
    };
    let folder = fp.into_path().map_err(|e| e.to_string())?;
    let mut st = state.lock().expect("estado do app");
    match playlist_folder::link(&st.db, uuid, &folder).map_err(|e| e.to_string())? {
        Ok(()) => {
            playlist_folder::sync_all(&mut st.db).map_err(|e| e.to_string())?;
            Ok(())
        }
        Err(player_core::playlist_folder::ForaDaBiblioteca) => {
            Err("essa pasta está fora da biblioteca atual".into())
        }
    }
}

#[tauri::command]
pub fn playlist_unlink_folder(
    state: St<'_>,
    id: String,
    root_id: i64,
    rel_prefix: String,
) -> Result<(), String> {
    let st = state.lock().expect("estado do app");
    playlist_folder::unlink(&st.db, parse_uuid(&id)?, root_id, &rel_prefix)
        .map_err(|e| e.to_string())
}
