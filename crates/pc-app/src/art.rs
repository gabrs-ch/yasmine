//! Carregamento das capas para a interface.
//!
//! A lista **não** mostra capa — é uma decisão de densidade e de custo: 25
//! linhas visíveis seriam 25 texturas subindo e descendo a cada rolagem, e a
//! direção visual escolhida não pede miniatura por linha. A capa aparece só na
//! barra do player.
//!
//! Mesmo com uma capa por vez, ler e decodificar JPEG na thread da UI daria
//! engasgo ao trocar de faixa. Então o disco fica numa thread própria e a UI
//! só recebe pixels prontos.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use player_core::ArtRef;

type Key = [u8; 32];

/// Teto de texturas na GPU. A UI usa uma por vez; a folga é para trocas
/// rápidas de faixa não recarregarem do disco.
const MAX_TEXTURES: usize = 32;

struct Entry {
    texture: egui::TextureHandle,
    last_used: u64,
}

pub struct ArtLoader {
    requests: Sender<Key>,
    results: Receiver<(Key, egui::ColorImage)>,
    textures: HashMap<Key, Entry>,
    pending: HashSet<Key>,
    frame: u64,
}

impl ArtLoader {
    pub fn new(cache_dir: PathBuf) -> Self {
        let (req_tx, req_rx) = channel::<Key>();
        let (res_tx, res_rx) = channel();

        std::thread::Builder::new()
            .name("capas".into())
            .spawn(move || {
                while let Ok(hash) = req_rx.recv() {
                    // A miniatura de 96px é a que a barra do player usa; a de
                    // 512 fica para uma tela de faixa maior, mais adiante.
                    let path = ArtRef::thumb_path(&cache_dir, &hash, 96);
                    let Ok(image) = image::open(&path) else {
                        continue;
                    };
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    if res_tx.send((hash, color)).is_err() {
                        return;
                    }
                }
            })
            .ok();

        Self {
            requests: req_tx,
            results: res_rx,
            textures: HashMap::new(),
            pending: HashSet::new(),
            frame: 0,
        }
    }

    /// Sobe o que chegou do disco e descarta o que não é usado há mais tempo.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame += 1;

        while let Ok((hash, image)) = self.results.try_recv() {
            self.pending.remove(&hash);
            let texture = ctx.load_texture("capa", image, egui::TextureOptions::LINEAR);
            self.textures.insert(
                hash,
                Entry {
                    texture,
                    last_used: self.frame,
                },
            );
        }

        if self.textures.len() > MAX_TEXTURES {
            let mut ages: Vec<_> = self
                .textures
                .iter()
                .map(|(key, entry)| (entry.last_used, *key))
                .collect();
            ages.sort_unstable();
            for (_, key) in ages.iter().take(self.textures.len() - MAX_TEXTURES) {
                self.textures.remove(key);
            }
        }
    }

    /// Textura da capa, pedindo o carregamento se ainda não estiver pronta.
    /// Devolver `None` é normal: a faixa pode não ter capa, ou ela ainda estar
    /// vindo do disco.
    pub fn texture(&mut self, hash: &Key) -> Option<egui::TextureId> {
        if let Some(entry) = self.textures.get_mut(hash) {
            entry.last_used = self.frame;
            return Some(entry.texture.id());
        }
        if self.pending.insert(*hash) {
            let _ = self.requests.send(*hash);
        }
        None
    }
}
