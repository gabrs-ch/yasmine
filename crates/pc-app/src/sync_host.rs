//! Host do sync embutido no app: o QR e o servidor que serve a biblioteca
//! pro celular, dentro do processo do Tauri em vez de um binário à parte
//! (`yasmine-sync-host` continua existindo pra quem quiser rodar sem UI).
//!
//! Estado gerenciado à parte do `AppState`: o servidor roda em threads
//! próprias e não deve prender o lock que o loop de playback pega 4×/s.

use std::net::{IpAddr, UdpSocket};
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use player_core::Db;
use player_core::db::now_ms;
use yasmine_sync::{AuthDecision, DeviceId, Identity, PairPayload, Server, ServerEvent};

use crate::state::AppState;

/// Estado do host. `None` = parado.
#[derive(Default)]
pub struct SyncHost(pub Mutex<Option<Running>>);

pub struct Running {
    server: Server,
    pair_url: String,
    qr_svg: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInfo {
    pub running: bool,
    pub pair_url: Option<String>,
    /// SVG completo do QR (o front joga num `dangerouslySetInnerHTML`).
    pub qr_svg: Option<String>,
}

impl SyncInfo {
    fn from(r: &Running) -> Self {
        Self {
            running: true,
            pair_url: Some(r.pair_url.clone()),
            qr_svg: Some(r.qr_svg.clone()),
        }
    }
    fn stopped() -> Self {
        Self {
            running: false,
            pair_url: None,
            qr_svg: None,
        }
    }
}

/// Espelho de `ServerEvent` pro front (`sync://event`).
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum SyncEvent {
    Listening,
    PeerConnected { name: String },
    Sending { peer: String, done: u64, total: u64 },
    PeerFinished,
    Error { msg: String },
}

impl From<ServerEvent> for SyncEvent {
    fn from(e: ServerEvent) -> Self {
        match e {
            ServerEvent::Listening(_) => Self::Listening,
            ServerEvent::PeerConnected { name, .. } => Self::PeerConnected { name },
            ServerEvent::Sending { peer, done, total } => Self::Sending { peer, done, total },
            ServerEvent::PeerFinished { .. } => Self::PeerFinished,
            ServerEvent::ConnectionError(msg) => Self::Error { msg },
        }
    }
}

/// Sobe o servidor (idempotente) e devolve o QR.
pub fn start(app: &AppHandle) -> Result<SyncInfo, String> {
    let host = app.state::<SyncHost>();
    let mut slot = host.0.lock().expect("sync host");
    if let Some(r) = slot.as_ref() {
        return Ok(SyncInfo::from(r));
    }

    let name = hostname();
    let (db_path, identity) = {
        let state = app.state::<Mutex<AppState>>();
        let mut st = state.lock().expect("estado do app");
        let roots: i64 = st
            .db
            .conn()
            .query_row("SELECT count(*) FROM library_root", [], |r| r.get(0))
            .unwrap_or(0);
        if roots == 0 {
            return Err("Escolha uma pasta de música antes de sincronizar.".into());
        }
        // A chave estática Noise mora no próprio índice; criar é idempotente.
        let id = Identity::load_or_create(&mut st.db, &name).map_err(|e| e.to_string())?;
        (st.paths.db.clone(), id)
    };

    let auth_db = db_path.clone();
    let ev_app = app.clone();
    let mut server = Server::bind(
        ("0.0.0.0", 0),
        identity.clone(),
        db_path,
        move |id, name| auto_pair(&auth_db, id, name),
        move |ev| {
            let _ = ev_app.emit("sync://event", SyncEvent::from(ev));
        },
    )
    .map_err(|e| e.to_string())?;
    // mDNS é melhor-esforço: sem ele, o QR ainda carrega host:porta.
    let _ = server.advertise(&name);

    let payload = PairPayload {
        key: identity.public(),
        host: lan_ip().map(|ip| ip.to_string()),
        port: Some(server.addr().port()),
        name,
    };
    let pair_url = payload.to_url();
    let qr_svg = qr_svg(&pair_url);

    let running = Running {
        server,
        pair_url,
        qr_svg,
    };
    let info = SyncInfo::from(&running);
    *slot = Some(running);
    Ok(info)
}

pub fn stop(app: &AppHandle) {
    if let Some(r) = app.state::<SyncHost>().0.lock().expect("sync host").take() {
        r.server.shutdown();
    }
}

pub fn info(app: &AppHandle) -> SyncInfo {
    app.state::<SyncHost>()
        .0
        .lock()
        .expect("sync host")
        .as_ref()
        .map_or_else(SyncInfo::stopped, SyncInfo::from)
}

/// O QR aqui é o próprio grant: quem escaneou está autorizado. Aceita e grava
/// a linha `device` (conexão curta e própria, pra não disputar a do servidor).
fn auto_pair(db_path: &Path, id: DeviceId, name: &str) -> AuthDecision {
    if let Ok(db) = Db::open(db_path) {
        let _ = db.conn().execute(
            "INSERT INTO device (id, name, is_self, paired_at) VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                paired_at = COALESCE(device.paired_at, excluded.paired_at)",
            rusqlite::params![id.as_bytes().as_slice(), name, now_ms()],
        );
    }
    AuthDecision::Accept
}

fn qr_svg(data: &str) -> String {
    use qrcode::QrCode;
    use qrcode::render::svg;
    QrCode::new(data.as_bytes()).map_or_else(
        |_| String::new(),
        |code| {
            code.render::<svg::Color<'_>>()
                .min_dimensions(232, 232)
                .quiet_zone(true)
                .dark_color(svg::Color("#0b0b0f"))
                .light_color(svg::Color("#ffffff"))
                .build()
        },
    )
}

/// IP de saída pra LAN, sem mandar pacote: só pergunta ao SO por qual
/// interface uma rota externa sairia. (Cópia do `yasmine-sync-host`.)
fn lan_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.168.0.1:9")
        .or_else(|_| sock.connect("8.8.8.8:80"))
        .ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "Yasmine PC".to_string())
}
