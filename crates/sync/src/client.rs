//! Lado do celular: conecta, faz merge da camada do usuário, baixa os
//! arquivos que faltam e roda o `scan` na pasta baixada — que daí em diante é
//! um `library_root` normal. Depois disso o celular tem biblioteca própria e
//! toca offline.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use data_encoding::HEXLOWER;
use player_core::{ArtCache, Db, DeviceId};

use crate::channel::Channel;
use crate::identity::Identity;
use crate::protocol::{Msg, TrackMeta};
use crate::{Error, PROTO_VERSION, Result, discovery, merge};

/// Prazos de rede. Sem eles um host que abre a conexão e emudece deixa o app
/// pendurado para sempre — e o `cancel`, que só é lido entre chunks, nunca
/// chega a ser consultado.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// O host pergunta ao usuário do PC se aceita parear **depois** do handshake,
/// então a resposta ao primeiro `Hello` espera por um humano.
const PAIR_TIMEOUT: Duration = Duration::from_secs(180);
const IO_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    MergingUserData,
    FetchingList,
    Downloading,
    Indexing,
    Done,
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub phase: Phase,
    pub tracks_done: u64,
    pub tracks_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PullReport {
    pub peer: DeviceId,
    pub tracks_added: u64,
    pub bytes: u64,
    pub playlists_merged: usize,
    /// Arquivos que chegaram mas não bateram com o hash pedido — descartados.
    pub hash_mismatch: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum Target {
    /// Endereço já conhecido (QR trouxe `host:porta`, ou IP digitado à mão).
    Addr { addr: SocketAddr, id: DeviceId },
    /// Só a chave: resolve o endereço por mDNS.
    Discovered(DeviceId),
}

/// Baixa a biblioteca inteira do `target` para `dest`, fazendo merge da camada
/// do usuário. `cache_dir` é onde as miniaturas de capa são geradas (o `scan`
/// cuida disso). `cancel` interrompe entre chunks — o que já baixou fica, e
/// uma chamada seguinte retoma de onde parou.
pub fn pull(
    db: &mut Db,
    identity: &Identity,
    target: Target,
    dest: &Path,
    cache_dir: &Path,
    on_progress: &dyn Fn(Progress),
    cancel: &AtomicBool,
) -> Result<PullReport> {
    let (addr, expected) = resolve(&target)?;

    let mut p = Progress {
        phase: Phase::Connecting,
        tracks_done: 0,
        tracks_total: 0,
        bytes_done: 0,
        bytes_total: 0,
        current: None,
    };
    on_progress(p.clone());

    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true)?;
    // Cópia do descritor só para mexer nos prazos: o `Channel` consome o
    // stream, e o prazo certo muda conforme a fase (handshake é máquina com
    // máquina, o `Hello` de volta espera o humano do PC confirmar).
    let ctl = stream.try_clone()?;
    ctl.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    ctl.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;

    // A verificação da chave do host contra a do QR é o próprio handshake IK:
    // se a estática não bate, o `read_message` de `msg2` falha.
    let mut ch = Channel::initiator(stream, identity, expected.as_bytes())?;

    ctl.set_read_timeout(Some(PAIR_TIMEOUT))?;
    ch.send(&Msg::Hello {
        proto: PROTO_VERSION,
        device_name: self_name(db),
    })?;
    match ch.recv()? {
        Msg::Hello { proto, .. } if proto == PROTO_VERSION => {}
        Msg::Hello { proto, .. } => {
            return Err(Error::Protocol(format!(
                "o host fala a versão {proto} do protocolo, este app fala a {PROTO_VERSION}"
            )));
        }
        Msg::Error { msg } => return Err(Error::Protocol(msg)),
        other => return Err(Error::Protocol(format!("esperava Hello, veio {other:?}"))),
    }
    ctl.set_read_timeout(Some(IO_TIMEOUT))?;
    ctl.set_write_timeout(Some(IO_TIMEOUT))?;

    // --- camada do usuário ---
    p.phase = Phase::MergingUserData;
    on_progress(p.clone());
    let layer = match ch.recv()? {
        Msg::User(l) => l,
        other => return Err(Error::Protocol(format!("esperava User, veio {other:?}"))),
    };
    let playlists_merged = merge::apply(db, &layer, expected)?;

    // --- diff ---
    p.phase = Phase::FetchingList;
    on_progress(p.clone());
    ch.send(&Msg::Have {
        hashes: local_hashes(db)?,
    })?;
    let metas = match ch.recv()? {
        Msg::Tracks(m) => m,
        other => return Err(Error::Protocol(format!("esperava Tracks, veio {other:?}"))),
    };

    // --- download ---
    p.phase = Phase::Downloading;
    p.tracks_total = metas.len() as u64;
    // `size` vem do par: saturar em vez de estourar o u64 do somatório.
    p.bytes_total = metas.iter().fold(0u64, |acc, m| acc.saturating_add(m.size));
    on_progress(p.clone());

    fs::create_dir_all(dest)?;
    let incoming = dest.join(".incoming");
    fs::create_dir_all(&incoming)?;

    let mut report = PullReport {
        peer: expected,
        tracks_added: 0,
        bytes: 0,
        playlists_merged,
        hash_mismatch: 0,
    };
    let mut bytes_base = 0u64;

    for (i, m) in metas.iter().enumerate() {
        check_cancel(cancel)?;
        let final_path = target_path(dest, m);
        if !dentro_de(dest, &final_path) {
            return Err(Error::Protocol(
                "o host pediu um destino fora da pasta da biblioteca".into(),
            ));
        }

        if final_path.exists()
            && fs::metadata(&final_path).map(|md| md.len()).unwrap_or(0) == m.size
        {
            bytes_base += m.size;
            p.tracks_done = (i + 1) as u64;
            p.bytes_done = bytes_base;
            on_progress(p.clone());
            continue;
        }

        let part = incoming.join(format!("{}.part", HEXLOWER.encode(&m.hash)));
        let mut from = fs::metadata(&part).map(|md| md.len()).unwrap_or(0);
        if from > m.size {
            let _ = fs::remove_file(&part);
            from = 0;
        }
        if from == 0 && part.exists() {
            let _ = fs::remove_file(&part);
        }

        p.current = Some(display_name(m));
        ch.send(&Msg::NeedBlob { hash: m.hash, from })?;

        let mut file = OpenOptions::new().create(true).append(true).open(&part)?;
        let mut got = from;
        loop {
            check_cancel(cancel)?;
            match ch.recv()? {
                Msg::Blob {
                    hash, data, last, ..
                } => {
                    if hash != m.hash {
                        return Err(Error::Protocol("blob de hash inesperado".into()));
                    }
                    // Quem fala quando acaba é o `last` do host; sem este teto
                    // um host hostil escreve até encher o armazenamento.
                    if got.saturating_add(data.len() as u64) > m.size {
                        return Err(Error::Protocol(
                            "o host mandou mais bytes que o tamanho que anunciou".into(),
                        ));
                    }
                    file.write_all(&data)?;
                    got += data.len() as u64;
                    p.bytes_done = bytes_base + got;
                    on_progress(p.clone());
                    if last {
                        break;
                    }
                }
                Msg::Error { msg } => return Err(Error::Protocol(msg)),
                other => return Err(Error::Protocol(format!("esperava Blob, veio {other:?}"))),
            }
        }
        file.flush()?;
        drop(file);

        if blake3_file(&part)? != m.hash {
            let _ = fs::remove_file(&part);
            report.hash_mismatch += 1;
            continue;
        }
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&part, &final_path)?;
        report.tracks_added += 1;
        report.bytes += m.size;
        bytes_base += got;

        p.tracks_done = (i + 1) as u64;
        on_progress(p.clone());
    }

    ch.send(&Msg::Done)?;
    let _ = fs::remove_dir(&incoming); // some se estiver vazio

    // --- indexação ---
    p.phase = Phase::Indexing;
    p.current = None;
    on_progress(p.clone());
    let art = ArtCache::new(cache_dir.to_path_buf());
    player_core::scan(db, dest, &art)?;
    // Os itens de playlist apontam por `track_key`; hashear as faixas novas
    // faz os buracos da lista virarem faixas tocáveis.
    let ids = player_core::library::view(db, player_core::Sort::default())?;
    player_core::hash::ensure_hashes(db, &ids)?;

    p.phase = Phase::Done;
    on_progress(p);
    Ok(report)
}

fn resolve(target: &Target) -> Result<(SocketAddr, DeviceId)> {
    match *target {
        Target::Addr { addr, id } => Ok((addr, id)),
        Target::Discovered(id) => {
            let addr = discovery::resolve_once(&id, Duration::from_secs(6))?
                .ok_or_else(|| Error::Mdns("o device não apareceu na LAN".into()))?;
            Ok((addr, id))
        }
    }
}

fn local_hashes(db: &mut Db) -> Result<Vec<[u8; 32]>> {
    let ids = player_core::library::view(db, player_core::Sort::default())?;
    let map = player_core::hash::ensure_hashes(db, &ids)?;
    let mut v: Vec<[u8; 32]> = map.into_values().map(|k| k.0).collect();
    v.sort_unstable();
    Ok(v)
}

fn self_name(db: &Db) -> String {
    db.conn()
        .query_row("SELECT name FROM device WHERE is_self = 1", [], |r| {
            r.get(0)
        })
        .unwrap_or_else(|_| "celular".to_string())
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

fn display_name(m: &TrackMeta) -> String {
    match (&m.artist, &m.title) {
        (Some(a), Some(t)) => format!("{a} — {t}"),
        (_, Some(t)) => t.clone(),
        _ => HEXLOWER.encode(&m.hash[..6]),
    }
}

/// `dest/<artista>/<álbum>/<NN título>.<ext>`, tudo higienizado. Sem tags
/// suficientes, cai pra `dest/<hex>.<ext>` — nada fica sem lugar.
fn target_path(dest: &Path, m: &TrackMeta) -> PathBuf {
    let artist = sanitize(
        m.album_artist
            .as_deref()
            .or(m.artist.as_deref())
            .unwrap_or("Sem artista"),
    );
    let album = sanitize(m.album.as_deref().unwrap_or("Sem álbum"));
    let ext = sanitize_ext(&m.ext);

    let stem = match m.title.as_deref() {
        Some(t) if !t.trim().is_empty() => {
            let prefix = m.track_no.map(|n| format!("{n:02} ")).unwrap_or_default();
            format!("{prefix}{}", sanitize(t))
        }
        _ => return dest.join(format!("{}.{ext}", HEXLOWER.encode(&m.hash[..16]))),
    };
    dest.join(artist).join(album).join(format!("{stem}.{ext}"))
}

/// A extensão vem do fio e é colada crua no nome do arquivo, então tem de ser
/// um token — não um caminho. Sem isto um `ext` com `../` escapa do `dest`
/// mesmo com `stem`, artista e álbum higienizados.
fn sanitize_ext(ext: &str) -> &str {
    let ok = (1..=8).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric());
    if ok { ext } else { "bin" }
}

/// O destino montado está mesmo dentro de `dest`? `Path::starts_with` é
/// lexical e deixa `..` passar, então não serve sozinho: aqui exigimos que a
/// parte relativa não tenha nada além de componentes normais.
fn dentro_de(dest: &Path, path: &Path) -> bool {
    path.strip_prefix(dest).is_ok_and(|rel| {
        rel.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    })
}

fn sanitize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\x1f' => '_',
            c => c,
        })
        .collect();
    let trimmed = out.trim().trim_matches('.').to_string();
    out = if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed
    };
    if out.chars().count() > 120 {
        out = out.chars().take(120).collect();
    }
    out
}

fn blake3_file(path: &Path) -> Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 128 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    use crate::channel::Channel;
    use crate::protocol::{Msg, UserLayer};

    fn meta(hash: [u8; 32], ext: &str, size: u64) -> TrackMeta {
        TrackMeta {
            hash,
            ext: ext.into(),
            size,
            title: Some("Faixa".into()),
            artist: Some("Artista".into()),
            album: Some("Album".into()),
            album_artist: None,
            disc_no: None,
            track_no: Some(1),
            year: None,
            genre: None,
        }
    }

    /// Um "host" que fala o protocolo direitinho mas mente na metadata. É o
    /// que o celular tem de aguentar: o pareamento autentica *quem* fala, não
    /// garante que o que ele fala seja honesto.
    fn host_malicioso(
        m: TrackMeta,
        corpo: Vec<u8>,
        pedacos: usize,
    ) -> (SocketAddr, DeviceId, thread::JoinHandle<()>) {
        let id = Identity::ephemeral();
        let device_id = id.device_id();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let h = thread::spawn(move || {
            let (sock, _) = listener.accept().expect("accept");
            let (mut ch, _peer) = Channel::responder(sock, &id).expect("responder");
            let _ = ch.recv(); // Hello do celular
            let _ = ch.send(&Msg::Hello {
                proto: crate::PROTO_VERSION,
                device_name: "host hostil".into(),
            });
            let _ = ch.send(&Msg::User(UserLayer::default()));
            let _ = ch.recv(); // Have
            let _ = ch.send(&Msg::Tracks(vec![m.clone()]));
            if ch.recv().is_err() {
                return; // NeedBlob
            }
            for i in 0..pedacos {
                let blob = Msg::Blob {
                    hash: m.hash,
                    offset: 0,
                    data: corpo.clone(),
                    last: i + 1 == pedacos,
                };
                if ch.send(&blob).is_err() {
                    return; // o celular cortou — é o que esperamos num dos testes
                }
            }
            let _ = ch.recv(); // Done
        });
        (addr, device_id, h)
    }

    fn celular(raiz: &Path) -> (Db, Identity, PathBuf) {
        let mut db = Db::open_in_memory().expect("índice em memória");
        let id = Identity::load_or_create(&mut db, "Celular").expect("identidade");
        let cache = raiz.join("cache");
        fs::create_dir_all(&cache).expect("mkdir cache");
        (db, id, cache)
    }

    fn arquivos(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(rd) = fs::read_dir(root) else {
            return out;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(arquivos(&p));
            } else {
                out.push(p);
            }
        }
        out
    }

    #[test]
    fn ext_malicioso_nao_escapa_da_pasta() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        // Aninhado de propósito: os `..` do `ext` abaixo sobem menos níveis do
        // que isto, então um escape cai dentro do tempdir e o teste o vê (sem
        // este cuidado o arquivo vaza pra fora e a asserção não pega nada).
        let dest = tmp.path().join("a/b/c/d/musica");
        let corpo = b"audio falso".to_vec();
        let hash: [u8; 32] = blake3::hash(&corpo).into();
        let alvo = "mp3/../../../../../fora.txt";
        let (addr, host_id, h) = host_malicioso(meta(hash, alvo, corpo.len() as u64), corpo, 1);

        let (mut db, id, cache) = celular(tmp.path());
        let rel = pull(
            &mut db,
            &id,
            Target::Addr { addr, id: host_id },
            &dest,
            &cache,
            &|_| {},
            &AtomicBool::new(false),
        )
        .expect("pull");
        let _ = h.join();

        assert_eq!(
            rel.tracks_added, 1,
            "a faixa devia entrar, só que com nome seguro"
        );
        let escapou = arquivos(tmp.path())
            .into_iter()
            .filter(|p| !p.starts_with(&dest) && !p.starts_with(&cache))
            .collect::<Vec<_>>();
        assert!(escapou.is_empty(), "escreveu fora do dest: {escapou:?}");

        let dentro = arquivos(&dest);
        assert_eq!(dentro.len(), 1, "esperava um arquivo só: {dentro:?}");
        assert_eq!(
            dentro[0].extension().and_then(|e| e.to_str()),
            Some("bin"),
            "a extensão do fio não foi neutralizada: {:?}",
            dentro[0]
        );
    }

    #[test]
    fn host_que_manda_mais_bytes_que_o_anunciado_e_cortado() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let dest = tmp.path().join("musica");
        let corpo = vec![0u8; 64 * 1024];
        let hash: [u8; 32] = blake3::hash(&corpo).into();
        // Anuncia 4 bytes e tenta despejar 8 × 64 KiB sem nunca marcar `last`.
        let (addr, host_id, h) = host_malicioso(meta(hash, "mp3", 4), corpo, 8);

        let (mut db, id, cache) = celular(tmp.path());
        let err = pull(
            &mut db,
            &id,
            Target::Addr { addr, id: host_id },
            &dest,
            &cache,
            &|_| {},
            &AtomicBool::new(false),
        )
        .expect_err("devia ter recusado o excesso");
        let _ = h.join();

        assert!(
            err.to_string().contains("mais bytes"),
            "erro inesperado: {err}"
        );
        let gravado: u64 = arquivos(&dest)
            .iter()
            .filter_map(|p| p.metadata().ok())
            .map(|md| md.len())
            .sum();
        assert!(gravado <= 4, "gravou {gravado} B para um anúncio de 4 B");
    }
}
