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

use std::collections::HashMap;
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
use player_core::{ArtCache, ArtistId, Db, TrackId};
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

/// Chave no `meta` com o hash (hex) da foto que o usuário escolheu para a
/// linha "Your Library" da sidebar. Cosmético e local a este device — por
/// isso vive no `meta`, não numa tabela que sincroniza.
const META_LIBRARY_IMAGE: &str = "library_image";

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
    /// Só as faixas de um artista. O nome pra exibir fica em
    /// `App::artist_name` — `Source` é `Copy` e não carrega `String`.
    Artist(ArtistId),
}

/// A aba da sidebar abaixo de "Your Library" — playlists ou artistas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SideTab {
    Playlists,
    Artists,
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
    /// Capa de cada playlist, resolvida no `reload` e não a cada frame: o
    /// hash da escolha do usuário quando existe, senão até quatro capas das
    /// faixas (uma imagem, ou o mosaico 2×2). Ver `player_core::playlist`.
    playlist_covers: HashMap<Uuid, Vec<[u8; 32]>>,
    /// Foto escolhida para a linha "Your Library" (hash no cache de capas).
    library_image: Option<[u8; 32]>,
    /// Capa do cabeçalho da lista (playlist/artista aberto). `None` na
    /// biblioteca, que não tem cabeçalho.
    hero_art: Option<[u8; 32]>,
    view: Vec<TrackId>,
    /// Posição de cada item de `view` na playlist ativa — paralelo a `view`,
    /// vazio quando a fonte é a biblioteca. É o que permite "remover da
    /// playlist" e "mover" sem uma segunda ida ao banco por linha.
    view_positions: Vec<String>,
    query: String,
    sort: Sort,
    stats: Stats,
    playlists: Vec<Playlist>,
    /// Artistas com faixa local, pra aba "Artists" da sidebar. Recarregada
    /// junto com `playlists` — a lista só muda quando o índice muda.
    artists: Vec<library::ArtistBrief>,
    /// Qual lista a sidebar mostra abaixo de "Your Library".
    side_tab: SideTab,
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

    /// Nome do artista quando `source` é `Source::Artist` — só pra mostrar
    /// na sidebar. `None` em qualquer outra fonte.
    artist_name: Option<String>,

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
    /// Espelho de `status` no frame anterior + quando ele mudou pela última
    /// vez: o balãozinho de status some sozinho uns segundos depois de
    /// aparecer, sem cada `self.status = …` ter que carimbar a hora.
    status_prev: String,
    status_at: Instant,
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
            // Com mipmap: a marca de 256px aparece a 42px na sidebar (redução
            // de 6×) — sem os níveis intermediários o `Linear` sozinho serrilha
            // e lê como borrado. Mesmo tratamento das capas (ver `art.rs`).
            cc.egui_ctx.load_texture(
                "marca",
                color,
                egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)),
            )
        };

        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            art: ArtLoader::new(paths.cache.clone()),
            mark,
            db,
            paths,
            root,
            source: Source::Library,
            playlist_covers: HashMap::new(),
            library_image: None,
            hero_art: None,
            artist_name: None,
            view: Vec::new(),
            view_positions: Vec::new(),
            query: String::new(),
            sort: Sort::ArtistAlbum,
            stats: Stats::default(),
            playlists: Vec::new(),
            artists: Vec::new(),
            side_tab: SideTab::Playlists,
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
            status_prev: String::new(),
            status_at: Instant::now(),
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

        app.library_image = app
            .db
            .conn()
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [META_LIBRARY_IMAGE],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|hex| hex_decode(&hex));

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
            Source::Artist(id) => {
                self.view = library::by_artist(&self.db, id, self.sort).unwrap_or_default();
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
        self.artists = library::artists(&self.db).unwrap_or_default();
        self.selected = None;

        // Capas das playlists e do cabeçalho, resolvidas aqui e não a cada
        // frame: são consultas ao índice, e `reload` só roda quando algo
        // muda de verdade (troca de fonte, fim de scan, edição de playlist).
        self.playlist_covers = self
            .playlists
            .iter()
            .map(|pl| {
                (
                    pl.id,
                    playlist::cover_hashes(&self.db, pl.id).unwrap_or_default(),
                )
            })
            .collect();
        self.hero_art = self.resolve_hero_art();
    }

    /// Capa a mostrar no cabeçalho da lista: a da playlist (escolhida ou a
    /// primeira das faixas) ou a da primeira faixa do artista. `None` na
    /// biblioteca — lá não há cabeçalho.
    fn resolve_hero_art(&self) -> Option<[u8; 32]> {
        match self.source {
            Source::Library => None,
            Source::Playlist(id) => self
                .playlists
                .iter()
                .find(|pl| pl.id == id)
                .and_then(|pl| pl.image_hash)
                .or_else(|| {
                    self.playlist_covers
                        .get(&id)
                        .and_then(|c| c.first().copied())
                }),
            Source::Artist(_) => library::rows(&self.db, &self.view[..self.view.len().min(1)])
                .ok()
                .and_then(|mut rows| rows.pop())
                .and_then(|row| row.art_hash),
        }
    }

    /// Miniatura de uma playlist na sidebar: a foto escolhida pelo usuário,
    /// senão o mosaico 2×2 das capas das faixas, senão a primeira delas,
    /// senão um gradiente fixo pra aquela playlist.
    fn playlist_tile(&mut self, pl: &Playlist) -> Tile {
        if let Some(hash) = pl.image_hash
            && let Some(tex) = self.art.texture(&hash, false)
        {
            return Tile::Image(tex);
        }
        let covers = self
            .playlist_covers
            .get(&pl.id)
            .cloned()
            .unwrap_or_default();
        if covers.len() >= 4 {
            let ready: Vec<_> = covers
                .iter()
                .take(4)
                .filter_map(|hash| self.art.texture(hash, false))
                .collect();
            if let Ok(four) = <[egui::TextureId; 4]>::try_from(ready) {
                return Tile::Mosaic(four);
            }
        }
        if let Some(hash) = covers.first()
            && let Some(tex) = self.art.texture(hash, false)
        {
            return Tile::Image(tex);
        }
        let (a, b) = tile_gradient(pl.id.as_bytes()[0]);
        Tile::Gradient(a, b)
    }

    /// Miniatura da linha "Your Library": a foto escolhida pelo usuário, ou a
    /// marca do app.
    fn library_tile(&mut self) -> Tile {
        if let Some(hash) = self.library_image
            && let Some(tex) = self.art.texture(&hash, false)
        {
            return Tile::Image(tex);
        }
        Tile::Image(self.mark.id())
    }

    /// Troca a fonte da lista e recarrega. Sair de um filtro de artista pra
    /// qualquer coisa que não seja outro artista limpa o nome guardado.
    fn set_source(&mut self, source: Source) {
        if self.source == source {
            return;
        }
        if !matches!(source, Source::Artist(_)) {
            self.artist_name = None;
        }
        self.source = source;
        self.reload();
    }

    /// Passa a listar só as faixas de um artista. `name` vem da linha em que
    /// o usuário clicou — não custa uma consulta pra reencontrar.
    fn view_artist(&mut self, id: ArtistId, name: String) {
        self.artist_name = Some(name);
        self.set_source(Source::Artist(id));
    }

    /// Cria uma playlist e a deixa pronta para o usuário nomear.
    ///
    /// `track`, quando presente, já entra na lista nova — é o caso de "Nova
    /// playlist…" a partir do menu de contexto de uma faixa.
    fn create_playlist(&mut self, track: Option<TrackId>) {
        let Ok(id) = playlist::create(&self.db, "New Playlist") else {
            return;
        };
        if let Some(track) = track {
            let _ = playlist::append(&mut self.db, id, &[track]);
        }
        self.playlists = playlist::all(&self.db).unwrap_or_default();
        self.renaming = Some((id, "New Playlist".to_owned(), true));
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
            .set_title("Choose your music folder")
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
            .set_title("Choose a cover image")
            .add_filter("Image", &["jpg", "jpeg", "png", "webp", "bmp", "gif"])
            .pick_file()
        else {
            return;
        };

        let Ok(bytes) = std::fs::read(&path) else {
            self.status = format!("couldn't read {}", path.display());
            return;
        };

        let cache = ArtCache::new(self.paths.cache.clone());
        match player_core::art::set_album_art(&self.db, &cache, track, &bytes) {
            Ok(Some(_)) => self.reload(),
            Ok(None) => self.status = "that file isn't a valid image".into(),
            Err(err) => self.status = format!("couldn't save cover: {err}"),
        }
    }

    /// Pede uma imagem ao usuário e a materializa no cache de capas (mesmas
    /// miniaturas 96/512 das capas de álbum). Devolve o hash do blob, ou
    /// `None` se o usuário cancelou ou o arquivo não é imagem — o caminho de
    /// erro já deixa a mensagem em `self.status`.
    fn pick_image_into_cache(&mut self) -> Option<[u8; 32]> {
        let path = rfd::FileDialog::new()
            .set_title("Choose a photo")
            .add_filter("Image", &["jpg", "jpeg", "png", "webp", "bmp", "gif"])
            .pick_file()?;

        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                self.status = format!("couldn't read {}", path.display());
                return None;
            }
        };

        match ArtCache::new(self.paths.cache.clone()).store(&bytes) {
            Some(art) => Some(art.hash),
            None => {
                self.status = "that file isn't a valid image".into();
                None
            }
        }
    }

    fn set_playlist_image(&mut self, playlist_id: Uuid) {
        let Some(hash) = self.pick_image_into_cache() else {
            return;
        };
        let _ = playlist::set_image(&self.db, playlist_id, Some(&hash));
        self.reload();
    }

    fn clear_playlist_image(&mut self, playlist_id: Uuid) {
        let _ = playlist::set_image(&self.db, playlist_id, None);
        self.reload();
    }

    fn set_library_image(&mut self) {
        let Some(hash) = self.pick_image_into_cache() else {
            return;
        };
        let _ = self.db.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (META_LIBRARY_IMAGE, hex_encode(&hash)),
        );
        self.library_image = Some(hash);
    }

    fn clear_library_image(&mut self) {
        let _ = self
            .db
            .conn()
            .execute("DELETE FROM meta WHERE key = ?1", [META_LIBRARY_IMAGE]);
        self.library_image = None;
    }

    /// Vincula uma playlist a uma pasta escolhida pelo usuário: toda faixa
    /// que já está (ou vier a entrar, no próximo scan) dentro dela passa a
    /// fazer parte da playlist sozinha.
    fn link_playlist_folder(&mut self, playlist_id: Uuid) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Choose a folder to link to the playlist")
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
                self.status = "folder linked".into();
            }
            Ok(Err(player_core::playlist_folder::ForaDaBiblioteca)) => {
                self.status = "that folder is outside the current library".into();
            }
            Err(err) => self.status = format!("couldn't link folder: {err}"),
        }
    }

    fn unlink_playlist_folder(&mut self, playlist_id: Uuid, root_id: i64, rel_prefix: &str) {
        let _ = player_core::playlist_folder::unlink(&self.db, playlist_id, root_id, rel_prefix);
        self.status = "folder unlinked".into();
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
                // Vigiar é conveniência: sem ele resta o botão "Rescan".
                self.status = format!("couldn't watch folder: {err}");
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

        self.status = "scanning…".into();
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
                    "{} new · {} updated · {} removed · {} unchanged in {:.1}s",
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
            Err(err) => self.status = format!("scan failed: {err}"),
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
            self.status = "couldn't index the opened files".into();
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
            self.status = "couldn't find that track's file".into();
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
            ui.add_space(12.0);
            // Antes de escolher uma biblioteca, "Choose music folder…" é a
            // única coisa que dá pra fazer — fica como botão rotulado de ação
            // primária, e a tela de boas-vindas repete essa oferta grande.
            // Com biblioteca carregada, vira um ícone de pasta com o caminho
            // ao lado: reconfiguração ocasional não precisa de um botão de
            // texto ocupando a barra.
            if self.root.is_none() {
                if primary_button(ui, "Choose music folder…").clicked() {
                    self.pick_folder();
                }
            } else {
                if icon_button(ui, theme::icon_glyph::FOLDER)
                    .on_hover_text("Choose a different music folder")
                    .clicked()
                {
                    self.pick_folder();
                }
                if let Some(root) = &self.root {
                    let label = root.to_string_lossy();
                    ui.label(
                        egui::RichText::new(shorten(&label, 40))
                            .font(theme::small())
                            .color(theme::DIM),
                    );
                }
                if self.scan.is_none()
                    && icon_button(ui, theme::icon_glyph::REFRESH)
                        .on_hover_text("Rescan folder")
                        .clicked()
                    && let Some(root) = self.root.clone()
                {
                    self.start_scan(root);
                }
            }

            // A busca só filtra a biblioteca — dentro de uma playlist ela
            // ficaria filtrando contra o índice errado. Desabilitada, não
            // escondida: o texto continua ali para quando o usuário voltar.
            let in_library = self.source == Source::Library;
            let search_width = (ui.available_width() - 240.0).clamp(140.0, 380.0);
            // Centraliza o campo de busca na janela (no espírito do Spotify),
            // não colado no cluster da esquerda. `search_icon` + folga de -6
            // ocupam ~14px antes do campo.
            let cluster_w = 14.0 + search_width;
            let target = full.center().x - cluster_w / 2.0;
            let here = ui.cursor().left();
            ui.add_space((target - here).max(10.0));

            ui.add_enabled_ui(in_library, |ui| {
                search_icon(ui);
                ui.add_space(-6.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .desired_width(search_width)
                        .hint_text("Search your library"),
                );
                if std::mem::take(&mut self.focus_search) && in_library {
                    search.request_focus();
                }
                if search.changed() {
                    self.reload();
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Mesmo respiro do canto esquerdo, agora do lado direito.
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
                // Só enquanto escaneia: o resto do tempo o contexto da fonte
                // aberta mora no cabeçalho da lista, não aqui.
                if let Some(job) = &self.scan {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "scanning… {}",
                            job.progress.load(Ordering::Relaxed)
                        ))
                        .font(theme::small())
                        .color(theme::DIM),
                    );
                }
            });
        });
    }

    /// Barra lateral no estilo "Your Library" do Spotify: a biblioteca e cada
    /// playlist como uma linha com miniatura, nome e subtítulo.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;

        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Your Library")
                    .font(theme::strong(15.0))
                    .color(theme::TEXT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                if add_playlist_button(ui)
                    .on_hover_text("New playlist")
                    .clicked()
                {
                    self.create_playlist(None);
                }
            });
        });
        ui.add_space(8.0);

        let lib_tile = self.library_tile();
        let lib_sub = format!(
            "{} tracks · {} albums",
            self.stats.tracks, self.stats.albums
        );
        let lib = sidebar_entry(
            ui,
            &lib_tile,
            "Your Library",
            &lib_sub,
            self.source == Source::Library,
            None,
        );
        if lib.clicked() {
            self.set_source(Source::Library);
        }
        lib.context_menu(|ui| {
            if ui.button("Change photo…").clicked() {
                self.set_library_image();
                ui.close();
            }
            if self.library_image.is_some() && ui.button("Remove photo").clicked() {
                self.clear_library_image();
                ui.close();
            }
        });

        // Filtro de artista ativo: uma linha logo abaixo, marcada como a
        // fonte atual. Clicar em "Your Library" acima limpa o filtro.
        if let (Source::Artist(id), Some(name)) = (self.source, self.artist_name.clone()) {
            let (a, b) = tile_gradient((id.0 & 0xff) as u8);
            sidebar_entry(
                ui,
                &Tile::Gradient(a, b),
                &name,
                "Artist",
                true,
                Some(theme::icon_glyph::USER),
            );
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            if chip(ui, "Playlists", self.side_tab == SideTab::Playlists).clicked() {
                self.side_tab = SideTab::Playlists;
            }
            if chip(ui, "Artists", self.side_tab == SideTab::Artists).clicked() {
                self.side_tab = SideTab::Artists;
            }
        });
        ui.add_space(8.0);

        if self.side_tab == SideTab::Artists {
            self.artist_list(ui);
            return;
        }

        let playlists = self.playlists.clone();
        let mut pending_delete = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for pl in &playlists {
                    let is_active = self.source == Source::Playlist(pl.id);
                    let is_renaming = matches!(&self.renaming, Some((id, _, _)) if *id == pl.id);

                    if is_renaming {
                        let response = ui
                            .horizontal(|ui| {
                                ui.add_space(14.0);
                                let (_, buf, _) = self
                                    .renaming
                                    .as_mut()
                                    .expect("checado por is_renaming acima");
                                ui.add(
                                    egui::TextEdit::singleline(buf)
                                        .desired_width(ui.available_width() - 16.0)
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

                    let tile = self.playlist_tile(pl);
                    let sub = format!("Playlist · {} tracks", pl.items);
                    let row = sidebar_entry(ui, &tile, &pl.name, &sub, is_active, None);
                    if row.clicked() {
                        self.set_source(Source::Playlist(pl.id));
                    }
                    row.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            self.renaming = Some((pl.id, pl.name.clone(), true));
                            ui.close();
                        }
                        if ui.button("Change photo…").clicked() {
                            self.set_playlist_image(pl.id);
                            ui.close();
                        }
                        if pl.image_hash.is_some() && ui.button("Remove photo").clicked() {
                            self.clear_playlist_image(pl.id);
                            ui.close();
                        }
                        if ui.button("Delete").clicked() {
                            pending_delete = Some(pl.id);
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .button("Link folder…")
                            .on_hover_text(
                                "Every track in that folder joins this playlist automatically",
                            )
                            .clicked()
                        {
                            self.link_playlist_folder(pl.id);
                            ui.close();
                        }
                        for link in player_core::playlist_folder::links_for(&self.db, pl.id)
                            .unwrap_or_default()
                        {
                            let label = if link.rel_prefix.is_empty() {
                                "Unlink whole folder".to_owned()
                            } else {
                                format!("Unlink \"{}\"", link.rel_prefix)
                            };
                            if ui.button(label).clicked() {
                                self.unlink_playlist_folder(pl.id, link.root_id, &link.rel_prefix);
                                ui.close();
                            }
                        }
                    });
                }
            });

        if let Some(id) = pending_delete {
            self.delete_playlist(id);
        }
    }

    /// A aba "Artists" da sidebar: cada artista com faixa local, clicar abre
    /// `Source::Artist` (a mesma view do "Only tracks by …" do menu de faixa).
    fn artist_list(&mut self, ui: &mut egui::Ui) {
        if self.artists.is_empty() {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new("No artists indexed yet.")
                        .font(theme::small())
                        .color(theme::DIM),
                );
            });
            return;
        }

        let artists = self.artists.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for artist in &artists {
                    let active = self.source == Source::Artist(artist.id);
                    let (a, b) = tile_gradient((artist.id.0 & 0xff) as u8);
                    let sub = if artist.tracks == 1 {
                        "1 track".to_owned()
                    } else {
                        format!("{} tracks", artist.tracks)
                    };
                    let row = sidebar_entry(
                        ui,
                        &Tile::Gradient(a, b),
                        &artist.name,
                        &sub,
                        active,
                        Some(theme::icon_glyph::USER),
                    );
                    if row.clicked() {
                        self.view_artist(artist.id, artist.name.clone());
                    }
                }
            });
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
                    _ if self.root.is_none() => "Choose a music folder to get started.",
                    Source::Playlist(_) => {
                        "This playlist is empty. Right-click a track in your library to add it."
                    }
                    Source::Artist(_) => "No tracks by this artist.",
                    Source::Library if self.query.is_empty() => "No tracks indexed in this folder.",
                    Source::Library => "Nothing found.",
                };
                ui.label(egui::RichText::new(texto).color(theme::DIM));
                // A tela de boas-vindas ganha um botão de verdade, não só a
                // dica de texto — o ícone de pasta no topo já faz isso, mas
                // escondido num canto pequeno na primeira execução (tela em
                // branco, nada pra olhar) é fácil de não notar.
                if self.root.is_none() {
                    ui.add_space(16.0);
                    if primary_button(ui, "Choose music folder…").clicked() {
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

        // Cabeçalho da fonte aberta (playlist/artista): capa grande, nome e
        // contagem, sobre um banho do acento que desce até o `PANEL` — o
        // "hero" do Spotify, encolhido pra não engolir a janela. A biblioteca
        // não tem: ela já é o estado padrão, não precisa se anunciar.
        if !matches!(self.source, Source::Library) {
            let (hero, _) = ui.allocate_exact_size(vec2(width, 108.0), Sense::hover());
            let painter = ui.painter();
            let cr = CornerRadius {
                nw: theme::CARD_RADIUS,
                ne: theme::CARD_RADIUS,
                sw: 0,
                se: 0,
            };
            // Degradê do roxo (topo) até o `PANEL` (base), pra o hero se
            // dissolver na lista em vez de terminar numa borda. `epaint` não
            // tem pincel de degradê, então são quatro faixas opacas de altura
            // decrescente — cada uma com os cantos de cima arredondados como
            // o cartão, o que evita o "dente" quadrado no canto.
            painter.rect_filled(hero, cr, theme::PANEL);
            let top = Color32::from_rgb(0x3B, 0x2D, 0x6D);
            for (frac, t) in [(1.0_f32, 0.78_f32), (0.70, 0.52), (0.44, 0.28), (0.22, 0.0)] {
                let band = Rect::from_min_max(
                    hero.min,
                    pos2(hero.right(), hero.top() + hero.height() * frac),
                );
                painter.rect_filled(band, cr, lerp_color(top, theme::PANEL, t));
            }

            let cover = Rect::from_min_size(
                pos2(hero.left() + 22.0, hero.bottom() - 16.0 - 80.0),
                vec2(80.0, 80.0),
            );
            image_shadow(painter, cover, theme::RADIUS);
            if let Some(texture) = self.hero_art.and_then(|hash| self.art.texture(&hash, true)) {
                rounded_image(painter, cover, texture, theme::RADIUS);
            } else {
                let seed = match self.source {
                    Source::Playlist(id) => id.as_bytes()[0],
                    Source::Artist(id) => (id.0 & 0xff) as u8,
                    Source::Library => 0,
                };
                let (a, b) = tile_gradient(seed);
                gradient_fill(painter, cover, f32::from(theme::RADIUS), a, b);
            }

            let (eyebrow, name, count) = match self.source {
                Source::Playlist(id) => (
                    "PLAYLIST",
                    self.playlists
                        .iter()
                        .find(|pl| pl.id == id)
                        .map_or_else(String::new, |pl| pl.name.clone()),
                    self.view.len(),
                ),
                Source::Artist(_) => (
                    "ARTIST",
                    self.artist_name.clone().unwrap_or_default(),
                    self.view.len(),
                ),
                Source::Library => ("", String::new(), 0),
            };
            let tx = cover.right() + 16.0;
            let tw = (hero.right() - 20.0 - tx).max(0.0);
            painter.text(
                pos2(tx, cover.top() + 3.0),
                Align2::LEFT_TOP,
                eyebrow,
                theme::small(),
                theme::DIM,
            );
            cell(
                painter,
                Rect::from_min_size(pos2(tx, cover.center().y - 17.0), vec2(tw, 34.0)),
                &name,
                theme::strong(24.0),
                theme::TEXT,
            );
            painter.text(
                pos2(tx, cover.bottom() - 2.0),
                Align2::LEFT_BOTTOM,
                format!("{count} tracks"),
                theme::small(),
                theme::DIM,
            );
        }

        // Cabeçalho de colunas: rótulos apagados e uma régua de 1px.
        let (header, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
        let painter = ui.painter();
        for (label, rect) in [
            ("#", cols.num(header)),
            ("TITLE", cols.title(header)),
            ("ARTIST", cols.artist(header)),
            ("ALBUM", cols.album(header)),
            ("TIME", cols.duration(header)),
        ] {
            // A coluna de capa não tem cabeçalho — imagem não precisa de
            // rótulo, e "COVER" ocuparia espaço sem informar nada.
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
                pos2(header.left() + 6.0, header.bottom() - 0.5),
                pos2(header.right() - 6.0, header.bottom() - 0.5),
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
                    if let Some(texture) =
                        row.art_hash.and_then(|hash| self.art.texture(&hash, false))
                    {
                        image_shadow(painter, art_rect, theme::RADIUS_SM);
                        rounded_image(painter, art_rect, texture, theme::RADIUS_SM);
                    } else {
                        painter.rect_filled(
                            art_rect,
                            CornerRadius::same(theme::RADIUS_SM),
                            theme::PANEL,
                        );
                    }

                    let title_color = if is_playing {
                        theme::ACCENT_BRIGHT
                    } else {
                        theme::TEXT
                    };
                    if is_playing {
                        // No lugar do número da faixa: as barrinhas de "isto
                        // está tocando", no acento — a mesma pista visual do
                        // Spotify.
                        let num = cols.num(rect);
                        painter.text(
                            pos2(num.left() + 4.0, num.center().y),
                            Align2::LEFT_CENTER,
                            theme::icon_glyph::AUDIO_LINES,
                            theme::icon(13.0),
                            theme::ACCENT_BRIGHT,
                        );
                    } else {
                        cell(
                            painter,
                            cols.num(rect),
                            &row.track_no.map_or_else(String::new, |n| n.to_string()),
                            theme::mono(),
                            theme::FAINT,
                        );
                    }
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
                    let artist = row.artist_id.zip(row.artist.clone());
                    response.context_menu(|ui| {
                        if let Some((aid, aname)) = &artist
                            && !matches!(self.source, Source::Artist(id) if id == *aid)
                        {
                            if ui.button(format!("Only tracks by {aname}")).clicked() {
                                self.view_artist(*aid, aname.clone());
                                ui.close();
                            }
                            ui.separator();
                        }

                        ui.menu_button("Add to playlist", |ui| {
                            for pl in &playlists {
                                if ui.button(&pl.name).clicked() {
                                    self.add_to_playlist(pl.id, track);
                                    ui.close();
                                }
                            }
                            if !playlists.is_empty() {
                                ui.separator();
                            }
                            if ui.button("New playlist…").clicked() {
                                self.create_playlist(Some(track));
                                ui.close();
                            }
                        });

                        if matches!(source, Source::Playlist(_)) {
                            ui.separator();
                            let up = ui.add_enabled(index > 0, egui::Button::new("Move up"));
                            if up.clicked() {
                                self.nudge_in_playlist(index, -1);
                                ui.close();
                            }
                            let down =
                                ui.add_enabled(index + 1 < total, egui::Button::new("Move down"));
                            if down.clicked() {
                                self.nudge_in_playlist(index, 1);
                                ui.close();
                            }
                            if ui.button("Remove from playlist").clicked() {
                                self.remove_from_playlist(index);
                                ui.close();
                            }
                        }

                        ui.separator();
                        if ui.button("Choose album cover…").clicked() {
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
        // Fio quase invisível na borda de cima — o mesmo do mockup
        // (`rgba(255,255,255,.06)`): o player pousa sobre o vão um degrau à
        // frente do conteúdo, sem uma régua dura marcando a divisa.
        ui.painter().line_segment(
            [full.left_top(), full.right_top()],
            Stroke::new(1.0, Color32::from_white_alpha(14)),
        );

        // Três faixas de largura fixa nas pontas e o miolo no meio: o
        // transporte fica centrado na JANELA, não no espaço que sobra depois
        // da faixa tocando. Rects explícitos (não layout que "vai empurrando")
        // porque centrar de verdade era o ponto.
        let left_w = (full.width() * 0.30).clamp(240.0, 320.0);
        let right_w = (full.width() * 0.26).clamp(170.0, 220.0);
        let left_rect = Rect::from_min_max(full.min, pos2(full.left() + left_w, full.bottom()));
        let right_rect = Rect::from_min_max(pos2(full.right() - right_w, full.top()), full.max);
        let center_rect = Rect::from_min_max(
            pos2(left_rect.right() + 12.0, full.top()),
            pos2(right_rect.left() - 12.0, full.bottom()),
        );

        // ---- Esquerda: capa + faixa tocando ----
        {
            let mut ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let ui = &mut ui;
            ui.add_space(14.0);
            self.draw_cover(ui, 56.0);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.add_space(9.0);
                match &self.now {
                    Some(row) => {
                        ui.label(
                            egui::RichText::new(shorten(&row.title, 26))
                                .font(theme::strong(13.0))
                                .color(theme::TEXT),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(shorten(
                                &format!(
                                    "{}  ·  {}",
                                    row.artist.as_deref().unwrap_or("—"),
                                    row.album.as_deref().unwrap_or("—")
                                ),
                                36,
                            ))
                            .font(theme::small())
                            .color(theme::DIM),
                        );
                    }
                    None => {
                        ui.label(
                            egui::RichText::new("Nothing playing")
                                .font(theme::small())
                                .color(theme::FAINT),
                        );
                    }
                }
            });
        }

        // ---- Centro: transporte por cima, progresso embaixo, os dois
        // centrados horizontalmente em `center_rect` ----
        {
            let mut ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(center_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            let ui = &mut ui;
            ui.add_space(((center_rect.height() - 60.0) * 0.5).max(0.0));

            ui.allocate_ui_with_layout(
                vec2(220.0, 34.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 13.0;
                    if icon_toggle(ui, Icon::Shuffle, self.queue.shuffle())
                        .on_hover_text("Shuffle (S)")
                        .clicked()
                    {
                        let on = !self.queue.shuffle();
                        self.queue.set_shuffle(on);
                        self.queue_next();
                    }
                    if transport(ui, Glyph::Prev).clicked() {
                        self.prev_track();
                    }
                    if transport_primary(ui, state.playing).clicked() {
                        self.toggle_play();
                    }
                    if transport(ui, Glyph::Next).clicked() {
                        self.next_track();
                    }
                    let repeat_hint = match self.queue.repeat() {
                        Repeat::Off => "Repeat: off (R)",
                        Repeat::All => "Repeat: all (R)",
                        Repeat::One => "Repeat: one (R)",
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
                },
            );

            ui.add_space(9.0);
            let bar_total = (center_rect.width() - 8.0).clamp(200.0, 520.0);
            ui.allocate_ui_with_layout(
                vec2(bar_total, 18.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.label(
                        egui::RichText::new(format_duration(state.position))
                            .font(theme::mono())
                            .color(theme::FAINT),
                    );
                    let bar_w = (ui.available_width() - 48.0).max(80.0);
                    self.progress_bar(ui, state, bar_w);
                    ui.label(
                        egui::RichText::new(
                            state
                                .duration
                                .map_or_else(|| "--:--".into(), format_duration),
                        )
                        .font(theme::mono())
                        .color(theme::FAINT),
                    );
                },
            );
        }

        // ---- Direita: volume, modo compacto, posição na fila ----
        {
            let mut ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            let ui = &mut ui;
            ui.add_space(16.0);
            if let Some(volume) = volume_slider(ui, self.engine.volume()) {
                self.set_volume(volume);
            }
            ui.add_space(6.0);
            glyph_label(ui, theme::icon_glyph::VOLUME, 14.0, theme::DIM);
            ui.add_space(10.0);
            if icon_toggle(ui, Icon::Mini, self.mini)
                .on_hover_text("Compact mode (Ctrl+M)")
                .clicked()
            {
                self.toggle_mini(ui.ctx());
            }
            if let Some(position) = self.queue.position() {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!("{position} / {}", self.queue.len()))
                        .font(theme::mono())
                        .color(theme::FAINT),
                );
            }
        }
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
    fn progress_bar(&mut self, ui: &mut egui::Ui, state: player_audio::PlaybackState, width: f32) {
        const HIT_HEIGHT: f32 = 20.0;
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
            .and_then(|hash| self.art.texture(&hash, true));
        if let Some(texture) = texture {
            rounded_image(painter, cover, texture, radius);
        }
        // Fio interno claríssimo em volta da capa — o mesmo `inset` do mockup.
        // Some a borda dura contra o vão e dá um respiro entre a arte e o
        // fundo preto do player, com capa ou sem.
        painter.rect_stroke(
            cover,
            CornerRadius::same(radius),
            Stroke::new(
                1.0,
                if texture.is_some() {
                    Color32::from_white_alpha(20)
                } else {
                    theme::RULE
                },
            ),
            egui::StrokeKind::Inside,
        );
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
                            egui::RichText::new("Nothing playing")
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
                    .on_hover_text("Exit compact mode (Ctrl+M)")
                    .clicked()
                {
                    self.toggle_mini(ui.ctx());
                }
                ui.add_space(6.0);
            });
        });

        ui.add_space(6.0);
        let bar_w = ui.available_width() - 16.0;
        self.progress_bar(ui, state, bar_w);
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

        // No modo compacto a janela fica sempre por cima — o ponto dela é
        // ficar visível num canto enquanto se usa outra coisa. Volta ao
        // normal ao sair.
        let level = if self.mini {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
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

        // O balãozinho de status é um aviso passageiro ("140 new…", "folder
        // linked", um erro), não um rodapé permanente: some uns segundos
        // depois de a mensagem parar de mudar. Detectar a mudança aqui num
        // canto só evita carimbar a hora em cada `self.status = …`.
        const STATUS_TTL: Duration = Duration::from_secs(6);
        if self.status != self.status_prev {
            self.status_prev = self.status.clone();
            self.status_at = Instant::now();
        }
        if !self.status.is_empty() {
            if self.status_at.elapsed() >= STATUS_TTL {
                self.status.clear();
                self.status_prev.clear();
            } else {
                ctx.request_repaint_after(STATUS_TTL - self.status_at.elapsed());
            }
        }

        if self.mini {
            // Modo compacto: a janela inteira é a barra do player, sem
            // topo, sidebar ou lista — é para isso que ela existe.
            egui::CentralPanel::no_frame()
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ui, |ui| self.mini_bar(ui));
        } else {
            // A barra de comando e a barra do player ficam sobre o vão
            // (`BG`), sem fundo próprio — no espírito do Spotify, onde só a
            // sidebar e o conteúdo são "cartões" e o resto é a moldura
            // escura da janela.
            egui::Panel::top("comando")
                .exact_size(46.0)
                .frame(egui::Frame::new().fill(theme::BG))
                .show(ui, |ui| self.top_bar(ui));

            egui::Panel::bottom("player")
                .exact_size(88.0)
                .frame(egui::Frame::new().fill(theme::BG))
                .show(ui, |ui| self.player_bar(ui));

            // Sidebar e lista viram cartões arredondados: preenchimento
            // `PANEL` (um degrau acima do vão), cantos `CARD_RADIUS`, um fio
            // discreto no contorno pra o canto pegar luz, e uma folga
            // separando um do outro e da borda da janela.
            let card = |left: i8, right: i8| {
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .corner_radius(theme::CARD_RADIUS)
                    .stroke(Stroke::new(1.0, theme::CARD_STROKE))
                    .outer_margin(egui::Margin {
                        left,
                        right,
                        top: 2,
                        bottom: 6,
                    })
            };

            egui::Panel::left("sidebar")
                .exact_size(264.0)
                .resizable(false)
                .frame(card(8, 4).inner_margin(egui::Margin {
                    left: 0,
                    right: 0,
                    top: 6,
                    bottom: 2,
                }))
                .show(ui, |ui| self.sidebar(ui));

            egui::CentralPanel::no_frame()
                .frame(card(4, 8).inner_margin(egui::Margin::ZERO))
                .show(ui, |ui| self.list(ui));

            // Status de scan / erro: um balãozinho flutuante no canto de
            // baixo, não uma faixa de largura inteira com régua — essa faixa
            // era uma das "linhas de divisão duras" que sobravam. Some sozinho
            // no próximo scan.
            if !self.status.is_empty() {
                let content = ctx.content_rect();
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("status-toast"),
                ));
                let galley =
                    painter.layout_no_wrap(self.status.clone(), theme::small(), theme::DIM);
                let pad = vec2(12.0, 6.0);
                let rect = Rect::from_min_size(
                    pos2(
                        content.left() + 284.0,
                        content.bottom() - 88.0 - 12.0 - (galley.size().y + pad.y * 2.0),
                    ),
                    galley.size() + pad * 2.0,
                );
                painter.rect(
                    rect,
                    CornerRadius::same(theme::RADIUS),
                    theme::ELEVATED,
                    Stroke::new(1.0, theme::RULE),
                    egui::StrokeKind::Inside,
                );
                painter.galley(rect.min + pad, galley, theme::DIM);
            }
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

/// Oito duplas de cor para a miniatura sem capa, escolhidas pelo primeiro
/// byte do id — determinístico, então a mesma playlist tem sempre o mesmo
/// gradiente. Metade puxa pro roxo da identidade, metade não, pra sidebar
/// não virar um degradê monocromático.
fn tile_gradient(seed: u8) -> (Color32, Color32) {
    const G: [(Color32, Color32); 8] = [
        (
            Color32::from_rgb(0x7C, 0x5C, 0xFF),
            Color32::from_rgb(0x3A, 0x1F, 0x8C),
        ),
        (
            Color32::from_rgb(0x1F, 0x6F, 0x5C),
            Color32::from_rgb(0x0B, 0x3B, 0x3F),
        ),
        (
            Color32::from_rgb(0xB8, 0x42, 0x2F),
            Color32::from_rgb(0x5A, 0x1F, 0x1F),
        ),
        (
            Color32::from_rgb(0x2D, 0x5F, 0xB8),
            Color32::from_rgb(0x12, 0x24, 0x4F),
        ),
        (
            Color32::from_rgb(0xC2, 0x4D, 0x7E),
            Color32::from_rgb(0x4B, 0x15, 0x28),
        ),
        (
            Color32::from_rgb(0xD7, 0x9A, 0x2A),
            Color32::from_rgb(0x5A, 0x38, 0x06),
        ),
        (
            Color32::from_rgb(0x4B, 0x4F, 0x57),
            Color32::from_rgb(0x1B, 0x1D, 0x22),
        ),
        (
            Color32::from_rgb(0x59, 0x42, 0xB8),
            Color32::from_rgb(0x24, 0x1D, 0x3F),
        ),
    ];
    G[(seed % 8) as usize]
}

/// O que desenhar na miniatura de uma linha da sidebar.
enum Tile {
    /// Uma capa (escolhida pelo usuário, ou a da primeira faixa).
    Image(egui::TextureId),
    /// Mosaico 2×2 das capas das faixas — o fallback do Spotify quando a
    /// playlist não tem uma capa só.
    Mosaic([egui::TextureId; 4]),
    /// Sem capa nenhuma: um gradiente determinístico, sempre o mesmo pra
    /// aquela playlist, pra não ficar um buraco cinza.
    Gradient(Color32, Color32),
}

/// Uma linha da sidebar no estilo Spotify: miniatura quadrada arredondada +
/// nome + subtítulo ("Playlist · 42 tracks"). Fundo arredondado em hover, um
/// degrau acima (`ELEVATED`) e nome no acento quando é a fonte aberta.
fn sidebar_entry(
    ui: &mut egui::Ui,
    tile: &Tile,
    title: &str,
    subtitle: &str,
    active: bool,
    glyph: Option<char>,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 54.0), Sense::click());
    let painter = ui.painter();

    if active || response.hovered() {
        let bg = if active {
            theme::ELEVATED
        } else {
            theme::HOVER
        };
        painter.rect_filled(
            rect.shrink2(vec2(6.0, 3.0)),
            CornerRadius::same(theme::RADIUS),
            bg,
        );
    }

    let thumb = Rect::from_min_size(
        pos2(rect.left() + 14.0, rect.center().y - 21.0),
        vec2(42.0, 42.0),
    );
    let r = theme::RADIUS_SM;
    image_shadow(painter, thumb, r);
    match tile {
        Tile::Image(tex) => rounded_image(painter, thumb, *tex, r),
        Tile::Mosaic(t) => {
            let h = thumb.width() / 2.0;
            let q = |dx: f32, dy: f32| {
                Rect::from_min_size(pos2(thumb.left() + dx, thumb.top() + dy), vec2(h, h))
            };
            rounded_image_cr(
                painter,
                q(0.0, 0.0),
                t[0],
                CornerRadius {
                    nw: r,
                    ne: 0,
                    sw: 0,
                    se: 0,
                },
            );
            rounded_image_cr(
                painter,
                q(h, 0.0),
                t[1],
                CornerRadius {
                    nw: 0,
                    ne: r,
                    sw: 0,
                    se: 0,
                },
            );
            rounded_image_cr(
                painter,
                q(0.0, h),
                t[2],
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: r,
                    se: 0,
                },
            );
            rounded_image_cr(
                painter,
                q(h, h),
                t[3],
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 0,
                    se: r,
                },
            );
        }
        Tile::Gradient(a, b) => {
            gradient_fill(painter, thumb, f32::from(r), *a, *b);
            if let Some(ch) = glyph {
                painter.text(
                    thumb.center(),
                    Align2::CENTER_CENTER,
                    ch,
                    theme::icon(18.0),
                    Color32::from_white_alpha(200),
                );
            }
        }
    }

    let text_left = thumb.right() + 11.0;
    let text_w = (rect.right() - 10.0 - text_left).max(0.0);
    let title_rect =
        Rect::from_min_size(pos2(text_left, rect.center().y - 15.0), vec2(text_w, 16.0));
    let sub_rect = Rect::from_min_size(pos2(text_left, rect.center().y + 1.0), vec2(text_w, 14.0));
    let title_color = if active {
        theme::ACCENT_BRIGHT
    } else {
        theme::TEXT
    };
    cell(painter, title_rect, title, theme::strong(13.0), title_color);
    cell(painter, sub_rect, subtitle, theme::small(), theme::DIM);

    response
}

/// Desenha `texture` dentro de `rect` com cantos arredondados — imagem com
/// máscara de raio, não o retângulo reto que `Painter::image` desenha.
/// `RectShape` do epaint aceita textura (`brush`) e `corner_radius` juntos;
/// é essa combinação que faz a capa ficar arredondada sem cortar a imagem
/// à mão.
fn rounded_image(painter: &egui::Painter, rect: Rect, texture: egui::TextureId, radius: u8) {
    rounded_image_cr(painter, rect, texture, CornerRadius::same(radius));
}

/// Sombra difusa atrás de uma capa — o degrau que faz a arte "flutuar" sobre
/// o cartão em vez de estar recortada nele. É a mesma ideia do `box-shadow`
/// do mockup; como o `epaint::Shadow` só borra pra fora, desenha antes da
/// imagem. Só vale a pena sobre o `PANEL` (claro): sobre o vão quase preto,
/// preto sobre preto não aparece.
fn image_shadow(painter: &egui::Painter, rect: Rect, radius: u8) {
    let shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    painter.add(shadow.as_shape(rect, CornerRadius::same(radius)));
}

/// Como [`rounded_image`], mas com raio por canto — o mosaico 2×2 arredonda
/// só o canto externo de cada quadrante.
fn rounded_image_cr(
    painter: &egui::Painter,
    rect: Rect,
    texture: egui::TextureId,
    corner_radius: CornerRadius,
) {
    let mut shape = egui::epaint::RectShape::filled(rect, corner_radius, Color32::WHITE);
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

/// O botão de play/pause central — círculo claro preenchido, glifo escuro,
/// maior que os vizinhos. É o único controle da barra que "salta": no
/// Spotify é o mesmo desenho, e faz sentido, é o que a mão procura primeiro.
fn transport_primary(ui: &mut egui::Ui, playing: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
    let painter = ui.painter();

    let fill = if response.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT
    };
    painter.circle_filled(rect.center(), 17.0, fill);
    let ch = if playing {
        theme::icon_glyph::PAUSE
    } else {
        theme::icon_glyph::PLAY
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        ch,
        theme::icon(15.0),
        theme::BG,
    );

    response
}

/// Pílula de alternância da sidebar ("Playlists" / "Artists"). Ativa = fundo
/// claro, texto escuro (igual ao mockup); inativa = um degrau acima do
/// cartão, texto apagado.
fn chip(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(76.0, 24.0), Sense::click());
    let painter = ui.painter();
    let (bg, fg) = if active {
        (theme::TEXT, theme::BG)
    } else if response.hovered() {
        (theme::HOVER, theme::TEXT)
    } else {
        (theme::ELEVATED, theme::DIM)
    };
    painter.rect_filled(rect, CornerRadius::same(12), bg);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::small(),
        fg,
    );
    response
}

/// Um glifo da fonte de ícones como widget num layout — o `painter.text` dos
/// outros botões não aloca espaço, e num `horizontal` isso desalinha o resto.
fn glyph_label(ui: &mut egui::Ui, glyph: char, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size + 4.0, size + 4.0), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        theme::icon(size),
        color,
    );
}

/// Botão de ícone genérico — o mesmo tratamento do transporte (fundo de
/// hover arredondado, glifo neutro), pra qualquer glifo da fonte de ícones.
/// Quem chama põe o `on_hover_text` explicando o que faz, já que sem rótulo
/// o ícone sozinho nem sempre é óbvio.
fn icon_button(ui: &mut egui::Ui, glyph: char) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::click());
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
        glyph,
        theme::icon(15.0),
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

/// Hash de 32 bytes em hex — o formato em que a foto da biblioteca fica
/// guardada no `meta` (que só aceita texto).
fn hex_encode(hash: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    hash.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn hex_decode(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex.trim();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(bytes.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
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
