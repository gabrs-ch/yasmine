//! Lógica compartilhada entre a CLI e a GUI: opções, abrir o índice, montar o
//! payload do QR, e a política de pareamento.

use std::io::{BufRead, Write};
use std::net::{IpAddr, UdpSocket};
use std::path::PathBuf;

use player_core::db::now_ms;
use player_core::{ArtCache, Db};
use yasmine_sync::{AuthDecision, DeviceId, Identity, PairPayload, Server, ServerEvent};

pub struct Options {
    pub db_path: PathBuf,
    pub cache_dir: PathBuf,
    pub music: Option<PathBuf>,
    pub port: u16,
    pub name: String,
    pub mdns: bool,
    pub gui: bool,
}

impl Options {
    pub const USAGE: &'static str = "\
uso: yasmine-sync-host [opções]
  --db <arquivo>     índice SQLite (padrão: pasta de dados do Yasmine)
  --music <pasta>    varre esta pasta antes de servir (primeira vez)
  --port <n>         porta TCP (padrão: 0 = o SO escolhe)
  --name <texto>     nome deste PC no pareamento (padrão: hostname)
  --mdns             anuncia na LAN por mDNS (dispensa digitar IP no celular)
  --gui              abre janela com o QR grande (requer build --features gui)
  -h, --help";

    pub fn from_args() -> Result<Self, String> {
        let dirs = directories::ProjectDirs::from("", "", "Yasmine")
            .ok_or("não achei a pasta de dados do usuário")?;
        let mut o = Self {
            db_path: dirs.data_dir().join("library.db"),
            cache_dir: dirs.cache_dir().to_path_buf(),
            music: None,
            port: 0,
            name: hostname(),
            mdns: false,
            gui: false,
        };

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut val = || args.next().ok_or(format!("{arg} precisa de um valor"));
            match arg.as_str() {
                "--db" => o.db_path = PathBuf::from(val()?),
                "--music" => o.music = Some(PathBuf::from(val()?)),
                "--port" => {
                    o.port = val()?
                        .parse()
                        .map_err(|_| "--port precisa ser um número".to_string())?;
                }
                "--name" => o.name = val()?,
                "--mdns" => o.mdns = true,
                "--gui" => o.gui = true,
                "-h" | "--help" => return Err("ajuda:".into()),
                outro => return Err(format!("opção desconhecida: {outro}")),
            }
        }
        Ok(o)
    }
}

/// Estado vivo do host: o servidor rodando e o payload do QR já pronto.
pub struct HostSession {
    pub pair_url: String,
    pub server: Server,
}

impl HostSession {
    /// Abre o índice, escaneia se pedido, sobe o servidor e monta o QR.
    /// `on_event` recebe o andamento das conexões.
    pub fn start(
        opts: &Options,
        on_event: impl Fn(ServerEvent) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        if let Some(parent) = opts.db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut db = Db::open(&opts.db_path).map_err(|e| e.to_string())?;
        let identity = Identity::load_or_create(&mut db, &opts.name).map_err(|e| e.to_string())?;

        if let Some(music) = &opts.music {
            let art = ArtCache::new(opts.cache_dir.clone());
            let report =
                player_core::scan(&mut db, music, &art).map_err(|e| format!("scan: {e}"))?;
            eprintln!(
                "indexado: {} nova(s), {} atualizada(s), {} intacta(s)",
                report.added, report.updated, report.unchanged
            );
        }

        let roots: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM library_root", [], |r| r.get(0))
            .unwrap_or(0);
        if roots == 0 {
            return Err("o índice está vazio — rode uma vez com --music <pasta>".into());
        }
        drop(db);

        let db_path = opts.db_path.clone();
        let server = Server::bind(
            ("0.0.0.0", opts.port),
            identity.clone(),
            db_path.clone(),
            move |id, name| pairing_policy(&db_path, id, name),
            on_event,
        )
        .map_err(|e| e.to_string())?;

        let mut this = Self {
            pair_url: String::new(),
            server,
        };
        if opts.mdns {
            this.server
                .advertise(&opts.name)
                .map_err(|e| e.to_string())?;
        }

        let host_ip = lan_ip();
        let payload = PairPayload {
            key: identity.public(),
            host: host_ip.map(|ip| ip.to_string()),
            port: Some(this.server.addr().port()),
            name: opts.name.clone(),
        };
        this.pair_url = payload.to_url();
        Ok(this)
    }
}

/// Aceita device já pareado sem perguntar; para um novo, pergunta no stdin e,
/// no "sim", grava a linha `device` (uma conexão curta e própria, pra não
/// disputar a do servidor).
fn pairing_policy(db_path: &std::path::Path, id: DeviceId, name: &str) -> AuthDecision {
    if let Ok(db) = Db::open(db_path) {
        let known: bool = db
            .conn()
            .query_row(
                "SELECT 1 FROM device WHERE id = ?1 AND paired_at IS NOT NULL",
                [id.as_bytes().as_slice()],
                |_| Ok(()),
            )
            .is_ok();
        if known {
            return AuthDecision::Accept;
        }
    }

    print!("\nParear com \"{name}\" ({id})? [s/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return AuthDecision::Reject;
    }
    if !matches!(line.trim(), "s" | "S" | "sim" | "y" | "Y") {
        println!("recusado.");
        return AuthDecision::Reject;
    }

    if let Ok(db) = Db::open(db_path) {
        let _ = db.conn().execute(
            "INSERT INTO device (id, name, is_self, paired_at) VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, paired_at = excluded.paired_at",
            rusqlite::params![id.as_bytes().as_slice(), name, now_ms()],
        );
    }
    println!("pareado.");
    AuthDecision::Accept
}

pub fn run_cli(opts: Options) -> Result<(), String> {
    let session = HostSession::start(&opts, |ev| log_event(&ev))?;

    println!("\n╭─ Yasmine · sync host");
    println!("│  {}", opts.name);
    println!("│  escutando em {}", session.server.addr());
    if opts.mdns {
        println!("│  anunciando na LAN (_yasmine-sync._tcp)");
    }
    println!("╰─ aponte o celular pro QR:\n");
    println!("{}", qr_ascii(&session.pair_url));
    println!("{}\n", session.pair_url);
    println!("Ctrl-C encerra.");

    // O servidor roda nas próprias threads; aqui só seguramos o processo vivo.
    // Ctrl-C (SIGINT) derruba tudo — o mDNS desregistra no melhor esforço.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

fn log_event(ev: &ServerEvent) {
    match ev {
        ServerEvent::Listening(addr) => eprintln!("· escutando {addr}"),
        ServerEvent::PeerConnected { name, .. } => eprintln!("· {name} conectou"),
        ServerEvent::Sending { peer, done, total } => {
            let pct = if *total > 0 { done * 100 / total } else { 100 };
            eprint!("\r· enviando pra {peer}: {pct}%   ");
            let _ = std::io::stderr().flush();
        }
        ServerEvent::PeerFinished { .. } => eprintln!("\r· concluído              "),
        ServerEvent::ConnectionError(e) => eprintln!("\r! {e}                     "),
    }
}

/// QR em blocos unicode (2 módulos por caractere na vertical).
pub fn qr_ascii(data: &str) -> String {
    use qrcode::QrCode;
    use qrcode::render::unicode;
    match QrCode::new(data.as_bytes()) {
        Ok(code) => code
            .render::<unicode::Dense1x2>()
            .quiet_zone(true)
            .module_dimensions(1, 1)
            .build(),
        Err(_) => "(QR grande demais para o terminal — use a URL abaixo)".to_string(),
    }
}

/// IP de saída pra LAN, sem mandar pacote nenhum: só pergunta ao SO por qual
/// interface uma rota externa sairia.
fn lan_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.168.0.1:9")
        .or_else(|_| sock.connect("8.8.8.8:80"))
        .ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_unspecified() || ip.is_loopback() {
        None
    } else {
        Some(ip)
    }
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
