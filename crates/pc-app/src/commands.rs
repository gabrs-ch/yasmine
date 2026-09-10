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

use player_core::{library, playlist, playlist_folder};

use crate::dto::{
    self, ArtistDto, OpenResult, PlaylistDto, SortArg, SourceArg, StatsDto, TrackRowDto,
};
use crate::hexhash;
use crate::scan;
use crate::state::{AppState, Source};

type St<'a> = State<'a, Mutex<AppState>>;

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

    let ids = match src {
        Source::Library if query.trim().is_empty() => library::view(&st.db, sort_core),
        Source::Library => library::search(&st.db, &query, sort_core),
        Source::Playlist(id) => playlist::tracks(&st.db, id),
        Source::Artist(a) => library::by_artist(&st.db, a, sort_core),
    }
    .map_err(|e| e.to_string())?;

    st.source = src;
    st.sort = sort;
    st.query = query;
    st.view = ids;
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

/// Reindexação manual da pasta atual.
#[tauri::command]
pub fn rescan(app: AppHandle, state: St<'_>) -> Result<(), String> {
    let (db_path, cache, root) = {
        let st = state.lock().expect("estado do app");
        let root = st.root.clone().ok_or("nenhuma pasta escolhida")?;
        (st.paths.db.clone(), st.paths.cache.clone(), root)
    };
    scan::spawn(app, db_path, cache, root);
    Ok(())
}
