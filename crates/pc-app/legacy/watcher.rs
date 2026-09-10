//! Vigia a pasta de música e avisa quando algo muda.
//!
//! # Rescan inteiro, não escaneamento cirúrgico
//!
//! Dá para olhar o evento, descobrir exatamente qual arquivo mudou e tocar só
//! nele. Mas o rescan incremental já custa décimos de segundo numa biblioteca
//! de 50 000 faixas — ele nem abre os arquivos cujo `(tamanho, mtime)` bate —
//! e é o mesmo caminho de código que já está testado. Escaneamento cirúrgico
//! só acrescentaria casos de borda (arquivo movido, pasta renomeada, cópia
//! ainda em andamento) por um ganho que não aparece no relógio.
//!
//! # A rajada
//!
//! Copiar um álbum para dentro da pasta dispara dezenas de eventos em
//! sequência. Reagir a cada um seria dezenas de rescans, e ainda por cima
//! sobre arquivos pela metade. Por isso a espera por silêncio: o aviso só sai
//! depois de [`QUIET`] sem nenhum evento novo.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher as _};

use player_core::scan::AUDIO_EXT;

/// Silêncio necessário antes de considerar a rajada terminada.
///
/// Longo o suficiente para uma cópia de álbum inteiro caber numa rajada só,
/// curto o suficiente para arrastar um arquivo e vê-lo aparecer.
const QUIET: Duration = Duration::from_millis(800);

pub struct Watcher {
    /// Precisa continuar vivo: soltar o valor cancela a vigilância.
    _watcher: notify::RecommendedWatcher,
    changes: Receiver<()>,
    root: PathBuf,
}

impl Watcher {
    /// Começa a vigiar `root`.
    ///
    /// `wake` é chamado quando uma rajada termina. Ele existe porque a janela
    /// só redesenha quando acontece alguma coisa: sem alguém cutucando, um
    /// app ocioso nunca chegaria a perguntar se algo mudou, e o arquivo novo
    /// só apareceria no próximo clique.
    pub fn new(root: &Path, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        let (raw_tx, raw_rx) = channel();
        let (quiet_tx, quiet_rx) = channel();

        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event
                    && interessa(&event)
                {
                    let _ = raw_tx.send(());
                }
            })
            .map_err(|err| err.to_string())?;

        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|err| err.to_string())?;

        std::thread::Builder::new()
            .name("vigia".into())
            .spawn(move || coalesce(&raw_rx, &quiet_tx, &wake))
            .map_err(|err| err.to_string())?;

        Ok(Self {
            _watcher: watcher,
            changes: quiet_rx,
            root: root.to_path_buf(),
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Houve mudança desde a última consulta? Não bloqueia.
    ///
    /// Consome tudo o que estiver pendente: várias rajadas viram um rescan só.
    pub fn take_change(&self) -> bool {
        let mut changed = false;
        loop {
            match self.changes.try_recv() {
                Ok(()) => changed = true,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return changed,
            }
        }
    }
}

/// Só arquivos de áudio e mudanças estruturais importam.
///
/// Sem este filtro, qualquer `.jpg`, `.nfo` ou arquivo temporário de outro
/// programa dentro da pasta dispararia um rescan da biblioteca inteira.
fn interessa(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|path| {
        // Sem extensão pode ser um diretório criado, renomeado ou removido —
        // isso muda a biblioteca e precisa passar.
        path.extension()
            .and_then(|e| e.to_str())
            .is_none_or(|ext| AUDIO_EXT.contains(&ext.to_ascii_lowercase().as_str()))
    })
}

/// Junta uma rajada de eventos num aviso só.
fn coalesce(raw: &Receiver<()>, quiet: &Sender<()>, wake: &(impl Fn() + ?Sized)) {
    loop {
        // Espera indefinidamente pelo primeiro evento da rajada.
        if raw.recv().is_err() {
            return;
        }
        // Depois disso, espera o silêncio.
        loop {
            match raw.recv_timeout(QUIET) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        if quiet.send(()).is_err() {
            return;
        }
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::time::Instant;

    fn dir(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("player-watch-{nome}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("criar pasta");
        dir
    }

    fn espera_mudanca(watcher: &Watcher, limite: Duration) -> bool {
        let inicio = Instant::now();
        while inicio.elapsed() < limite {
            if watcher.take_change() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn avisa_quando_entra_arquivo_de_audio() {
        let root = dir("audio");
        let watcher = Watcher::new(&root, || {}).expect("vigiar");

        fs::write(root.join("nova.mp3"), b"qualquer coisa").expect("gravar");
        assert!(espera_mudanca(&watcher, Duration::from_secs(5)));
    }

    /// Sem o filtro, qualquer arquivo solto na pasta custaria um rescan.
    #[test]
    fn ignora_arquivo_que_nao_e_audio() {
        let root = dir("nao-audio");
        let watcher = Watcher::new(&root, || {}).expect("vigiar");

        fs::write(root.join("capa.jpg"), b"nao sou musica").expect("gravar");
        fs::write(root.join("notas.txt"), b"nem eu").expect("gravar");
        assert!(!espera_mudanca(&watcher, Duration::from_secs(2)));
    }

    /// Copiar um álbum inteiro tem que dar um rescan, não trinta.
    #[test]
    fn rajada_vira_um_aviso_so() {
        let root = dir("rajada");
        let watcher = Watcher::new(&root, || {}).expect("vigiar");

        for n in 0..30 {
            fs::write(root.join(format!("{n:02}.mp3")), b"faixa").expect("gravar");
        }
        assert!(espera_mudanca(&watcher, Duration::from_secs(5)));
        // Passada a rajada, nada mais fica pendente.
        assert!(!espera_mudanca(&watcher, Duration::from_secs(2)));
    }

    #[test]
    fn pasta_nova_conta_como_mudanca() {
        let root = dir("subpasta");
        let watcher = Watcher::new(&root, || {}).expect("vigiar");

        fs::create_dir(root.join("Álbum Novo")).expect("criar subpasta");
        assert!(espera_mudanca(&watcher, Duration::from_secs(5)));
    }
}
