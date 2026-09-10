//! Ponta a ponta: um `Server` de verdade sobre loopback e `client::pull`
//! contra ele, com duas bibliotecas em pastas temporárias e MP3 com tag.
//!
//! Cobre o caminho que motiva a Fase 4: parear, baixar a biblioteca inteira,
//! e as playlists encaixarem por `track_key` do outro lado.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use lofty::config::WriteOptions;
use lofty::prelude::Accessor;
use lofty::tag::{Tag, TagExt, TagType};
use player_core::{ArtCache, Db};
use yasmine_sync::{AuthDecision, Identity, Server, ServerEvent, Target, pull};

const FRAME_LEN: usize = 417;
const FRAME_HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];

/// ~1 s de silêncio em frames MPEG-1 Layer III válidos (mesma técnica do
/// `player_core::testutil`).
fn mp3_silencioso() -> Vec<u8> {
    let mut out = vec![0u8; 39 * FRAME_LEN];
    let mut i = 0;
    while i + FRAME_LEN <= out.len() {
        out[i..i + 4].copy_from_slice(&FRAME_HEADER);
        i += FRAME_LEN;
    }
    out
}

fn escreve_mp3(dir: &Path, rel: &str, titulo: &str, artista: &str, album: &str, faixa: u32) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().expect("pai")).expect("mkdir");
    std::fs::write(&path, mp3_silencioso()).expect("gravar mp3");

    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title(titulo.to_owned());
    tag.set_artist(artista.to_owned());
    tag.set_album(album.to_owned());
    tag.set_track(faixa);
    tag.save_to_path(&path, WriteOptions::default())
        .expect("gravar tag");
}

struct Fixture {
    _tmp: tempfile::TempDir,
    host_db_path: std::path::PathBuf,
    host_pub: yasmine_sync::DeviceId,
    phone_db: Db,
    phone_id: Identity,
    phone_music: std::path::PathBuf,
    phone_cache: std::path::PathBuf,
}

/// Monta o host (5 faixas, 1 playlist com as 2 primeiras) e o celular vazio.
fn fixture(n_faixas: u32) -> Fixture {
    let tmp = tempfile::tempdir().expect("tmpdir");

    let host_music = tmp.path().join("host/musica");
    let host_db_path = tmp.path().join("host/library.db");
    let host_cache = tmp.path().join("host/cache");
    std::fs::create_dir_all(&host_music).expect("mkdir host");
    for i in 0..n_faixas {
        escreve_mp3(
            &host_music,
            &format!("Legiao Urbana/Dois/{i:02}.mp3"),
            &format!("Faixa {i}"),
            "Legião Urbana",
            "Dois",
            i + 1,
        );
    }

    let mut host_db = Db::open(&host_db_path).expect("abrir host db");
    let host_id = Identity::load_or_create(&mut host_db, "PC do Gabriel").expect("id host");
    let host_pub = host_id.device_id();
    player_core::scan(&mut host_db, &host_music, &ArtCache::new(host_cache)).expect("scan host");

    let ids = player_core::library::view(&host_db, player_core::Sort::default()).expect("view");
    let pl = player_core::playlist::create(&host_db, "Favoritas").expect("playlist");
    player_core::playlist::append(&mut host_db, pl, &ids[..2]).expect("append");
    drop(host_db);

    let phone_dir = tmp.path().join("phone");
    std::fs::create_dir_all(&phone_dir).expect("mkdir phone");
    let mut phone_db = Db::open(&phone_dir.join("library.db")).expect("abrir phone db");
    let phone_id = Identity::load_or_create(&mut phone_db, "Celular").expect("id phone");

    Fixture {
        _tmp: tmp,
        host_db_path,
        host_pub,
        phone_db,
        phone_id,
        phone_music: phone_dir.join("Musica"),
        phone_cache: phone_dir.join("cache"),
    }
}

fn start_server(db_path: &Path, accept: Option<yasmine_sync::DeviceId>) -> Server {
    let mut host_db = Db::open(db_path).expect("reabrir host db");
    let host_id = Identity::load_or_create(&mut host_db, "PC do Gabriel").expect("id host");
    drop(host_db);

    Server::bind(
        "127.0.0.1:0",
        host_id,
        db_path.to_path_buf(),
        move |id, _name| match accept {
            Some(allowed) if id == allowed => AuthDecision::Accept,
            None => AuthDecision::Accept,
            _ => AuthDecision::Reject,
        },
        |_ev: ServerEvent| {},
    )
    .expect("subir server")
}

#[test]
fn baixa_biblioteca_inteira_e_playlists_encaixam() {
    let mut fx = fixture(5);
    let server = start_server(&fx.host_db_path, Some(fx.phone_id.device_id()));
    let addr = server.addr();

    let cancel = AtomicBool::new(false);
    let report = pull(
        &mut fx.phone_db,
        &fx.phone_id,
        Target::Addr {
            addr,
            id: fx.host_pub,
        },
        &fx.phone_music,
        &fx.phone_cache,
        &|_p| {},
        &cancel,
    )
    .expect("pull");

    assert_eq!(report.tracks_added, 5, "faltou baixar faixa");
    assert_eq!(report.hash_mismatch, 0, "arquivo corrompido no caminho");
    assert_eq!(report.playlists_merged, 1);

    let stats = player_core::library::stats(&fx.phone_db).expect("stats");
    assert_eq!(stats.tracks, 5, "o scan não indexou a pasta baixada");

    let pls = player_core::playlist::all(&fx.phone_db).expect("playlists");
    assert_eq!(pls.len(), 1);
    assert_eq!(pls[0].name, "Favoritas");
    let faixas = player_core::playlist::tracks(&fx.phone_db, pls[0].id).expect("tracks");
    assert_eq!(faixas.len(), 2, "itens da playlist não resolveram por hash");

    let pareado: i64 = fx
        .phone_db
        .conn()
        .query_row(
            "SELECT count(*) FROM device WHERE is_self = 0 AND paired_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("consultar device");
    assert!(pareado >= 1, "o host não ficou registrado como pareado");

    // Arquivos de verdade em disco, sob a árvore artista/álbum.
    let n_mp3 = walk_count(&fx.phone_music, "mp3");
    assert_eq!(n_mp3, 5);

    server.shutdown();
}

#[test]
fn retoma_apos_cancelar_no_meio() {
    let mut fx = fixture(6);
    let server = start_server(&fx.host_db_path, Some(fx.phone_id.device_id()));
    let addr = server.addr();

    // Cancela assim que a 2ª faixa termina.
    let done = AtomicUsize::new(0);
    let cancel = AtomicBool::new(false);
    let target = Target::Addr {
        addr,
        id: fx.host_pub,
    };
    let err = pull(
        &mut fx.phone_db,
        &fx.phone_id,
        target,
        &fx.phone_music,
        &fx.phone_cache,
        &|p| {
            if p.tracks_done >= 2 && done.swap(1, Ordering::Relaxed) == 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    )
    .expect_err("deveria cancelar");
    assert!(
        matches!(err, yasmine_sync::Error::Cancelled),
        "veio {err:?}"
    );

    let parciais_antes = walk_count(&fx.phone_music, "mp3");
    assert!(parciais_antes < 6, "cancelou tarde demais pro teste valer");

    // Segunda passada: sem cancelar, conclui reaproveitando o que já veio.
    let cancel2 = AtomicBool::new(false);
    let report = pull(
        &mut fx.phone_db,
        &fx.phone_id,
        target,
        &fx.phone_music,
        &fx.phone_cache,
        &|_p| {},
        &cancel2,
    )
    .expect("segundo pull");

    assert!(
        report.tracks_added < 6,
        "rebaixou tudo em vez de retomar ({} faixas)",
        report.tracks_added
    );
    assert_eq!(
        player_core::library::stats(&fx.phone_db)
            .expect("stats")
            .tracks,
        6
    );
    assert_eq!(walk_count(&fx.phone_music, "mp3"), 6);

    server.shutdown();
}

#[test]
fn recusa_device_nao_pareado() {
    let mut fx = fixture(2);
    // Aceita só uma chave aleatória — nunca a do celular.
    let server = start_server(&fx.host_db_path, Some(yasmine_sync::DeviceId([0xAB; 32])));
    let addr = server.addr();

    let cancel = AtomicBool::new(false);
    let err = pull(
        &mut fx.phone_db,
        &fx.phone_id,
        Target::Addr {
            addr,
            id: fx.host_pub,
        },
        &fx.phone_music,
        &fx.phone_cache,
        &|_p| {},
        &cancel,
    )
    .expect_err("não pareado deve falhar");
    // O host responde com Msg::Error após o handshake.
    assert!(
        matches!(err, yasmine_sync::Error::Protocol(_)),
        "veio {err:?}"
    );

    server.shutdown();
}

fn walk_count(root: &Path, ext: &str) -> usize {
    let mut n = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
                n += 1;
            }
        }
    }
    n
}
