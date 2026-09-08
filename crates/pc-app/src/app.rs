//! A aplicação.
//!
//! # Política de repaint
//!
//! Metade do custo de CPU de um player parado vem de redesenhar à toa. Aqui:
//!
//! - parado, a janela só redesenha quando acontece algo (modo reativo do egui);
//! - tocando, redesenha a ~4 Hz — o suficiente para a barra de progresso andar;
//! - escaneando, a ~10 Hz, para o contador não parecer travado.
//!
//! Nunca 60 fps.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, pos2, vec2};
use player_audio::{Engine, Event};
use player_core::library::{self, Sort, Stats, TrackRow};
use player_core::playlist::{self, Playlist};
use player_core::scan::scan_with_progress;
use player_core::{ArtCache, Db, TrackId};
use uuid::Uuid;

use crate::art::ArtLoader;
use crate::paths::Paths;
use crate::queue::{Queue, Repeat};
use crate::theme;
use crate::watcher::Watcher;

/// Chave no `meta` onde a pasta escolhida fica guardada, para a próxima
/// execução abrir direto na biblioteca.
const META_ROOT: &str = "library_root";

/// Chave no `meta` onde o volume mestre fica guardado entre execuções.
const META_VOLUME: &str = "volume";

/// Ganho linear a aplicar numa faixa: o do nivelador se já foi medido, 1.0
/// (sem ajuste) se a tarefa de fundo ainda não chegou nela. Faixa nova nunca
/// espera a medição para tocar — só ganha o nivelamento na vez seguinte.
fn track_gain(info: &library::PlaybackInfo) -> f32 {
    match (info.gain_db, info.peak) {
        (Some(gain_db), Some(peak)) => {
            player_audio::linear_gain(player_audio::Loudness { gain_db, peak })
        }
        _ => 1.0,
    }
}

/// Mede o nivelador de todas as faixas pendentes, em lotes pequenos, numa
/// conexão própria — roda numa thread de fundo enquanto o app continua
/// respondendo normalmente (WAL permite a leitura da UI e esta escrita
/// convivendo).
///
/// Faixa que falha ao analisar (arquivo corrompido, formato de borda) recebe
/// ganho neutro em vez de ficar pendente para sempre: sem isso, um arquivo
/// ruim faria esta função tentar ele de novo em todo scan, indefinidamente.
fn run_loudness_fill(db_path: &std::path::Path) {
    const BATCH: usize = 16;

    let Ok(db) = player_core::Db::open(db_path) else {
        return;
    };

    loop {
        let Ok(batch) = player_core::loudness::pending(&db, BATCH) else {
            return;
        };
        if batch.is_empty() {
            return;
        }

        for item in batch {
            let loudness =
                player_audio::loudness::analyze(&item.path).unwrap_or(player_audio::Loudness {
                    gain_db: 0.0,
                    peak: 1.0,
                });
            let _ = player_core::loudness::set(&db, item.id, loudness.gain_db, loudness.peak);
        }
    }
}

/// Tamanho da janela no modo compacto. Largo o bastante pra capa, título,
/// artista e os três botões de transporte não se atropelarem; nada além.
const MINI_SIZE: egui::Vec2 = egui::Vec2::new(340.0, 112.0);

/// Tamanho mínimo da janela normal — abaixo disso a lista de faixas não
/// cabe de um jeito legível. Compartilhado com `main.rs`, que usa o mesmo
/// valor para configurar a janela na primeira abertura.
pub const NORMAL_MIN_SIZE: egui::Vec2 = egui::Vec2::new(620.0, 380.0);

/// De onde a lista visível vem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Library,
    Playlist(Uuid),
}

struct ScanJob {
    progress: Arc<AtomicUsize>,
    result: Receiver<Result<player_core::ScanReport, String>>,
    started: Instant,
}

pub struct App {
    /// Guardado para acordar a janela a partir de outra thread.
    ctx: egui::Context,
    db: Db,
    paths: Paths,
    root: Option<PathBuf>,

    source: Source,
    view: Vec<TrackId>,
    /// Posição de cada item de `view` na playlist ativa — paralelo a `view`,
    /// vazio quando a fonte é a biblioteca. É o que permite "remover da
    /// playlist" e "mover" sem uma segunda ida ao banco por linha.
    view_positions: Vec<String>,
    query: String,
    sort: Sort,
    stats: Stats,
    playlists: Vec<Playlist>,
    /// Playlist em edição de nome: (id, texto do campo, pedir foco).
    renaming: Option<(Uuid, String, bool)>,

    engine: Engine,
    art: ArtLoader,
    /// A marca do Yasmine, carregada uma vez do PNG embutido no binário.
    /// Só aparece na tela de boas-vindas — pequena demais e única o
    /// bastante pra não precisar do sistema de carregamento sob demanda do
    /// `ArtLoader`.
    mark: egui::TextureHandle,

    /// Índice na `view` da linha selecionada.
    selected: Option<usize>,
    queue: Queue,
    /// Faixa tocando, por id — **não** por índice.
    ///
    /// A fila e a lista visível divergem assim que o usuário busca algo, e um
    /// índice guardado passaria a apontar para outra faixa. O id não.
    now_id: Option<TrackId>,
    now: Option<TrackRow>,

    scan: Option<ScanJob>,
    /// Chegou mudança do disco enquanto um scan já rodava.
    rescan_pending: bool,
    /// Arquivos que vieram de "abrir com" do gerenciador de arquivos,
    /// esperando o scan (disparado por `set_root`) terminar de indexar a
    /// pasta antes de virarem `TrackId` e tocarem. `None` no caminho normal
    /// — abrir sem argumento nenhum, ou só uma pasta pra apontar a
    /// biblioteca sem tocar nada.
    pending_play: Option<Vec<PathBuf>>,
    watcher: Option<Watcher>,
    status: String,
    focus_search: bool,

    /// Modo compacto: só capa, transporte e progresso, numa janela pequena
    /// o bastante para ficar num canto da tela sem competir por espaço.
    mini: bool,
    /// Tamanho da janela antes de entrar no modo compacto, para restaurar
    /// exatamente o que o usuário tinha — não um tamanho padrão qualquer.
    normal_size: egui::Vec2,

    /// Uma tarefa de nivelamento já está rodando em segundo plano. Evita
    /// empilhar uma tarefa nova a cada rescan enquanto a anterior ainda não
    /// terminou de cobrir uma biblioteca grande.
    loudness_running: Arc<AtomicBool>,
}

/// O que a linha de comando pediu pra abrir.
///
/// "Abrir com" do gerenciador de arquivos entrega um ou mais caminhos de
/// *arquivo* (nunca uma pasta) — um clique numa faixa só manda um argumento,
/// selecionar várias e abrir todas de uma vez manda vários. Abrir a partir
/// de um atalho ou da linha de comando com uma pasta continua valendo,
/// exatamente como antes.
enum Opened {
    Nothing,
    /// Só aponta a biblioteca pra cá — comportamento de sempre, nada toca
    /// sozinho.
    Folder(PathBuf),
    /// Aponta a biblioteca pra pasta que contém os arquivos e toca eles,
    /// nessa ordem, assim que o scan terminar de indexá-la.
    Files {
        folder: PathBuf,
        files: Vec<PathBuf>,
    },
}

impl Opened {
    fn from_args(args: &[PathBuf]) -> Self {
        if args.len() == 1 && args[0].is_dir() {
            return Self::Folder(args[0].clone());
        }
        let files: Vec<PathBuf> = args.iter().filter(|p| p.is_file()).cloned().collect();
        let Some(folder) = files
            .first()
            .and_then(|f| f.parent())
            .map(Path::to_path_buf)
        else {
            return Self::Nothing;
        };
        Self::Files { folder, files }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: &[PathBuf]) -> Result<Self, String> {
        theme::apply(&cc.egui_ctx);

        let paths = Paths::resolve().map_err(|err| err.to_string())?;
        let db = Db::open(&paths.db).map_err(|err| err.to_string())?;

        let root: Option<PathBuf> = db
            .conn()
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [META_ROOT],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .map(PathBuf::from);

        let mark = {
            let bytes = include_bytes!("../assets/icon-256.png");
            let rgba = image::load_from_memory(bytes)
                .expect("assets/icon-256.png embutido no binário deveria ser válido")
                .to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            cc.egui_ctx
                .load_texture("marca", color, egui::TextureOptions::LINEAR)
        };

        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            art: ArtLoader::new(paths.cache.clone()),
            mark,
            db,
            paths,
            root,
            source: Source::Library,
            view: Vec::new(),
            view_positions: Vec::new(),
            query: String::new(),
            sort: Sort::ArtistAlbum,
            stats: Stats::default(),
            playlists: Vec::new(),
            renaming: None,
            engine: Engine::new(),
            selected: None,
            queue: Queue::default(),
            now_id: None,
            now: None,
            scan: None,
            rescan_pending: false,
            pending_play: None,
            watcher: None,
            status: String::new(),
            focus_search: false,
            mini: false,
            normal_size: vec2(1000.0, 660.0),
            loudness_running: Arc::new(AtomicBool::new(false)),
        };

        // O volume mestre é a única preferência que o app lembra — e não é
        // "configuração" no sentido que este projeto evita: é o mesmo tipo
        // de memória que qualquer player tem, tão básica quanto lembrar a
        // pasta escolhida.
        let saved_volume: f32 = app
            .db
            .conn()
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [META_VOLUME],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        app.engine.set_volume(saved_volume);

        app.reload();
        match Opened::from_args(args) {
            // Pasta vinda da linha de comando manda sobre a guardada.
            Opened::Folder(folder) => app.set_root(folder),
            // "Abrir com" de um ou mais arquivos: aponta a biblioteca pra
            // pasta deles (mesmo caminho de sempre — indexar é indexar) e
            // guarda quais tocar assim que o scan terminar.
            Opened::Files { folder, files } => {
                app.set_root(folder);
                app.pending_play = Some(files);
            }
            // Reescaneia a pasta guardada em segundo plano. Um rescan de
            // biblioteca intacta custa décimos de segundo e não abre arquivo
            // nenhum, então sai mais barato que pedir ao usuário que clique
            // em "Reescanear" — e músicas novas simplesmente aparecem.
            Opened::Nothing => {
                if let Some(root) = app.root.clone() {
                    app.watch(&root);
                    app.start_scan(root);
                }
            }
        }
        Ok(app)
    }

    // -------------------------------------------------------------------------
    // Biblioteca
    // -------------------------------------------------------------------------

    fn reload(&mut self) {
        match self.source {
            Source::Library => {
                self.view = library::search(&self.db, &self.query, self.sort).unwrap_or_default();
                self.view_positions.clear();
            }
            Source::Playlist(id) => {
                // Uma consulta só: dá tanto a lista de faixas tocáveis quanto
                // a posição de cada uma, sem duas idas ao banco.
                let items = playlist::items(&self.db, id).unwrap_or_default();
                self.view = Vec::with_capacity(items.len());
                self.view_positions = Vec::with_capacity(items.len());
                for item in items {
                    if let Some(track) = item.track {
                        self.view.push(track);
                        self.view_positions.push(item.position);
                    }
                }
            }
        }
        self.stats = library::stats(&self.db).unwrap_or_default();
        self.playlists = playlist::all(&self.db).unwrap_or_default();
        self.selected = None;
    }

    /// Troca a fonte da lista (biblioteca ou uma playlist) e recarrega.
    fn set_source(&mut self, source: Source) {
        if self.source == source {
            return;
        }
        self.source = source;
        self.reload();
    }

    /// Cria uma playlist e a deixa pronta para o usuário nomear.
    ///
    /// `track`, quando presente, já entra na lista nova — é o caso de "Nova
    /// playlist…" a partir do menu de contexto de uma faixa.
    fn create_playlist(&mut self, track: Option<TrackId>) {
        let Ok(id) = playlist::create(&self.db, "Nova playlist") else {
            return;
        };
        if let Some(track) = track {
            let _ = playlist::append(&mut self.db, id, &[track]);
        }
        self.playlists = playlist::all(&self.db).unwrap_or_default();
        self.renaming = Some((id, "Nova playlist".to_owned(), true));
        self.set_source(Source::Playlist(id));
    }

    fn commit_rename(&mut self) {
        let Some((id, name, _)) = self.renaming.take() else {
            return;
        };
        let name = name.trim();
        if !name.is_empty() {
            let _ = playlist::rename(&self.db, id, name);
        }
        self.playlists = playlist::all(&self.db).unwrap_or_default();
    }

    fn delete_playlist(&mut self, id: Uuid) {
        let _ = playlist::delete(&self.db, id);
        if self.source == Source::Playlist(id) {
            self.set_source(Source::Library);
        } else {
            self.playlists = playlist::all(&self.db).unwrap_or_default();
        }
    }

    /// Acrescenta `track` ao fim de uma playlist. Hasheia sob demanda — é a
    /// primeira vez que este arquivo precisa de identidade entre devices.
    fn add_to_playlist(&mut self, playlist_id: Uuid, track: TrackId) {
        let _ = playlist::append(&mut self.db, playlist_id, &[track]);
        if self.source == Source::Playlist(playlist_id) {
            self.reload();
        } else {
            self.playlists = playlist::all(&self.db).unwrap_or_default();
        }
    }

    /// Remove um item pela posição — só faz sentido com uma playlist ativa.
    fn remove_from_playlist(&mut self, index: usize) {
        let Source::Playlist(id) = self.source else {
            return;
        };
        let Some(position) = self.view_positions.get(index).cloned() else {
            return;
        };
        let _ = playlist::remove(&self.db, id, &position);
        self.reload();
    }

    /// Move um item uma posição para cima ou para baixo na playlist ativa.
    ///
    /// `move_item` reconstrói a lista como `antes ++ [item] ++ depois`, então
    /// o `to` que passamos já É o índice final do item na lista resultante —
    /// não precisa compensar a remoção.
    fn nudge_in_playlist(&mut self, index: usize, delta: isize) {
        let Source::Playlist(id) = self.source else {
            return;
        };
        let Some(position) = self.view_positions.get(index).cloned() else {
            return;
        };
        let target = index as isize + delta;
        if target < 0 || target as usize >= self.view.len() {
            return;
        }
        let _ = playlist::move_item(&self.db, id, &position, target as usize);
        self.reload();
    }

    /// Aponta a biblioteca para `folder`, guarda a escolha e escaneia.
    fn set_root(&mut self, folder: PathBuf) {
        let _ = self.db.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (META_ROOT, folder.to_string_lossy()),
        );
        // Uma pasta por vez: apontar outra esquece a anterior, senão a
        // contagem de faixas soma bibliotecas que o usuário não vê mais.
        let _ = player_core::keep_only_root(&self.db, &folder);
        self.watch(&folder);
        self.root = Some(folder.clone());
        self.queue.clear();
        self.now_id = None;
        self.now = None;
        self.selected = None;
        self.engine.stop();
        self.start_scan(folder);
    }

    fn pick_folder(&mut self) {
        // O diálogo nativo é modal e bloqueia esta thread — que é o
        // comportamento certo: não há nada a desenhar enquanto ele está aberto.
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Escolha a pasta de música")
            .pick_folder()
        else {
            return;
        };

        self.set_root(folder);
    }

    /// Deixa o usuário escolher uma imagem do disco pra capa do álbum de
    /// `track`. Vale pra todas as faixas do álbum, não só essa — é o álbum
    /// que carrega a capa no índice.
    fn pick_album_art(&mut self, track: TrackId) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Escolha uma imagem para a capa")
            .add_filter("Imagem", &["jpg", "jpeg", "png", "webp", "bmp", "gif"])
            .pick_file()
        else {
            return;
        };

        let Ok(bytes) = std::fs::read(&path) else {
            self.status = format!("não consegui ler {}", path.display());
            return;
        };

        let cache = ArtCache::new(self.paths.cache.clone());
        match player_core::art::set_album_art(&self.db, &cache, track, &bytes) {
            Ok(Some(_)) => self.reload(),
            Ok(None) => self.status = "esse arquivo não é uma imagem válida".into(),
            Err(err) => self.status = format!("não consegui salvar a capa: {err}"),
        }
    }

    /// Vincula uma playlist a uma pasta escolhida pelo usuário: toda faixa
    /// que já está (ou vier a entrar, no próximo scan) dentro dela passa a
    /// fazer parte da playlist sozinha.
    fn link_playlist_folder(&mut self, playlist_id: Uuid) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Escolha a pasta para vincular à playlist")
            .pick_folder()
        else {
            return;
        };

        match player_core::playlist_folder::link(&self.db, playlist_id, &folder) {
            Ok(Ok(())) => {
                let _ = player_core::playlist_folder::sync_all(&mut self.db);
                self.playlists = playlist::all(&self.db).unwrap_or_default();
                if self.source == Source::Playlist(playlist_id) {
                    self.reload();
                }
                self.status = "pasta vinculada".into();
            }
            Ok(Err(player_core::playlist_folder::ForaDaBiblioteca)) => {
                self.status = "essa pasta está fora da biblioteca atual".into();
            }
            Err(err) => self.status = format!("não consegui vincular a pasta: {err}"),
        }
    }

    fn unlink_playlist_folder(&mut self, playlist_id: Uuid, root_id: i64, rel_prefix: &str) {
        let _ = player_core::playlist_folder::unlink(&self.db, playlist_id, root_id, rel_prefix);
        self.status = "pasta desvinculada".into();
    }

    /// Passa a vigiar `folder`, trocando o vigia anterior.
    fn watch(&mut self, folder: &Path) {
        if self.watcher.as_ref().is_some_and(|w| w.root() == folder) {
            return;
        }
        let ctx = self.ctx.clone();
        self.watcher = match Watcher::new(folder, move || ctx.request_repaint()) {
            Ok(watcher) => Some(watcher),
            Err(err) => {
                // Vigiar é conveniência: sem ele resta o botão "Reescanear".
                self.status = format!("não consegui vigiar a pasta: {err}");
                None
            }
        };
    }

    /// Escaneia numa thread própria, com conexão própria.
    ///
    /// O WAL permite que a conexão da UI continue lendo enquanto o scanner
    /// escreve, então a lista segue navegável durante a varredura.
    fn start_scan(&mut self, root: PathBuf) {
        let progress = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = channel();
        let db_path = self.paths.db.clone();
        let cache = self.paths.cache.clone();
        let counter = Arc::clone(&progress);

        std::thread::Builder::new()
            .name("scan".into())
            .spawn(move || {
                let outcome =
                    Db::open(&db_path)
                        .map_err(|err| err.to_string())
                        .and_then(|mut db| {
                            let art = ArtCache::new(cache);
                            scan_with_progress(&mut db, &root, &art, &counter)
                                .map_err(|err| err.to_string())
                        });
                let _ = tx.send(outcome);
            })
            .ok();

        self.status = "escaneando…".into();
        self.scan = Some(ScanJob {
            progress,
            result: rx,
            started: Instant::now(),
        });
    }

    fn poll_scan(&mut self) {
        let Some(job) = &self.scan else { return };
        let Ok(outcome) = job.result.try_recv() else {
            return;
        };

        let elapsed = job.started.elapsed();
        self.scan = None;
        // Mudou o disco no meio do scan: o resultado pode estar velho.
        if std::mem::take(&mut self.rescan_pending)
            && let Some(root) = self.root.clone()
        {
            self.start_scan(root);
        }
        match outcome {
            Ok(report) => {
                self.status = format!(
                    "{} novas · {} atualizadas · {} removidas · {} inalteradas em {:.1}s",
                    report.added,
                    report.updated,
                    report.removed,
                    report.unchanged,
                    elapsed.as_secs_f64()
                );
                // Síncrono, não numa thread própria: é só um punhado de
                // consultas contra os vínculos existentes, ao contrário do
                // nivelador (que decodifica áudio inteiro e por isso roda à
                // parte). Depois de sincronizar, `reload` de novo — se a
                // fonte atual for uma playlist vinculada, as faixas que
                // acabaram de entrar já aparecem sem precisar de outro clique.
                let _ = player_core::playlist_folder::sync_all(&mut self.db);
                self.reload();
                self.spawn_loudness_fill();

                // "Abrir com" de um ou mais arquivos: agora que o scan
                // indexou a pasta deles, resolve os caminhos pra `TrackId`
                // e começa a tocar.
                if let Some(files) = self.pending_play.take() {
                    self.play_files(&files);
                }
            }
            Err(err) => self.status = format!("falha no scan: {err}"),
        }
    }

    /// Dispara a medição do nivelador para as faixas que ainda não têm,
    /// se não houver uma rodada já em andamento.
    ///
    /// Sequencial, não em paralelo entre núcleos como o hash e o scan: essa
    /// tarefa compete por CPU com a decodificação de quem estiver tocando
    /// *agora*, e um glitch audível custa muito mais que terminar de nivelar
    /// a biblioteca alguns minutos mais cedo.
    fn spawn_loudness_fill(&self) {
        if self
            .loudness_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        let db_path = self.paths.db.clone();
        let running = Arc::clone(&self.loudness_running);
        std::thread::Builder::new()
            .name("nivelador".into())
            .spawn(move || {
                run_loudness_fill(&db_path);
                running.store(false, Ordering::Release);
            })
            .ok();
    }

    /// Reage a arquivos que apareceram, sumiram ou mudaram na pasta.
    fn poll_watcher(&mut self) {
        if !self.watcher.as_ref().is_some_and(Watcher::take_change) {
            return;
        }
        match (&self.scan, self.root.clone()) {
            // Já tem scan rodando: anota para refazer quando terminar.
            (Some(_), _) => self.rescan_pending = true,
            (None, Some(root)) => self.start_scan(root),
            (None, None) => {}
        }
    }

    // -------------------------------------------------------------------------
    // Playback
    // -------------------------------------------------------------------------

    /// Toca os arquivos vindos de "abrir com" — já indexados pelo scan que
    /// `Opened::Files` disparou. A lista visível vira exatamente esses
    /// arquivos, na ordem em que o SO os entregou: é o que a pessoa
    /// selecionou no gerenciador de arquivos, não a biblioteca inteira.
    fn play_files(&mut self, files: &[PathBuf]) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let tracks: Vec<TrackId> = files
            .iter()
            .filter_map(|file| library::find_by_absolute_path(&self.db, &root, file).ok()?)
            .collect();
        if tracks.is_empty() {
            self.status = "não consegui indexar os arquivos abertos".into();
            return;
        }

        self.source = Source::Library;
        self.view = tracks;
        self.view_positions.clear();
        self.play_at(0);
    }

    /// Toca a partir de uma linha da lista, enfileirando o que está à vista.
    ///
    /// A fila vira uma cópia da view inteira — 12 bytes por faixa, 600 KB para
    /// 50 000. Barato o suficiente para não valer a pena ser esperto.
    fn play_at(&mut self, index: usize) {
        self.queue.replace(self.view.clone(), index);
        self.start_current();
    }

    /// Toca o que o cursor da fila aponta.
    fn start_current(&mut self) {
        let Some(id) = self.queue.current() else {
            self.engine.stop();
            self.now_id = None;
            self.now = None;
            return;
        };
        let Ok(Some(info)) = library::playback_info(&self.db, id) else {
            self.status = "não encontrei o arquivo dessa faixa".into();
            return;
        };

        let gain = track_gain(&info);
        self.engine.play(info.path, gain);
        self.adopt_current(id);
    }

    /// Atualiza o que a barra do player mostra e engata a faixa seguinte.
    fn adopt_current(&mut self, id: TrackId) {
        self.now_id = Some(id);
        self.now = library::rows(&self.db, &[id])
            .ok()
            .and_then(|mut rows| rows.pop());
        self.queue_next();
    }

    /// Entrega a próxima faixa ao motor antes de a atual acabar. Sem isto não
    /// há gapless: o motor precisa abrir o próximo arquivo com antecedência.
    fn queue_next(&mut self) {
        let next = self.queue.peek_next().and_then(|id| {
            let info = library::playback_info(&self.db, id).ok().flatten()?;
            let gain = track_gain(&info);
            Some((info.path, gain))
        });
        self.engine.set_next(next);
    }

    fn next_track(&mut self) {
        if self.queue.advance().is_some() {
            self.start_current();
        }
    }

    fn prev_track(&mut self) {
        if self.queue.previous().is_some() {
            self.start_current();
        }
    }

    fn toggle_play(&mut self) {
        let state = self.engine.state();
        if state.playing {
            self.engine.pause();
        } else if self.now_id.is_some() && !self.queue.is_empty() {
            self.engine.resume();
        } else {
            self.play_at(self.selected.unwrap_or(0));
        }
    }

    /// Ajusta o volume mestre e lembra a escolha para a próxima abertura.
    fn set_volume(&mut self, volume: f32) {
        self.engine.set_volume(volume);
        let _ = self.db.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (META_VOLUME, self.engine.volume().to_string()),
        );
    }

    fn poll_audio(&mut self) {
        while let Some(event) = self.engine.poll_event() {
            match event {
                // A emenda já aconteceu no motor; aqui só acompanhamos o
                // índice e engatamos a faixa seguinte.
                Event::Advanced { .. } => {
                    // O motor já emendou; aqui a fila só acompanha.
                    self.queue.advance();
                    if let Some(id) = self.queue.current() {
                        self.adopt_current(id);
                    }
                }
                Event::Finished => {
                    self.now_id = None;
                    self.now = None;
                }
                Event::Error(err) => self.status = err,
                Event::Started { .. } => {}
            }
        }
    }

    // -------------------------------------------------------------------------
    // Desenho
    // -------------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        // `horizontal` não centraliza no eixo vertical — só aloca a altura
        // do conteúdo e larga ele no topo da faixa de 34px, colado na
        // borda. `horizontal_centered` aloca a faixa inteira e centraliza.
        let full = ui.max_rect();

        // Sem decoração nativa (`with_decorations(false)` no viewport,
        // main.rs): esta barra também É a barra de título — a barra cinza
        // clara que o xfwm4 desenhava, colada direto num conteúdo quase
        // preto sem ter nada a ver com ele, era a fonte real da "janela
        // feia" que nenhum ajuste de cor dentro do app resolvia. Área livre
        // (fora dos botões, adicionados depois — eles ganham prioridade por
        // serem registrados por último) arrasta a janela; clique duplo
        // maximiza/restaura, como qualquer barra de título de verdade.
        let drag = ui.interact(full, ui.id().with("titlebar-drag"), Sense::click_and_drag());
        if drag.double_clicked() {
            let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        ui.horizontal_centered(|ui| {
            // 4px bastava quando a decoração nativa emprestava uma borda de
            // janela de verdade antes do conteúdo começar. Sem ela, esse
            // espaço É a única coisa entre o botão e a quina da janela — 4px
            // ficava colado, esquisito. 12px é o mesmo respiro que sobra nos
            // outros cantos agora.
            ui.add_space(12.0);
            // Sem pasta ainda, "Pasta…" é a única coisa que dá pra fazer —
            // ganha o acento de ação primária. Com biblioteca carregada vira
            // reconfiguração ocasional, não pede mais destaque que isso.
            let pick = if self.root.is_none() {
                primary_button(ui, "Escolher pasta de música…")
            } else {
                ui.button("Pasta…")
            };
            if pick.clicked() {
                self.pick_folder();
            }

            if let Some(root) = &self.root {
                let label = root.to_string_lossy();
                ui.label(
                    egui::RichText::new(shorten(&label, 48))
                        .font(theme::small())
                        .color(theme::DIM),
                );
                if self.scan.is_none() && ui.button("Reescanear").clicked() {
                    self.start_scan(root.clone());
                }
            }

            ui.add_space(8.0);
            // Largura responsiva, não fixa: numa janela estreita, uma caixa
            // de busca de 220px sobra do espaço disponível e invade o texto
            // da direita — as duas são desenhadas sem uma saber da outra,
            // então o resultado é sobreposição, não quebra de linha. Reserva
            // uma folga pro texto da direita antes de decidir a largura —
            // agora incluindo os três controles de janela do lado direito.
            let search_width = (ui.available_width() - 340.0).clamp(60.0, 220.0);
            // A busca só filtra a biblioteca — dentro de uma playlist ela
            // ficaria filtrando contra o índice errado. Desabilitada, não
            // escondida: o texto continua ali para quando o usuário voltar.
            let in_library = self.source == Source::Library;
            ui.add_enabled_ui(in_library, |ui| {
                search_icon(ui);
                ui.add_space(-6.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .desired_width(search_width)
                        .hint_text("buscar"),
                );
                if std::mem::take(&mut self.focus_search) && in_library {
                    search.request_focus();
                }
                if search.changed() {
                    self.reload();
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Mesmo respiro do canto esquerdo, agora do lado direito —
                // sem ele o botão de fechar ficava colado na quina.
                ui.add_space(12.0);
                // Controles de janela — os que o xfwm4 desenhava sozinho
                // antes de `with_decorations(false)`. Primeiro a entrar no
                // layout right-to-left é o que fica mais à direita.
                if window_button(ui, WindowGlyph::Close).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if window_button(ui, WindowGlyph::Maximize).clicked() {
                    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                if window_button(ui, WindowGlyph::Minimize).clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                ui.add_space(10.0);
                let texto = match (&self.scan, self.source) {
                    (Some(job), _) => {
                        format!("escaneando… {}", job.progress.load(Ordering::Relaxed))
                    }
                    (None, Source::Playlist(id)) => self
                        .playlists
                        .iter()
                        .find(|p| p.id == id)
                        .map_or_else(String::new, |p| {
                            format!("{}  ·  {} faixas", p.name, p.items)
                        }),
                    (None, Source::Library) => format!(
                        "{} faixas · {} álbuns · {} artistas",
                        self.stats.tracks, self.stats.albums, self.stats.artists
                    ),
                };
                ui.label(
                    egui::RichText::new(texto)
                        .font(theme::small())
                        .color(theme::DIM),
                );
            });
        });
        // Fio de 1px separando o topo do conteúdo — o Apple Music usa a
        // mesma régua fina entre as regiões da janela; sem ela, o topo e a
        // lista eram a mesma cor sólida sem nenhuma articulação entre as
        // duas.
        ui.painter().line_segment(
            [
                pos2(full.left(), full.bottom() - 0.5),
                pos2(full.right(), full.bottom() - 0.5),
            ],
            Stroke::new(1.0, theme::RULE),
        );
    }

    /// Barra lateral: biblioteca + playlists. Larga o bastante para nomes
    /// razoáveis, estreita o bastante para não roubar espaço da lista.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(6.0);

        if sidebar_row(ui, "BIBLIOTECA", self.source == Source::Library).clicked() {
            self.set_source(Source::Library);
        }

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("PLAYLISTS")
                    .font(theme::small())
                    .color(theme::FAINT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if add_playlist_button(ui)
                    .on_hover_text("Nova playlist")
                    .clicked()
                {
                    self.create_playlist(None);
                }
            });
        });
        ui.add_space(4.0);

        let playlists = self.playlists.clone();
        let mut pending_delete = None;

        for pl in &playlists {
            let is_active = self.source == Source::Playlist(pl.id);
            let is_renaming = matches!(&self.renaming, Some((id, _, _)) if *id == pl.id);

            if is_renaming {
                let response = ui
                    .horizontal(|ui| {
                        ui.add_space(9.0);
                        let (_, buf, _) = self
                            .renaming
                            .as_mut()
                            .expect("checado por is_renaming acima");
                        ui.add(
                            egui::TextEdit::singleline(buf)
                                .desired_width(ui.available_width() - 12.0)
                                .font(theme::body()),
                        )
                    })
                    .inner;

                if let Some((_, _, focus)) = &mut self.renaming
                    && std::mem::take(focus)
                {
                    response.request_focus();
                }
                if response.lost_focus() {
                    self.commit_rename();
                }
                continue;
            }

            let label = format!("{}  ({})", pl.name, pl.items);
            let row = sidebar_row(ui, &label, is_active);
            if row.clicked() {
                self.set_source(Source::Playlist(pl.id));
            }
            row.context_menu(|ui| {
                if ui.button("Renomear").clicked() {
                    self.renaming = Some((pl.id, pl.name.clone(), true));
                    ui.close();
                }
                if ui.button("Apagar").clicked() {
                    pending_delete = Some(pl.id);
                    ui.close();
                }
                ui.separator();
                if ui
                    .button("Vincular pasta…")
                    .on_hover_text(
                        "Toda faixa dessa pasta entra sozinha na playlist, \
                         sem precisar arrastar uma por uma",
                    )
                    .clicked()
                {
                    self.link_playlist_folder(pl.id);
                    ui.close();
                }
                for link in
                    player_core::playlist_folder::links_for(&self.db, pl.id).unwrap_or_default()
                {
                    let label = if link.rel_prefix.is_empty() {
                        "Desvincular pasta inteira".to_owned()
                    } else {
                        format!("Desvincular \"{}\"", link.rel_prefix)
                    };
                    if ui.button(label).clicked() {
                        self.unlink_playlist_folder(pl.id, link.root_id, &link.rel_prefix);
                        ui.close();
                    }
                }
            });
        }

        if let Some(id) = pending_delete {
            self.delete_playlist(id);
        }

        // Mesmo fio de 1px do topo, na borda direita — separa a sidebar do
        // conteúdo em vez de deixar a diferença de tom (`PANEL` contra `BG`)
        // como única articulação entre as duas.
        ui.painter().line_segment(
            [
                pos2(full.right() - 0.5, full.top()),
                pos2(full.right() - 0.5, full.bottom()),
            ],
            Stroke::new(1.0, theme::RULE),
        );
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        if self.view.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(64.0);
                // A marca só aparece na primeira tela — bem-vindo, não em
                // toda lista vazia. Uma playlist sem faixas ou uma busca sem
                // resultado é estado normal de uso, não pede identidade.
                if self.root.is_none() {
                    let (rect, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
                    glow(ui.painter(), rect, theme::ACCENT);
                    ui.painter().image(
                        self.mark.id(),
                        rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    ui.add_space(16.0);
                }
                let texto = match self.source {
                    _ if self.root.is_none() => "Escolha uma pasta de música para começar.",
                    Source::Playlist(_) => {
                        "Esta playlist está vazia. Botão direito numa faixa da biblioteca para adicionar."
                    }
                    Source::Library if self.query.is_empty() => "Nenhuma faixa indexada nessa pasta.",
                    Source::Library => "Nada encontrado.",
                };
                ui.label(egui::RichText::new(texto).color(theme::DIM));
                // A tela de boas-vindas ganha um botão de verdade, não só a
                // dica de texto — "Pasta…" no topo já faz isso, mas escondido
                // num canto pequeno na primeira execução (tela em branco,
                // nada pra olhar) é fácil de não notar.
                if self.root.is_none() {
                    ui.add_space(16.0);
                    if primary_button(ui, "Escolher pasta de música…").clicked() {
                        self.pick_folder();
                    }
                }
            });
            return;
        }

        // O egui insere espaçamento vertical entre widgets; numa lista de
        // faixas isso abre uma fresta entre as linhas, quebra a zebra e come
        // densidade. Aqui as linhas se encostam.
        ui.spacing_mut().item_spacing.y = 0.0;

        let width = ui.available_width();
        let cols = Columns::new(width);

        // Cabeçalho: rótulos apagados e uma régua de 1px. Sem fundo, sem caixa.
        let (header, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
        let painter = ui.painter();
        for (label, rect) in [
            ("#", cols.num(header)),
            ("TÍTULO", cols.title(header)),
            ("ARTISTA", cols.artist(header)),
            ("ÁLBUM", cols.album(header)),
            ("DUR", cols.duration(header)),
        ] {
            // A coluna de capa não tem cabeçalho — imagem não precisa de
            // rótulo, e "CAPA" ocuparia espaço sem informar nada.
            painter.text(
                pos2(rect.left(), header.center().y),
                Align2::LEFT_CENTER,
                label,
                theme::small(),
                theme::FAINT,
            );
        }
        painter.line_segment(
            [
                pos2(header.left(), header.bottom() - 0.5),
                pos2(header.right(), header.bottom() - 0.5),
            ],
            Stroke::new(1.0, theme::RULE),
        );

        let mut clicked: Option<(usize, bool)> = None;
        // Capturados antes do closure para não precisar reler `self` dentro
        // dele: `playlists` é lido no submenu, `source` e `total` decidem se
        // "mover"/"remover" fazem sentido para a linha.
        let playlists = self.playlists.clone();
        let source = self.source;
        let total = self.view.len();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, theme::ROW_HEIGHT, self.view.len(), |ui, range| {
                // Só a janela visível vai ao banco — ~40 linhas, 0,1 ms.
                let ids = &self.view[range.clone()];
                let rows = library::rows(&self.db, ids).unwrap_or_default();

                for (offset, row) in rows.iter().enumerate() {
                    let index = range.start + offset;
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(width, theme::ROW_HEIGHT), Sense::click());

                    let is_playing = self.now_id == Some(row.id);
                    let is_selected = self.selected == Some(index);
                    let painter = ui.painter();

                    // Destaque recuado e arredondado — não mais um retângulo
                    // esticado de ponta a ponta. Sem zebra: capa + espaço já
                    // dão o suficiente pra olho seguir a linha numa lista
                    // grande, e listras junto com capa ficaria carregado.
                    if is_selected || (response.hovered() && !is_selected) {
                        let highlight = rect.shrink2(vec2(4.0, 2.0));
                        painter.rect_filled(
                            highlight,
                            CornerRadius::same(theme::RADIUS),
                            theme::HOVER,
                        );
                    }
                    if is_playing {
                        // Marca de 2px na canaleta esquerda: o acento aparece
                        // aqui e na barra de progresso, em mais nenhum lugar.
                        // Fica rente à borda, fora do destaque recuado.
                        painter.rect_filled(
                            Rect::from_min_size(rect.left_top(), vec2(2.0, rect.height())),
                            CornerRadius::ZERO,
                            theme::ACCENT,
                        );
                    }

                    let art_rect = cols.art(rect);
                    if let Some(texture) = row.art_hash.and_then(|hash| self.art.texture(&hash)) {
                        rounded_image(painter, art_rect, texture, theme::RADIUS_SM);
                    } else {
                        painter.rect_filled(
                            art_rect,
                            CornerRadius::same(theme::RADIUS_SM),
                            theme::PANEL,
                        );
                    }

                    let title_color = if is_playing {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    };
                    cell(
                        painter,
                        cols.num(rect),
                        &row.track_no.map_or_else(String::new, |n| n.to_string()),
                        theme::mono(),
                        theme::FAINT,
                    );
                    cell_strong(painter, cols.title(rect), &row.title, 13.0, title_color);
                    cell(
                        painter,
                        cols.artist(rect),
                        row.artist.as_deref().unwrap_or("—"),
                        theme::body(),
                        theme::DIM,
                    );
                    cell(
                        painter,
                        cols.album(rect),
                        row.album.as_deref().unwrap_or("—"),
                        theme::body(),
                        theme::DIM,
                    );
                    cell(
                        painter,
                        cols.duration(rect),
                        &format_ms(row.duration_ms),
                        theme::mono(),
                        theme::FAINT,
                    );

                    if response.clicked() {
                        clicked = Some((index, false));
                    }
                    if response.double_clicked() {
                        clicked = Some((index, true));
                    }

                    let track = row.id;
                    response.context_menu(|ui| {
                        ui.menu_button("Adicionar à playlist", |ui| {
                            for pl in &playlists {
                                if ui.button(&pl.name).clicked() {
                                    self.add_to_playlist(pl.id, track);
                                    ui.close();
                                }
                            }
                            if !playlists.is_empty() {
                                ui.separator();
                            }
                            if ui.button("Nova playlist…").clicked() {
                                self.create_playlist(Some(track));
                                ui.close();
                            }
                        });

                        if matches!(source, Source::Playlist(_)) {
                            ui.separator();
                            let up =
                                ui.add_enabled(index > 0, egui::Button::new("Mover para cima"));
                            if up.clicked() {
                                self.nudge_in_playlist(index, -1);
                                ui.close();
                            }
                            let down = ui.add_enabled(
                                index + 1 < total,
                                egui::Button::new("Mover para baixo"),
                            );
                            if down.clicked() {
                                self.nudge_in_playlist(index, 1);
                                ui.close();
                            }
                            if ui.button("Remover da playlist").clicked() {
                                self.remove_from_playlist(index);
                                ui.close();
                            }
                        }

                        ui.separator();
                        if ui.button("Escolher capa do álbum…").clicked() {
                            self.pick_album_art(track);
                            ui.close();
                        }
                    });
                }
            });

        if let Some((index, play)) = clicked {
            self.selected = Some(index);
            if play {
                self.play_at(index);
            }
        }
    }

    fn player_bar(&mut self, ui: &mut egui::Ui) {
        let state = self.engine.state();
        let full = ui.max_rect();
        // Fio bem claro (branco quase transparente, não a régua escura de
        // sempre) na borda de cima — separa o player do conteúdo por trás
        // como se ele flutuasse um pouco à frente, não só a mudança de tom.
        ui.painter().line_segment(
            [full.left_top(), full.right_top()],
            Stroke::new(1.0, Color32::from_white_alpha(14)),
        );

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);

            // Capa. Só aparece aqui e no modo compacto.
            self.draw_cover(ui, 56.0);

            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.add_space(6.0);
                match &self.now {
                    Some(row) => {
                        ui.label(egui::RichText::new(&row.title).heading().color(theme::TEXT));
                        ui.label(
                            egui::RichText::new(format!(
                                "{}  ·  {}",
                                row.artist.as_deref().unwrap_or("—"),
                                row.album.as_deref().unwrap_or("—")
                            ))
                            .font(theme::small())
                            .color(theme::DIM),
                        );
                    }
                    None => {
                        ui.label(
                            egui::RichText::new("nada tocando")
                                .font(theme::small())
                                .color(theme::FAINT),
                        );
                    }
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                if let Some(position) = self.queue.position() {
                    ui.label(
                        egui::RichText::new(format!("{position} / {}", self.queue.len()))
                            .font(theme::mono())
                            .color(theme::FAINT),
                    );
                    ui.add_space(10.0);
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{} / {}",
                        format_duration(state.position),
                        state
                            .duration
                            .map_or_else(|| "--:--".into(), format_duration)
                    ))
                    .font(theme::mono())
                    .color(theme::DIM),
                );

                ui.add_space(10.0);
                if let Some(volume) = volume_slider(ui, self.engine.volume()) {
                    self.set_volume(volume);
                }

                ui.add_space(10.0);
                // Ícones, não rótulo: aceso/apagado continua sendo a cor, o
                // acento continua reservado pra "isto está tocando".
                if icon_toggle(ui, Icon::Mini, self.mini)
                    .on_hover_text("Modo compacto (Ctrl+M)")
                    .clicked()
                {
                    self.toggle_mini(ui.ctx());
                }
                if icon_toggle(ui, Icon::Shuffle, self.queue.shuffle())
                    .on_hover_text("Shuffle (S)")
                    .clicked()
                {
                    let on = !self.queue.shuffle();
                    self.queue.set_shuffle(on);
                    self.queue_next();
                }
                let repeat_hint = match self.queue.repeat() {
                    Repeat::Off => "Repetir: desligado (R)",
                    Repeat::All => "Repetir: tudo (R)",
                    Repeat::One => "Repetir: uma faixa (R)",
                };
                if icon_toggle(
                    ui,
                    Icon::Repeat(self.queue.repeat() == Repeat::One),
                    self.queue.repeat() != Repeat::Off,
                )
                .on_hover_text(repeat_hint)
                .clicked()
                {
                    let mode = self.queue.repeat().next();
                    self.queue.set_repeat(mode);
                    self.queue_next();
                }

                ui.add_space(10.0);
                if transport(ui, Glyph::Next).clicked() {
                    self.next_track();
                }
                if transport(
                    ui,
                    if state.playing {
                        Glyph::Pause
                    } else {
                        Glyph::Play
                    },
                )
                .clicked()
                {
                    self.toggle_play();
                }
                if transport(ui, Glyph::Prev).clicked() {
                    self.prev_track();
                }
            });
        });

        ui.add_space(6.0);
        self.progress_bar(ui, state);
    }

    /// Barra de progresso: trilho em pílula, igual ao slider de volume — o
    /// segundo e último uso do acento além da marca da faixa tocando. A
    /// bolinha só aparece em hover/arraste, pra não pesar visualmente numa
    /// barra que fica sempre visível durante o playback inteiro.
    ///
    /// A área clicável (`HIT_HEIGHT`) é bem maior que o traço visual de 4px:
    /// clicar/arrastar em cima ou embaixo da linha, não só exatamente nela,
    /// continua funcionando — sem isso o alvo de clique real era de uns 12px,
    /// difícil de acertar de primeira.
    fn progress_bar(&mut self, ui: &mut egui::Ui, state: player_audio::PlaybackState) {
        const HIT_HEIGHT: f32 = 20.0;
        let width = ui.available_width();
        let knob_radius = 5.0;
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, HIT_HEIGHT), Sense::click_and_drag());
        let painter = ui.painter();

        let track_height = 4.0;
        let track = Rect::from_center_size(rect.center(), vec2(rect.width(), track_height));
        let radius = track_height / 2.0;
        painter.rect_filled(track, radius, theme::RULE);

        let fraction = state
            .duration
            .filter(|d| d.as_secs_f32() > 0.0)
            .map_or(0.0, |d| {
                (state.position.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0)
            });
        let knob_x = track.left() + track.width() * fraction;
        if fraction > 0.0 {
            let filled = Rect::from_min_size(
                track.left_top(),
                vec2((knob_x - track.left()).max(track_height), track.height()),
            );
            // Gradiente, não cor chapada — mesmo matiz nas duas pontas, só
            // luminosidade diferente, o único lugar da interface onde o
            // acento ganha profundidade.
            gradient_fill(
                painter,
                filled,
                radius,
                theme::ACCENT_DIM,
                theme::ACCENT_BRIGHT,
            );
        }
        if response.hovered() || response.dragged() {
            painter.circle_filled(
                pos2(knob_x, track.center().y),
                knob_radius,
                theme::ACCENT_BRIGHT,
            );
        }

        if let (true, Some(total), Some(pointer)) = (
            response.clicked() || response.dragged(),
            state.duration,
            response.interact_pointer_pos(),
        ) {
            let target = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
            self.engine
                .seek(Duration::from_secs_f32(total.as_secs_f32() * target));
        }
    }

    /// Desenha a capa da faixa atual num quadrado `size`x`size`. Único
    /// widget repetido entre o player normal e o modo compacto.
    fn draw_cover(&mut self, ui: &mut egui::Ui, size: f32) {
        let (cover, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        let painter = ui.painter();
        let radius = theme::RADIUS;
        // Só brilha tocando algo — capa vazia não precisa de halo em volta
        // de um retângulo em branco.
        if self.now.is_some() {
            glow(painter, cover, theme::ACCENT);
        }
        painter.rect_filled(cover, CornerRadius::same(radius), theme::PANEL);
        let texture = self
            .now
            .as_ref()
            .and_then(|row| row.art_hash)
            .and_then(|hash| self.art.texture(&hash));
        if let Some(texture) = texture {
            rounded_image(painter, cover, texture, radius);
        } else {
            painter.rect_stroke(
                cover,
                CornerRadius::same(radius),
                Stroke::new(1.0, theme::RULE),
                egui::StrokeKind::Inside,
            );
        }
    }

    /// Janela compacta: capa, título/artista, transporte e progresso. Cabe
    /// num canto da tela sem competir por espaço com outras janelas — o
    /// "sempre visível, sempre pequeno" que falta em muito player.
    fn mini_bar(&mut self, ui: &mut egui::Ui) {
        let state = self.engine.state();

        // Sem decoração nativa, o modo compacto também perdeu a barra de
        // título que dava pra arrastar — e arrastar é o ponto inteiro dele
        // ("fica num canto da tela"). Recupera isso à mão: área livre (fora
        // dos botões) move a janela.
        let full = ui.max_rect();
        if ui
            .interact(full, ui.id().with("mini-drag"), Sense::click_and_drag())
            .drag_started()
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            self.draw_cover(ui, 44.0);

            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_max_width(96.0);
                ui.add_space(3.0);
                match &self.now {
                    Some(row) => {
                        ui.label(egui::RichText::new(shorten(&row.title, 14)).color(theme::TEXT));
                        ui.label(
                            egui::RichText::new(shorten(row.artist.as_deref().unwrap_or("—"), 14))
                                .font(theme::small())
                                .color(theme::DIM),
                        );
                    }
                    None => {
                        ui.label(
                            egui::RichText::new("nada tocando")
                                .font(theme::small())
                                .color(theme::FAINT),
                        );
                    }
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                if transport(ui, Glyph::Next).clicked() {
                    self.next_track();
                }
                if transport(
                    ui,
                    if state.playing {
                        Glyph::Pause
                    } else {
                        Glyph::Play
                    },
                )
                .clicked()
                {
                    self.toggle_play();
                }
                if transport(ui, Glyph::Prev).clicked() {
                    self.prev_track();
                }
                // Volta pra janela normal. Precisa estar sempre visível: é o
                // único jeito de sair sem saber do atalho de cor.
                if icon_toggle(ui, Icon::Mini, true)
                    .on_hover_text("Sair do modo compacto (Ctrl+M)")
                    .clicked()
                {
                    self.toggle_mini(ui.ctx());
                }
                ui.add_space(6.0);
            });
        });

        ui.add_space(6.0);
        self.progress_bar(ui, state);
    }

    /// Alterna entre a janela normal e o modo compacto, redimensionando a
    /// janela de verdade — não é só uma troca de layout.
    ///
    /// O tamanho anterior é lido do próprio `InputState` na hora de entrar no
    /// modo compacto, não guardado a cada frame: assim, se o usuário
    /// redimensionar a janela normal antes de encolher, é esse tamanho (e
    /// não um valor desatualizado) que volta ao sair do modo compacto.
    fn toggle_mini(&mut self, ctx: &egui::Context) {
        self.mini = !self.mini;
        let target = if self.mini {
            if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                self.normal_size = rect.size();
            }
            MINI_SIZE
        } else {
            self.normal_size
        };

        // O tamanho mínimo configurado na abertura (620x380, para a lista
        // de faixas não virar sopa de letrinhas) impediria a janela de
        // encolher até o tamanho compacto. Relaxa antes de pedir o novo
        // tamanho, e trava nele — a janela compacta não é para ser
        // redimensionada à mão, ela já nasce do jeito que precisa ser.
        let min = if self.mini {
            MINI_SIZE
        } else {
            NORMAL_MIN_SIZE
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Atalho não pode roubar tecla de quem está digitando na busca.
        if ctx.egui_wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::Escape) {
                    self.focus_search = false;
                }
            });
            return;
        }

        let (space, enter, down, up, find, shuffle, repeat, next, prev, mini) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
                i.modifiers.command && i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::S),
                i.key_pressed(egui::Key::R),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowLeft),
                i.modifiers.command && i.key_pressed(egui::Key::M),
            )
        });

        if space {
            self.toggle_play();
        }
        if shuffle {
            let on = !self.queue.shuffle();
            self.queue.set_shuffle(on);
            self.queue_next();
        }
        if repeat {
            let mode = self.queue.repeat().next();
            self.queue.set_repeat(mode);
            self.queue_next();
        }
        if next {
            self.next_track();
        }
        if prev {
            self.prev_track();
        }
        if find {
            self.focus_search = true;
        }
        if mini {
            self.toggle_mini(ctx);
        }
        if down || up {
            let last = self.view.len().saturating_sub(1);
            self.selected = Some(match self.selected {
                Some(index) if down => (index + 1).min(last),
                Some(index) => index.saturating_sub(1),
                None => 0,
            });
        }
        if enter && let Some(index) = self.selected {
            self.play_at(index);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.art.begin_frame(ctx);
        self.poll_audio();
        self.poll_scan();
        self.poll_watcher();
        self.shortcuts(ctx);

        if self.mini {
            // Modo compacto: a janela inteira é a barra do player, sem
            // topo, sidebar ou lista — é para isso que ela existe.
            egui::CentralPanel::no_frame()
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ui, |ui| self.mini_bar(ui));
        } else {
            egui::Panel::top("comando")
                .exact_size(34.0)
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ui, |ui| self.top_bar(ui));

            egui::Panel::bottom("player")
                .exact_size(92.0)
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ui, |ui| self.player_bar(ui));

            if !self.status.is_empty() {
                egui::Panel::bottom("status")
                    .exact_size(20.0)
                    .frame(egui::Frame::new().fill(theme::BG))
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(&self.status)
                                    .font(theme::small())
                                    .color(theme::FAINT),
                            );
                        });
                    });
            }

            egui::Panel::left("sidebar")
                .exact_size(170.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ui, |ui| self.sidebar(ui));

            egui::CentralPanel::no_frame()
                .frame(egui::Frame::new().fill(theme::BG))
                .show(ui, |ui| self.list(ui));
        }

        // Sem decoração nativa, a janela também perdeu o traço de 1px que o
        // gerenciador de janelas desenhava em volta dela — sem ele, o
        // retângulo se perde contra o fundo da área de trabalho por trás.
        // Numa camada de primeiro plano, por cima de tudo, porque não faz
        // parte de painel nenhum.
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("window-border"),
        ))
        .rect_stroke(
            ctx.content_rect().shrink(0.5),
            CornerRadius::ZERO,
            Stroke::new(1.0, theme::RULE),
            egui::StrokeKind::Inside,
        );

        // A política de repaint. Nada de 60 fps.
        if self.scan.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.engine.state().playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

// -----------------------------------------------------------------------------
// Peças de desenho
// -----------------------------------------------------------------------------

/// Larguras das colunas. Fixas onde o conteúdo é previsível (número, duração),
/// proporcionais onde não é.
struct Columns {
    num: f32,
    art: f32,
    title: f32,
    artist: f32,
    album: f32,
    duration: f32,
}

impl Columns {
    const PAD: f32 = 10.0;
    /// Lado da miniatura — cabe com folga dentro de `theme::ROW_HEIGHT`.
    const ART_SIZE: f32 = 32.0;

    fn new(width: f32) -> Self {
        let num = 28.0;
        let art = Self::ART_SIZE + Self::PAD;
        let duration = 52.0;
        let rest = (width - num - art - duration - Self::PAD * 2.0).max(120.0);
        Self {
            num,
            art,
            title: rest * 0.42,
            artist: rest * 0.30,
            album: rest * 0.28,
            duration,
        }
    }

    fn slice(rect: Rect, from: f32, width: f32) -> Rect {
        Rect::from_min_size(
            pos2(rect.left() + from + Columns::PAD, rect.top()),
            vec2(width - 8.0, rect.height()),
        )
    }

    fn num(&self, rect: Rect) -> Rect {
        Self::slice(rect, 0.0, self.num)
    }
    /// Quadrado da miniatura, centrado verticalmente na linha — não usa
    /// `slice` porque a arte é quadrada, não uma faixa de texto.
    fn art(&self, rect: Rect) -> Rect {
        let x = rect.left() + self.num + Columns::PAD;
        Rect::from_center_size(
            pos2(x + Self::ART_SIZE / 2.0, rect.center().y),
            vec2(Self::ART_SIZE, Self::ART_SIZE),
        )
    }
    fn title(&self, rect: Rect) -> Rect {
        Self::slice(rect, self.num + self.art, self.title)
    }
    fn artist(&self, rect: Rect) -> Rect {
        Self::slice(rect, self.num + self.art + self.title, self.artist)
    }
    fn album(&self, rect: Rect) -> Rect {
        Self::slice(
            rect,
            self.num + self.art + self.title + self.artist,
            self.album,
        )
    }
    fn duration(&self, rect: Rect) -> Rect {
        Self::slice(
            rect,
            self.num + self.art + self.title + self.artist + self.album,
            self.duration,
        )
    }
}

/// Uma linha da sidebar: rótulo alinhado à esquerda, marca de acento quando
/// ativa. Mesma linguagem visual da lista de faixas — a régua de 2px que
/// destaca "o que está tocando" aqui destaca "o que está selecionado".
fn sidebar_row(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, theme::ROW_HEIGHT), Sense::click());
    let painter = ui.painter();

    if active || response.hovered() {
        let highlight = rect.shrink2(vec2(4.0, 2.0));
        painter.rect_filled(highlight, CornerRadius::same(theme::RADIUS), theme::HOVER);
    }
    if active {
        painter.rect_filled(
            Rect::from_min_size(rect.left_top(), vec2(2.0, rect.height())),
            CornerRadius::ZERO,
            theme::ACCENT,
        );
    }

    let color = if active { theme::ACCENT } else { theme::TEXT };
    let text_rect = Rect::from_min_size(
        pos2(rect.left() + 10.0, rect.top()),
        vec2((rect.width() - 16.0).max(0.0), rect.height()),
    );
    cell(painter, text_rect, label, theme::body(), color);

    response
}

/// Desenha `texture` dentro de `rect` com cantos arredondados — imagem com
/// máscara de raio, não o retângulo reto que `Painter::image` desenha.
/// `RectShape` do epaint aceita textura (`brush`) e `corner_radius` juntos;
/// é essa combinação que faz a capa ficar arredondada sem cortar a imagem
/// à mão.
fn rounded_image(painter: &egui::Painter, rect: Rect, texture: egui::TextureId, radius: u8) {
    let mut shape =
        egui::epaint::RectShape::filled(rect, CornerRadius::same(radius), Color32::WHITE);
    shape.brush = Some(std::sync::Arc::new(egui::epaint::Brush {
        fill_texture_id: texture,
        uv: Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
    }));
    painter.add(shape);
}

/// Brilho suave atrás de um retângulo (capa, marca) — anéis concêntricos com
/// alfa decrescente. `egui::Painter` não tem desfoque de verdade; isso é
/// vetor puro (uns círculos a mais por frame) chegando perto do efeito sem
/// precisar de shader nem textura pré-borrada.
fn glow(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let base = rect.width().max(rect.height()) / 2.0;
    for (i, alpha) in [14u8, 9, 5].into_iter().enumerate() {
        let radius = base + 3.0 + i as f32 * 4.0;
        let tint = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        painter.circle_filled(center, radius, tint);
    }
}

fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
    Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

/// Preenchimento em gradiente horizontal dentro de uma pílula. `Painter` não
/// tem gradiente nativo: a base sólida em `to` cobre as pontas arredondadas
/// (raio pequeno, ninguém nota a costura), e por cima faixas verticais finas
/// com cor interpolada cobrem só o miolo reto — sem precisar recortar pra
/// forma arredondada.
fn gradient_fill(painter: &egui::Painter, rect: Rect, radius: f32, from: Color32, to: Color32) {
    painter.rect_filled(rect, radius, to);
    let inner = rect.shrink2(vec2(radius, 0.0));
    if inner.width() <= 0.0 {
        return;
    }
    const STEPS: usize = 16;
    let step_w = inner.width() / STEPS as f32;
    for i in 0..STEPS {
        let t = i as f32 / (STEPS - 1) as f32;
        let x0 = inner.left() + step_w * i as f32;
        let strip = Rect::from_min_max(
            pos2(x0, inner.top()),
            pos2(x0 + step_w + 0.5, inner.bottom()),
        );
        painter.rect_filled(strip, 0.0, lerp_color(from, to, t));
    }
}

/// Texto de uma célula, truncado na largura da coluna.
fn cell(painter: &egui::Painter, rect: Rect, text: &str, font: egui::FontId, color: Color32) {
    if text.is_empty() {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width());
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

/// Igual a [`cell`], mas com peso de verdade — fonte SemiBold embutida
/// (`theme::strong`), não mais o texto desenhado duas vezes com deslocamento
/// que fingia negrito antes de ter uma família de peso variável no binário.
/// Reservado pro título — usar em toda célula apagaria a hierarquia que ele
/// existe pra criar.
fn cell_strong(painter: &egui::Painter, rect: Rect, text: &str, size: f32, color: Color32) {
    cell(painter, rect, text, theme::strong(size), color);
}

/// Controle de volume mestre: trilho em pílula com uma bolinha arrastável,
/// no espírito do slider de volume do Apple Music.
///
/// Única exceção deliberada aos cantos retos do resto da interface: um
/// controle contínuo arrastável se lê melhor como objeto físico (um
/// dial) do que como dado tabular, e é isso que o resto da UI é — linhas,
/// réguas, caixas. Fica contida a este widget só; nada mais na interface
/// ganha curva por causa dele.
///
/// Neutro, não no acento: o acento aqui significa uma coisa só (o que está
/// tocando), e volume não é isso.
///
/// Devolve o novo valor quando o usuário mexe; `None` quando só está sendo
/// desenhado sem interação nesta chamada.
///
/// Mesmo alvo de clique generoso do `progress_bar`: `HIT_HEIGHT` bem maior
/// que o traço de 4px, pra não exigir mira milimétrica numa bolinha pequena.
fn volume_slider(ui: &mut egui::Ui, value: f32) -> Option<f32> {
    const HIT_HEIGHT: f32 = 20.0;
    let value = value.clamp(0.0, 1.0);
    let knob_radius = 5.0;
    let (rect, response) = ui.allocate_exact_size(vec2(56.0, HIT_HEIGHT), Sense::click_and_drag());
    let painter = ui.painter();

    // Trilho: pílula (raio = metade da altura), não retângulo — é a curva
    // que só existe aqui.
    let track_height = 4.0;
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), track_height));
    let radius = track_height / 2.0;
    painter.rect_filled(track, radius, theme::RULE);

    let knob_x = track.left() + track.width() * value;
    if value > 0.0 {
        let filled = Rect::from_min_size(
            track.left_top(),
            vec2((knob_x - track.left()).max(track_height), track.height()),
        );
        // Gradiente neutro (cinza pra cinza, nunca o acento — volume não é
        // "o que está tocando"): mesmo toque de profundidade da barra de
        // progresso, sem tomar emprestada a cor que devia significar outra
        // coisa.
        gradient_fill(painter, filled, radius, theme::FAINT, theme::DIM);
    }

    // A bolinha: mais clara em hover/arraste, para confirmar que pegou o
    // controle certo antes de soltar.
    let knob_color = if response.hovered() || response.dragged() {
        theme::TEXT
    } else {
        theme::DIM
    };
    painter.circle_filled(pos2(knob_x, track.center().y), knob_radius, knob_color);

    if response.clicked() || response.dragged() {
        let pointer = response.interact_pointer_pos()?;
        return Some(((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
    }
    None
}

/// Ícones dos alternadores da barra do player.
///
/// `Repeat(bool)` carrega se é "repetir uma" — desenha o mesmo laço com um
/// "1" pequeno dentro, em vez de trocar de ícone inteiro (o formato do laço
/// não muda, só o que está dentro dele).
#[derive(Clone, Copy)]
enum Icon {
    Shuffle,
    Repeat(bool),
    Mini,
}

/// Alternador com ícone da fonte Lucide. O fundo de hover arredonda como o
/// resto da interface; `Repeat(true)` sobrepõe um "1" pequeno no canto —
/// o Lucide já tem um glifo dedicado pra "repetir uma" (`repeat-1`), então
/// não precisa desenhar o dígito à mão como antes.
fn icon_toggle(ui: &mut egui::Ui, icon: Icon, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::click());
    let painter = ui.painter();

    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), theme::HOVER);
    }
    let color = if active {
        theme::TEXT
    } else if response.hovered() {
        theme::DIM
    } else {
        theme::FAINT
    };

    let glyph = match icon {
        Icon::Shuffle => theme::icon_glyph::SHUFFLE,
        Icon::Repeat(true) => theme::icon_glyph::REPEAT_ONE,
        Icon::Repeat(false) => theme::icon_glyph::REPEAT,
        Icon::Mini => theme::icon_glyph::PICTURE_IN_PICTURE,
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        theme::icon(15.0),
        color,
    );

    // Sublinhado de 1px marca ligado — mesma linguagem do toggle em texto.
    if active {
        painter.line_segment(
            [
                pos2(rect.left() + 5.0, rect.bottom() - 3.0),
                pos2(rect.right() - 5.0, rect.bottom() - 3.0),
            ],
            Stroke::new(1.0, theme::TEXT),
        );
    }

    response
}

/// Botão "+" de nova playlist, no cabeçalho da sidebar.
///
/// Era um `egui::Button` de texto (`"+".small()`) — uma caixa retangular
/// apertada em volta de um glifo de fonte, destoando do resto da interface,
/// que não usa texto pra ação nenhuma no player.
fn add_playlist_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let painter = ui.painter();

    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), theme::HOVER);
    }
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::DIM
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        theme::icon_glyph::PLUS,
        theme::icon(14.0),
        color,
    );

    response
}

/// Lupa à esquerda do campo de busca. Puramente decorativo
/// (`Sense::hover`), não captura clique — o campo de texto continua sendo
/// o alvo.
fn search_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::CENTER_CENTER,
        theme::icon_glyph::SEARCH,
        theme::icon(13.0),
        theme::FAINT,
    );
}

#[derive(Clone, Copy)]
enum WindowGlyph {
    Minimize,
    Maximize,
    Close,
}

/// Botão de controle de janela (minimizar/maximizar/fechar), no lugar do
/// que o gerenciador de janelas desenhava sozinho antes de
/// `with_decorations(false)`. Traço vetorial, mesma linguagem do resto —
/// fechar ganha o único vermelho da interface inteira, convenção forte
/// demais pra abrir mão dela só por causa da paleta de acento único.
fn window_button(ui: &mut egui::Ui, glyph: WindowGlyph) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 26.0), Sense::click());
    let painter = ui.painter();

    // Fundo circular, não o mesmo cantos-arredondados do resto dos botões —
    // um "botão de controle" lê melhor como forma fechada em si (o mesmo
    // motivo do macOS pros seus três pontinhos) do que como mais um botão
    // retangular entre os outros da barra.
    let is_close = matches!(glyph, WindowGlyph::Close);
    if response.hovered() {
        let bg = if is_close {
            Color32::from_rgb(0xC4, 0x3B, 0x3B)
        } else {
            theme::HOVER
        };
        painter.circle_filled(rect.center(), 12.0, bg);
    }
    let color = match (response.hovered(), is_close) {
        (true, true) => Color32::WHITE,
        (true, false) => theme::TEXT,
        (false, _) => theme::DIM,
    };

    let glyph_ch = match glyph {
        WindowGlyph::Minimize => theme::icon_glyph::MINUS,
        WindowGlyph::Maximize => theme::icon_glyph::SQUARE,
        WindowGlyph::Close => theme::icon_glyph::X,
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph_ch,
        theme::icon(15.0),
        color,
    );

    response
}

/// Botão de ação primária: preenchido no acento. Reservado pro momento em
/// que só existe UMA coisa a fazer na tela — escolher a pasta de música da
/// primeira vez. Fora disso o botão neutro do tema já basta: dar destaque de
/// acento a toda ação da interface viraria ruído, não hierarquia.
fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::body(), Color32::WHITE);
    let padding = vec2(14.0, 8.0);
    let (rect, response) = ui.allocate_exact_size(galley.size() + padding * 2.0, Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() {
        theme::ACCENT_BRIGHT
    } else {
        theme::ACCENT
    };
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    response
}

#[derive(Clone, Copy)]
enum Glyph {
    Prev,
    Play,
    Pause,
    Next,
}

/// Botão de transporte, com ícone da fonte Lucide.
fn transport(ui: &mut egui::Ui, glyph: Glyph) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 26.0), Sense::click());
    let painter = ui.painter();

    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), theme::HOVER);
    }
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::DIM
    };

    let ch = match glyph {
        Glyph::Play => theme::icon_glyph::PLAY,
        Glyph::Pause => theme::icon_glyph::PAUSE,
        Glyph::Prev => theme::icon_glyph::SKIP_BACK,
        Glyph::Next => theme::icon_glyph::SKIP_FORWARD,
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        ch,
        theme::icon(16.0),
        color,
    );

    response
}

fn format_ms(ms: Option<u64>) -> String {
    ms.map_or_else(
        || "--:--".into(),
        |ms| format_duration(Duration::from_millis(ms)),
    )
}

fn format_duration(d: Duration) -> String {
    let total = d.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}

/// Encurta pelo começo: o fim de um caminho é a parte que identifica a pasta.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let tail: String = text
        .chars()
        .skip(text.chars().count().saturating_sub(max - 1))
        .collect();
    format!("…{tail}")
}
