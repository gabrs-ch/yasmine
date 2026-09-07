//! O worker que amarra decodificador e dispositivo.
//!
//! Uma thread só, dona do decodificador e do `Stream`. Ela decodifica,
//! converte se precisar, e empurra samples no ring buffer. Toda a
//! comunicação com o resto do programa é por canal (comandos e eventos) e por
//! átomos (posição, estado).
//!
//! # Por que uma thread própria
//!
//! O `cpal::Stream` não é `Send` no Linux: quem cria tem que segurar. E
//! decodificar na thread da UI faria a interface engasgar a cada bloco de
//! áudio. Então: UI manda comando, worker trabalha, callback só copia.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use crate::convert::Converter;
use crate::decode::TrackDecoder;
use crate::output::{Output, Shared};
use crate::{Command, Event, PlaybackState, Result};

/// Quanto o worker dorme quando não há nada tocando.
const IDLE: Duration = Duration::from_millis(50);

/// Handle do motor de áudio. Soltar este valor encerra a thread.
pub struct Engine {
    commands: Sender<Command>,
    events: Receiver<Event>,
    shared: Arc<Shared>,
}

impl Engine {
    /// Sobe a thread de áudio. O dispositivo só é aberto na primeira faixa —
    /// abrir o cartão de som no start atrasaria a janela à toa.
    #[must_use]
    pub fn new() -> Self {
        let (command_tx, command_rx) = channel();
        let (event_tx, event_rx) = channel();
        let shared = Arc::new(Shared::default());

        let worker_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                Worker::new(command_rx, event_tx, worker_shared).run();
            })
            .ok();

        Self {
            commands: command_tx,
            events: event_rx,
            shared,
        }
    }

    /// Toca `path` do começo, descartando o que estiver tocando.
    pub fn play(&self, path: PathBuf) {
        self.send(Command::Play(path));
    }

    /// Diz qual faixa vem depois. É isto que faz o gapless: o worker abre a
    /// próxima antes de a atual acabar e continua enchendo o mesmo ring.
    pub fn set_next(&self, path: Option<PathBuf>) {
        self.send(Command::SetNext(path));
    }

    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    pub fn resume(&self) {
        self.send(Command::Resume);
    }

    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    pub fn seek(&self, position: Duration) {
        self.send(Command::Seek(position));
    }

    fn send(&self, command: Command) {
        // Worker morto não é motivo pra derrubar a UI.
        let _ = self.commands.send(command);
    }

    /// Estado atual, montado só de leituras atômicas — pode ser chamado a cada
    /// frame sem custo nem risco de travar a UI.
    #[must_use]
    pub fn state(&self) -> PlaybackState {
        let rate = self.shared.sample_rate.load(Ordering::Relaxed).max(1);
        let frames = self.shared.frames_played.load(Ordering::Relaxed);
        let duration = match self.shared.duration_ms.load(Ordering::Relaxed) {
            0 => None,
            ms => Some(Duration::from_millis(ms)),
        };

        PlaybackState {
            playing: self.shared.intent.load(Ordering::Relaxed),
            position: Duration::from_secs_f64(frames as f64 / f64::from(rate)),
            duration,
        }
    }

    /// Próximo evento, se houver. Não bloqueia.
    pub fn poll_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// O dispositivo pediu áudio e o ring estava vazio desde a última
    /// consulta. Diagnóstico, não estado — a leitura zera a marca.
    pub fn take_starved(&self) -> bool {
        self.shared.starved.swap(false, Ordering::Relaxed)
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// O que fazer quando o arquivo atual acabar de ser decodificado.
enum Tail {
    /// Nada na fila: avisa quando o ring esvaziar.
    Finish,
    /// Próxima faixa em outra taxa de amostragem. Não dá pra emendar sem
    /// reamostrar, então espera o ring esvaziar e reabre o dispositivo.
    Deferred(Box<TrackDecoder>, PathBuf),
}

/// Uma faixa que já está no ring mas ainda não começou a sair pelo alto-falante.
struct Queued {
    start_frame: u64,
    path: PathBuf,
    duration: Option<Duration>,
}

struct Worker {
    commands: Receiver<Command>,
    events: Sender<Event>,
    shared: Arc<Shared>,

    output: Option<Output>,
    decoder: Option<TrackDecoder>,
    converter: Option<Converter>,

    /// Faixa a tocar depois desta.
    next: Option<PathBuf>,
    /// Samples já convertidos, esperando espaço no ring.
    pending: Vec<f32>,
    pending_at: usize,

    tail: Option<Tail>,
    /// Intenção do usuário, separada do portão do callback.
    wants_play: bool,
    /// O ring já tem áudio suficiente para o dispositivo começar a puxar.
    primed: bool,
    /// Faixas emendadas que ainda vão começar a tocar.
    upcoming: VecDeque<Queued>,
    /// Frame em que a faixa atual começou, na contagem contínua do dispositivo.
    track_start: u64,
}

impl Worker {
    fn new(commands: Receiver<Command>, events: Sender<Event>, shared: Arc<Shared>) -> Self {
        Self {
            commands,
            events,
            shared,
            output: None,
            decoder: None,
            converter: None,
            next: None,
            pending: Vec::new(),
            pending_at: 0,
            tail: None,
            wants_play: false,
            primed: false,
            upcoming: VecDeque::new(),
            track_start: 0,
        }
    }

    fn run(mut self) {
        loop {
            let wait = self.pump();
            match self.commands.recv_timeout(wait) {
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => {}
                // O `Engine` foi solto: hora de fechar o dispositivo.
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Play(path) => {
                if let Err(err) = self.start(&path) {
                    self.emit(Event::Error(err.to_string()));
                }
            }
            Command::SetNext(path) => self.next = path,
            Command::Pause => self.wants_play = false,
            Command::Resume => self.wants_play = true,
            Command::Stop => {
                self.wants_play = false;
                self.primed = false;
                self.shared.playing.store(false, Ordering::Relaxed);
                self.shared.intent.store(false, Ordering::Relaxed);
                self.flush();
                self.decoder = None;
                self.converter = None;
                self.tail = None;
                self.upcoming.clear();
                self.pending.clear();
                self.pending_at = 0;
            }
            Command::Seek(position) => {
                if let Some(decoder) = self.decoder.as_mut() {
                    let rate = self.shared.sample_rate.load(Ordering::Relaxed).max(1);
                    if let Err(err) = decoder.seek(position) {
                        self.emit(Event::Error(err.to_string()));
                        return;
                    }
                    self.pending.clear();
                    self.pending_at = 0;
                    self.tail = None;
                    self.upcoming.clear();
                    self.flush();
                    // A contagem de frames é a fonte da posição, então ela
                    // precisa refletir o destino do seek imediatamente.
                    let frames = (position.as_secs_f64() * f64::from(rate)) as u64;
                    self.track_start = 0;
                    self.shared.frames_played.store(frames, Ordering::Relaxed);
                }
            }
        }
    }

    /// Começa uma faixa do zero, reabrindo o dispositivo se o formato pedir.
    fn start(&mut self, path: &Path) -> Result<()> {
        let decoder = TrackDecoder::open(path)?;
        let spec = decoder.spec();

        self.flush();
        let reopen = self
            .output
            .as_ref()
            .is_none_or(|output| output.spec.sample_rate != spec.sample_rate);
        if reopen {
            // Soltar o `Output` fecha o dispositivo antes de reabrir.
            self.output = None;
            self.output = Some(Output::open(spec, &self.shared)?);
        }

        let output_spec = self.output.as_ref().map_or(spec, |output| output.spec);
        self.converter = Some(Converter::new(spec, output_spec));
        self.shared
            .sample_rate
            .store(output_spec.sample_rate, Ordering::Relaxed);
        self.shared.duration_ms.store(
            decoder.duration().map_or(0, |d| d.as_millis() as u64),
            Ordering::Relaxed,
        );
        self.shared.frames_played.store(0, Ordering::Relaxed);

        self.decoder = Some(decoder);
        self.pending.clear();
        self.pending_at = 0;
        self.tail = None;
        self.upcoming.clear();
        self.track_start = 0;
        // Ainda não solta o áudio: o portão abre em `update_gate`, quando o
        // ring tiver enchido o bastante.
        self.wants_play = true;
        self.primed = false;

        self.emit(Event::Started {
            path: path.to_path_buf(),
        });
        Ok(())
    }

    /// Manda o callback jogar fora o que está no ring e espera a confirmação.
    ///
    /// A espera importa: sem ela, o worker voltaria a escrever antes do
    /// descarte e o áudio novo seria jogado fora junto com o velho.
    fn flush(&self) {
        let wanted = self.shared.flush_gen.fetch_add(1, Ordering::Release) + 1;
        // Teto para o caso de o dispositivo não estar rodando — aí a
        // confirmação nunca chegaria.
        let deadline = Instant::now() + Duration::from_millis(200);
        while self.shared.flush_ack.load(Ordering::Acquire) < wanted {
            if Instant::now() > deadline {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Enche o ring e devolve quanto dá pra dormir antes da próxima rodada.
    fn pump(&mut self) -> Duration {
        let mut hit_eof = false;

        if let (Some(output), Some(decoder), Some(converter)) = (
            self.output.as_mut(),
            self.decoder.as_mut(),
            self.converter.as_mut(),
        ) {
            loop {
                let free = output.producer.slots();
                if free == 0 {
                    break;
                }

                if self.pending_at >= self.pending.len() {
                    self.pending.clear();
                    self.pending_at = 0;
                    match decoder.decode_next() {
                        Ok(true) => converter.process(decoder.samples(), &mut self.pending),
                        Ok(false) => {
                            hit_eof = true;
                            break;
                        }
                        Err(err) => {
                            let _ = self.events.send(Event::Error(err.to_string()));
                            hit_eof = true;
                            break;
                        }
                    }
                    if self.pending.is_empty() {
                        continue;
                    }
                }

                let take = free.min(self.pending.len() - self.pending_at);
                let Ok(chunk) = output.producer.write_chunk_uninit(take) else {
                    break;
                };
                chunk.fill_from_iter(
                    self.pending[self.pending_at..self.pending_at + take]
                        .iter()
                        .copied(),
                );
                self.pending_at += take;
            }
        }

        if hit_eof {
            self.on_decoder_end();
        }
        self.check_tail();
        self.check_advance();
        self.update_gate();
        self.sleep_hint()
    }

    /// O arquivo atual acabou de ser decodificado — mas ainda está tocando, o
    /// que sobrou está no ring.
    fn on_decoder_end(&mut self) {
        self.decoder = None;
        self.converter = None;

        let Some(path) = self.next.take() else {
            self.tail = Some(Tail::Finish);
            return;
        };

        let decoder = match TrackDecoder::open(&path) {
            Ok(decoder) => decoder,
            Err(err) => {
                self.emit(Event::Error(err.to_string()));
                self.tail = Some(Tail::Finish);
                return;
            }
        };

        let spec = decoder.spec();
        let output_spec = self.output.as_ref().map(|output| output.spec);

        match output_spec {
            // Mesma taxa: dá pra emendar sem tocar no dispositivo. É o gapless
            // de verdade — a próxima faixa entra no mesmo ring, atrás da cauda
            // da atual.
            Some(output_spec) if output_spec.sample_rate == spec.sample_rate => {
                let frames_in_ring = self.frames_in_ring();
                let played = self.shared.frames_played.load(Ordering::Relaxed);
                self.upcoming.push_back(Queued {
                    start_frame: played + frames_in_ring,
                    path,
                    duration: decoder.duration(),
                });
                self.converter = Some(Converter::new(spec, output_spec));
                self.decoder = Some(decoder);
            }
            // Taxa diferente: emendar exigiria reamostrar a faixa nova, que é
            // exatamente o que se quis evitar. Deixa a atual terminar e reabre
            // o dispositivo na taxa certa.
            _ => self.tail = Some(Tail::Deferred(Box::new(decoder), path)),
        }
    }

    /// Executa o que ficou pendente para quando o ring esvaziar.
    fn check_tail(&mut self) {
        if self.tail.is_none() || self.frames_in_ring() > 0 {
            return;
        }

        match self.tail.take() {
            Some(Tail::Finish) => {
                self.wants_play = false;
                self.emit(Event::Finished);
            }
            Some(Tail::Deferred(decoder, path)) => {
                let spec = decoder.spec();
                self.output = None;
                match Output::open(spec, &self.shared) {
                    Ok(output) => {
                        let output_spec = output.spec;
                        self.output = Some(output);
                        self.converter = Some(Converter::new(spec, output_spec));
                        self.shared
                            .sample_rate
                            .store(output_spec.sample_rate, Ordering::Relaxed);
                        self.shared.duration_ms.store(
                            decoder.duration().map_or(0, |d| d.as_millis() as u64),
                            Ordering::Relaxed,
                        );
                        self.shared.frames_played.store(0, Ordering::Relaxed);
                        self.track_start = 0;
                        self.primed = false;
                        self.decoder = Some(*decoder);
                        self.emit(Event::Advanced { path });
                    }
                    Err(err) => {
                        self.emit(Event::Error(err.to_string()));
                        self.wants_play = false;
                        self.emit(Event::Finished);
                    }
                }
            }
            None => {}
        }
    }

    /// Uma faixa emendada começou a sair pelo alto-falante de verdade.
    fn check_advance(&mut self) {
        let played = self.shared.frames_played.load(Ordering::Relaxed);
        while self
            .upcoming
            .front()
            .is_some_and(|queued| played >= queued.start_frame)
        {
            let Some(queued) = self.upcoming.pop_front() else {
                break;
            };
            self.shared.duration_ms.store(
                queued.duration.map_or(0, |d| d.as_millis() as u64),
                Ordering::Relaxed,
            );
            // A posição é relativa à faixa, não ao dispositivo.
            self.shared
                .frames_played
                .store(played - queued.start_frame, Ordering::Relaxed);
            self.track_start = 0;
            self.emit(Event::Advanced { path: queued.path });
        }
    }

    /// Decide se o callback pode consumir do ring.
    ///
    /// A pré-carga é o que evita o underrun do primeiro callback: o
    /// dispositivo começa a pedir áudio assim que o fluxo abre, e nesse
    /// instante o worker mal teve tempo de decodificar o primeiro bloco.
    fn update_gate(&mut self) {
        if !self.primed
            && let Some(output) = self.output.as_ref()
        {
            let capacity = output.producer.buffer().capacity();
            let filled = capacity - output.producer.slots();
            // Um quarto do ring, ou o arquivo inteiro se for menor que isso.
            self.primed = filled * 4 >= capacity || self.decoder.is_none();
        }

        self.shared
            .playing
            .store(self.wants_play && self.primed, Ordering::Relaxed);
        self.shared.intent.store(self.wants_play, Ordering::Relaxed);
    }

    fn frames_in_ring(&self) -> u64 {
        let Some(output) = self.output.as_ref() else {
            return 0;
        };
        let channels = usize::from(output.spec.channels).max(1);
        let filled = output.producer.buffer().capacity() - output.producer.slots();
        (filled / channels) as u64
    }

    /// Dorme proporcional ao que já está no ring: cheio, o worker pode sumir
    /// por 50 ms; quase vazio, volta em 2 ms. É o que mantém a CPU perto de
    /// zero durante o playback sem arriscar underrun.
    fn sleep_hint(&self) -> Duration {
        let Some(output) = self.output.as_ref() else {
            return IDLE;
        };
        if self.decoder.is_none() && self.tail.is_none() {
            return IDLE;
        }

        let frames = self.frames_in_ring() as f64;
        let seconds = frames / f64::from(output.spec.sample_rate.max(1));
        Duration::from_secs_f64((seconds * 0.4).clamp(0.002, 0.050))
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }
}
