//! Lado do host (PC): escuta TCP, faz o handshake, e serve a biblioteca —
//! **só leitura** da camada do usuário e dos arquivos de áudio. A única
//! escrita que o servidor faz no índice é preencher `content_hash` das
//! próprias faixas (cache derivado, e o host é dono dos próprios arquivos):
//! sem os hashes ele não tem como responder "o que eu tenho que você não".
//!
//! Uma conexão por vez basta pro caso de uso (um celular pareando com um PC),
//! mas cada conexão roda na própria thread, então dois celulares ao mesmo
//! tempo também funcionam.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use player_core::{Db, DeviceId};

use crate::channel::Channel;
use crate::discovery::{self, Advertisement};
use crate::identity::Identity;
use crate::merge;
use crate::protocol::{CHUNK, Msg, TrackMeta};
use crate::{Error, PROTO_VERSION, Result};

/// Prazos de rede e teto de conexões. Sem eles, qualquer um na LAN abre
/// conexões, não fala nada, e segura uma thread do host em cada uma.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// O caso de uso é um celular por vez; o teto existe para que a LAN não
/// consiga esgotar as threads do host.
const MAX_CONNS: usize = 4;

/// O que o host decide sobre um device que acabou de se apresentar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthDecision {
    Accept,
    Reject,
}

#[derive(Debug, Clone)]
pub enum ServerEvent {
    Listening(SocketAddr),
    PeerConnected { id: DeviceId, name: String },
    Sending { peer: String, done: u64, total: u64 },
    PeerFinished { id: DeviceId },
    ConnectionError(String),
}

pub struct Server {
    addr: SocketAddr,
    device_id: DeviceId,
    shutdown: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    advert: Option<Advertisement>,
}

impl Server {
    /// Sobe o servidor. `auth` é chamado (podendo bloquear pra perguntar ao
    /// usuário) quando um device se apresenta; `on_event` recebe o andamento.
    pub fn bind<A, F, G>(
        addr: A,
        identity: Identity,
        db_path: PathBuf,
        auth: F,
        on_event: G,
    ) -> Result<Self>
    where
        A: ToSocketAddrs,
        F: Fn(DeviceId, &str) -> AuthDecision + Send + Sync + 'static,
        G: Fn(ServerEvent) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let device_id = identity.device_id();

        let shutdown = Arc::new(AtomicBool::new(false));
        let auth = Arc::new(auth);
        let on_event = Arc::new(on_event);
        on_event(ServerEvent::Listening(addr));

        let sd = shutdown.clone();
        let accept = thread::spawn(move || {
            let identity = Arc::new(identity);
            let vivas = Arc::new(AtomicUsize::new(0));
            loop {
                if sd.load(Ordering::Relaxed) {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _peer)) => {
                        if vivas.load(Ordering::SeqCst) >= MAX_CONNS {
                            drop(stream);
                            on_event(ServerEvent::ConnectionError(
                                "conexões demais ao mesmo tempo; recusada".into(),
                            ));
                            continue;
                        }
                        vivas.fetch_add(1, Ordering::SeqCst);
                        let identity = identity.clone();
                        let db_path = db_path.clone();
                        let auth = auth.clone();
                        let ev = on_event.clone();
                        let vivas = vivas.clone();
                        thread::spawn(move || {
                            if let Err(e) =
                                serve_conn(stream, &identity, &db_path, auth.as_ref(), ev.as_ref())
                            {
                                ev(ServerEvent::ConnectionError(e.to_string()));
                            }
                            vivas.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(100)),
                }
            }
        });

        Ok(Self {
            addr,
            device_id,
            shutdown,
            accept: Some(accept),
            advert: None,
        })
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Liga o anúncio mDNS (`_yasmine-sync._tcp`). Opcional: o QR pode
    /// carregar `host:porta` e dispensar isto.
    pub fn advertise(&mut self, name: &str) -> Result<()> {
        self.advert = Some(discovery::advertise(
            &self.device_id,
            self.addr.port(),
            name,
        )?);
        Ok(())
    }

    pub fn shutdown(mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.advert = None;
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn serve_conn<F, G>(
    stream: TcpStream,
    identity: &Identity,
    db_path: &Path,
    auth: &F,
    ev: &G,
) -> Result<()>
where
    F: Fn(DeviceId, &str) -> AuthDecision,
    G: Fn(ServerEvent),
{
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    // Cópia do descritor só para mexer nos prazos depois que o `Channel`
    // consumir o stream. O handshake é máquina com máquina, daí o aperto; a
    // sessão em si tem folga para uma transferência longa.
    let ctl = stream.try_clone()?;
    ctl.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    ctl.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;

    let (mut ch, peer_static) = Channel::responder(stream, identity)?;
    let peer_id = DeviceId(peer_static);
    ctl.set_read_timeout(Some(IDLE_TIMEOUT))?;
    ctl.set_write_timeout(Some(IDLE_TIMEOUT))?;

    let name = match ch.recv()? {
        Msg::Hello { proto, device_name } if proto == PROTO_VERSION => device_name,
        Msg::Hello { proto, .. } => {
            return Err(Error::Protocol(format!(
                "o par fala a versão {proto} do protocolo, este host fala a {PROTO_VERSION}"
            )));
        }
        other => return Err(Error::Protocol(format!("esperava Hello, veio {other:?}"))),
    };

    if auth(peer_id, &name) == AuthDecision::Reject {
        let _ = ch.send(&Msg::Error {
            msg: "device não pareado".into(),
        });
        return Err(Error::Unpaired(peer_id));
    }
    ev(ServerEvent::PeerConnected {
        id: peer_id,
        name: name.clone(),
    });

    let mut db = Db::open(db_path)?;
    let host_name: String = db
        .conn()
        .query_row("SELECT name FROM device WHERE is_self = 1", [], |r| {
            r.get(0)
        })
        .unwrap_or_else(|_| "Yasmine PC".to_string());
    ch.send(&Msg::Hello {
        proto: PROTO_VERSION,
        device_name: host_name,
    })?;

    // Camada do usuário primeiro: pequena, e a biblioteca do celular já
    // aparece povoada enquanto o áudio não chegou.
    ch.send(&Msg::User(merge::snapshot(&db)?))?;

    // Diff endereçado por conteúdo.
    let mine = local_hashes(&mut db)?;
    let theirs: std::collections::HashSet<[u8; 32]> = match ch.recv()? {
        Msg::Have { hashes } => hashes.into_iter().collect(),
        other => return Err(Error::Protocol(format!("esperava Have, veio {other:?}"))),
    };
    let missing: Vec<TrackMeta> = mine
        .iter()
        .filter(|(h, _)| !theirs.contains(h))
        .map(|(h, id)| track_meta(&db, h, *id))
        .collect::<Result<_>>()?;
    ch.send(&Msg::Tracks(missing))?;

    // Serve blobs sob demanda.
    loop {
        match ch.recv()? {
            Msg::NeedBlob { hash, from } => send_blob(&mut ch, &db, &hash, from, &name, ev)?,
            Msg::Done => break,
            Msg::Error { msg } => return Err(Error::Protocol(msg)),
            other => return Err(Error::Protocol(format!("inesperado: {other:?}"))),
        }
    }
    ev(ServerEvent::PeerFinished { id: peer_id });
    Ok(())
}

/// Hashes de conteúdo de toda a biblioteca local, com o `TrackId` pra achar o
/// arquivo depois. Força `ensure_hashes` — é o custo inerente de comparar
/// bibliotecas por conteúdo (a otimização por `(size, mtime)` fica pra
/// depois).
fn local_hashes(db: &mut Db) -> Result<Vec<([u8; 32], player_core::TrackId)>> {
    let ids = player_core::library::view(db, player_core::Sort::default())?;
    let map = player_core::hash::ensure_hashes(db, &ids)?;
    Ok(map.into_iter().map(|(id, key)| (key.0, id)).collect())
}

fn track_meta(db: &Db, hash: &[u8; 32], id: player_core::TrackId) -> Result<TrackMeta> {
    let row = db.conn().query_row(
        "SELECT t.title, ar.name, al.title, aa.name,
                t.disc_no, t.track_no, t.year, t.genre, t.rel_path, t.file_size
         FROM track t
         LEFT JOIN artist ar ON ar.id = t.artist_id
         LEFT JOIN album  al ON al.id = t.album_id
         LEFT JOIN artist aa ON aa.id = t.album_artist_id
         WHERE t.id = ?1",
        [id.0],
        |r| {
            let rel_path: String = r.get(8)?;
            Ok(TrackMeta {
                hash: *hash,
                ext: Path::new(&rel_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase(),
                size: r.get::<_, i64>(9)? as u64,
                title: r.get(0)?,
                artist: r.get(1)?,
                album: r.get(2)?,
                album_artist: r.get(3)?,
                disc_no: r.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                track_no: r.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                year: r.get::<_, Option<i64>>(6)?.map(|v| v as i32),
                genre: r.get(7)?,
            })
        },
    )?;
    Ok(row)
}

fn send_blob<S: std::io::Read + std::io::Write, G: Fn(ServerEvent)>(
    ch: &mut Channel<S>,
    db: &Db,
    hash: &[u8; 32],
    from: u64,
    peer: &str,
    ev: &G,
) -> Result<()> {
    let (root, rel, size): (String, String, i64) = db.conn().query_row(
        "SELECT r.path, t.rel_path, t.file_size
         FROM track t JOIN library_root r ON r.id = t.root_id
         WHERE t.content_hash = ?1 LIMIT 1",
        [hash.as_slice()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let total = size as u64;
    let path = Path::new(&root).join(rel);
    let mut file = File::open(&path)?;
    if from > 0 && from <= total {
        file.seek(SeekFrom::Start(from))?;
    }

    let mut sent = from.min(total);
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        let last = n == 0 || sent + n as u64 >= total;
        ch.send(&Msg::Blob {
            hash: *hash,
            offset: sent,
            data: buf[..n].to_vec(),
            last,
        })?;
        sent += n as u64;
        ev(ServerEvent::Sending {
            peer: peer.to_string(),
            done: sent,
            total,
        });
        if last {
            break;
        }
    }
    Ok(())
}
