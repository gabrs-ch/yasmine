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

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, folder: Option<PathBuf>) -> Result<Self, String> {
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
        match folder {
            // Pasta vinda da linha de comando manda sobre a guardada.
            Some(folder) => app.set_root(folder),
            // Reescaneia a pasta guardada em segundo plano. Um rescan de
            // biblioteca intacta custa décimos de segundo e não abre arquivo
            // nenhum, então sai mais barato que pedir ao usuário que clique
            // em "Reescanear" — e músicas novas simplesmente aparecem.
            None => {
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
                self.reload();
                self.spawn_loudness_fill();
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
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if ui.button("Pasta…").clicked() {
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
            // A busca só filtra a biblioteca — dentro de uma playlist ela
            // ficaria filtrando contra o índice errado. Desabilitada, não
            // escondida: o texto continua ali para quando o usuário voltar.
            let in_library = self.source == Source::Library;
            ui.add_enabled_ui(in_library, |ui| {
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .desired_width(220.0)
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
                ui.add_space(4.0);
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
    }

    /// Barra lateral: biblioteca + playlists. Larga o bastante para nomes
    /// razoáveis, estreita o bastante para não roubar espaço da lista.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
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
                if ui
                    .add(egui::Button::new(egui::RichText::new("+").font(theme::mono())).small())
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
            });
        }

        if let Some(id) = pending_delete {
            self.delete_playlist(id);
        }
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
                    ui.painter().image(
                        self.mark.id(),
                        rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    ui.add_space(12.0);
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

                    // Zebra sutil: ajuda a percorrer uma lista longa sem
                    // acrescentar nenhuma linha ou borda.
                    if index % 2 == 1 {
                        painter.rect_filled(rect, CornerRadius::ZERO, theme::PANEL);
                    }
                    if is_selected {
                        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
                    }
                    if response.hovered() && !is_selected {
                        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
                    }
                    if is_playing {
                        // Marca de 2px na canaleta esquerda: o acento aparece
                        // aqui e na barra de progresso, em mais nenhum lugar.
                        painter.rect_filled(
                            Rect::from_min_size(rect.left_top(), vec2(2.0, rect.height())),
                            CornerRadius::ZERO,
                            theme::ACCENT,
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
                    cell(
                        painter,
                        cols.title(rect),
                        &row.title,
                        theme::body(),
                        title_color,
                    );
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
                        ui.label(egui::RichText::new(&row.title).color(theme::TEXT));
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

    /// Barra de progresso: 3px, sem cantos, o segundo e último uso do acento
    /// além da marca da faixa tocando. Compartilhada entre o player normal
    /// e o modo compacto — a única diferença entre os dois é a largura.
    fn progress_bar(&mut self, ui: &mut egui::Ui, state: player_audio::PlaybackState) {
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, 3.0), Sense::click_and_drag());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, theme::RULE);

        let fraction = state
            .duration
            .filter(|d| d.as_secs_f32() > 0.0)
            .map_or(0.0, |d| {
                (state.position.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0)
            });
        if fraction > 0.0 {
            painter.rect_filled(
                Rect::from_min_size(
                    rect.left_top(),
                    vec2(rect.width() * fraction, rect.height()),
                ),
                CornerRadius::ZERO,
                theme::ACCENT,
            );
        }

        if let (true, Some(total), Some(pointer)) = (
            response.clicked() || response.dragged(),
            state.duration,
            response.interact_pointer_pos(),
        ) {
            let target = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            self.engine
                .seek(Duration::from_secs_f32(total.as_secs_f32() * target));
        }
    }

    /// Desenha a capa da faixa atual num quadrado `size`x`size`. Único
    /// widget repetido entre o player normal e o modo compacto.
    fn draw_cover(&mut self, ui: &mut egui::Ui, size: f32) {
        let (cover, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(cover, CornerRadius::ZERO, theme::PANEL);
        let texture = self
            .now
            .as_ref()
            .and_then(|row| row.art_hash)
            .and_then(|hash| self.art.texture(&hash));
        if let Some(texture) = texture {
            painter.image(
                texture,
                cover,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            painter.rect_stroke(
                cover,
                CornerRadius::ZERO,
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
                .exact_size(84.0)
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ui, |ui| self.player_bar(ui));

            if !self.status.is_empty() {
                egui::Panel::bottom("status")
                    .exact_size(20.0)
                    .frame(egui::Frame::new().fill(theme::BG))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
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
    title: f32,
    artist: f32,
    album: f32,
    duration: f32,
}

impl Columns {
    const PAD: f32 = 10.0;

    fn new(width: f32) -> Self {
        let num = 38.0;
        let duration = 52.0;
        let rest = (width - num - duration - Self::PAD * 2.0).max(120.0);
        Self {
            num,
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
    fn title(&self, rect: Rect) -> Rect {
        Self::slice(rect, self.num, self.title)
    }
    fn artist(&self, rect: Rect) -> Rect {
        Self::slice(rect, self.num + self.title, self.artist)
    }
    fn album(&self, rect: Rect) -> Rect {
        Self::slice(rect, self.num + self.title + self.artist, self.album)
    }
    fn duration(&self, rect: Rect) -> Rect {
        Self::slice(
            rect,
            self.num + self.title + self.artist + self.album,
            self.duration,
        )
    }
}

/// Texto de uma célula, truncado na largura da coluna.
/// Uma linha da sidebar: rótulo alinhado à esquerda, marca de acento quando
/// ativa. Mesma linguagem visual da lista de faixas — a régua de 2px que
/// destaca "o que está tocando" aqui destaca "o que está selecionado".
fn sidebar_row(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, theme::ROW_HEIGHT), Sense::click());
    let painter = ui.painter();

    if active || response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
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
fn volume_slider(ui: &mut egui::Ui, value: f32) -> Option<f32> {
    let value = value.clamp(0.0, 1.0);
    let knob_radius = 5.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(56.0, knob_radius * 2.0 + 2.0), Sense::click_and_drag());
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
        painter.rect_filled(filled, radius, theme::DIM);
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

/// Alternador desenhado como ícone vetorial — mesma razão do transporte
/// (`Glyph`): traço nítido garantido, sem depender da fonte do sistema ter
/// o símbolo certo. Cantos retos, sem curva nenhuma: é ícone, não o slider
/// de volume, e aqui a regra do resto da interface vale.
fn icon_toggle(ui: &mut egui::Ui, icon: Icon, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::click());
    let painter = ui.painter();

    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
    }
    let color = if active {
        theme::TEXT
    } else if response.hovered() {
        theme::DIM
    } else {
        theme::FAINT
    };

    let c = rect.center();
    let s = 5.0;

    match icon {
        Icon::Shuffle => {
            // Dois caminhos cruzando — os dois trocando de lugar, que é o
            // que shuffle faz de verdade. Sem seta: numa área de 28x26,
            // ponta de seta vira ruído em vez de esclarecer.
            let stroke = Stroke::new(1.4, color);
            painter.line_segment([pos2(c.x - s, c.y - s), pos2(c.x + s, c.y + s)], stroke);
            painter.line_segment([pos2(c.x - s, c.y + s), pos2(c.x + s, c.y - s)], stroke);
        }
        Icon::Repeat(one) => {
            // Retângulo — um laço fechado, sem precisar de seta nem curva
            // pra sugerir "roda e volta". O dial de dica (hover) que já
            // existe explica o resto.
            let r = Rect::from_center_size(c, vec2(s * 2.0, s * 1.7));
            painter.rect_stroke(
                r,
                CornerRadius::ZERO,
                Stroke::new(1.3, color),
                egui::StrokeKind::Inside,
            );
            if one {
                painter.text(c, Align2::CENTER_CENTER, "1", theme::small(), color);
            }
        }
        Icon::Mini => {
            // Retângulo grande com um pequeno preenchido no canto — o ícone
            // universal de picture-in-picture, e cai de graça na linguagem
            // de cantos retos.
            let outer = Rect::from_center_size(c, vec2(s * 2.4, s * 2.0));
            painter.rect_stroke(
                outer,
                CornerRadius::ZERO,
                Stroke::new(1.2, color),
                egui::StrokeKind::Inside,
            );
            let inset = 1.5;
            let inner_size = vec2(s * 1.1, s * 0.9);
            let inner = Rect::from_min_size(
                pos2(
                    outer.right() - inset - inner_size.x,
                    outer.bottom() - inset - inner_size.y,
                ),
                inner_size,
            );
            painter.rect_filled(inner, CornerRadius::ZERO, color);
        }
    }

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

#[derive(Clone, Copy)]
enum Glyph {
    Prev,
    Play,
    Pause,
    Next,
}

/// Botão de transporte desenhado à mão.
///
/// Formas geométricas em vez de glifos de fonte: garante o traço nítido que a
/// direção visual pede e não depende de a fonte do sistema ter os símbolos.
fn transport(ui: &mut egui::Ui, glyph: Glyph) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 26.0), Sense::click());
    let painter = ui.painter();

    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
    }
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::DIM
    };

    let c = rect.center();
    let s = 5.0;
    match glyph {
        Glyph::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x - s * 0.6, c.y - s),
                    pos2(c.x - s * 0.6, c.y + s),
                    pos2(c.x + s, c.y),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Glyph::Pause => {
            for dx in [-3.0, 1.5] {
                painter.rect_filled(
                    Rect::from_min_size(pos2(c.x + dx, c.y - s), vec2(2.5, s * 2.0)),
                    CornerRadius::ZERO,
                    color,
                );
            }
        }
        Glyph::Prev | Glyph::Next => {
            let dir = if matches!(glyph, Glyph::Next) {
                1.0
            } else {
                -1.0
            };
            for offset in [-s * 0.9, s * 0.1] {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(c.x + dir * offset, c.y - s * 0.8),
                        pos2(c.x + dir * offset, c.y + s * 0.8),
                        pos2(c.x + dir * (offset + s * 0.8), c.y),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(
                        c.x + dir * s * 0.9 - if dir > 0.0 { 0.0 } else { 2.0 },
                        c.y - s * 0.8,
                    ),
                    vec2(2.0, s * 1.6),
                ),
                CornerRadius::ZERO,
                color,
            );
        }
    }

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
