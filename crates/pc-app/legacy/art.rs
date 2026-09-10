//! Carregamento das capas para a interface.
//!
//! Duas miniaturas por capa (ver `player_core::art`): a de 96px pra
//! miniatura da linha da lista, a de 512px pra capa em destaque (barra do
//! player e modo compacto). Pedir a de 512 pra uma miniatura de 32px seria
//! decodificar 30× mais pixel à toa a cada linha rolada; pedir a de 96 pra
//! capa em destaque de 56px num monitor HiDPI (= 112px reais) deixa ela
//! mole. Cada chamador pede o tamanho que precisa.
//!
//! Ler e decodificar JPEG na thread da UI daria engasgo — ao rolar a lista
//! e ao trocar de faixa. Então o disco fica numa thread própria e a UI só
//! recebe pixels prontos.
//!
//! As texturas sobem com mipmap (`mipmap_mode`): a capa de 512px aparece na
//! barra do player num quadrado de ~56px (112px reais em tela HiDPI), e sem
//! mipmap essa redução de 5× serrilha. Com mipmap o backend escolhe o nível
//! certo e a capa pequena fica lisa. Só o backend glow (o que o app usa)
//! implementa isso hoje — nos outros o campo é ignorado, sem quebrar.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use player_core::ArtRef;

/// Hash da capa + se é a versão grande (512px) ou a pequena (96px). Duas
/// entradas distintas no cache — a mesma capa pode estar carregada nos dois
/// tamanhos ao mesmo tempo (linha da lista e player).
type Key = ([u8; 32], bool);

/// Teto de texturas na GPU. Uma lista rolando rápido enche isso de
/// miniaturas de 96px (leves); a folga é pra rolagem e troca de faixa não
/// recarregarem do disco a toda hora.
const MAX_TEXTURES: usize = 64;

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
                while let Ok(key @ (hash, big)) = req_rx.recv() {
                    let size = if big { 512 } else { 96 };
                    let path = ArtRef::thumb_path(&cache_dir, &hash, size);
                    let Ok(image) = image::open(&path) else {
                        continue;
                    };
                    let rgba = image.to_rgba8();
                    let dims = [rgba.width() as usize, rgba.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(dims, rgba.as_raw());
                    if res_tx.send((key, color)).is_err() {
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

        while let Ok((key, image)) = self.results.try_recv() {
            self.pending.remove(&key);
            let texture = ctx.load_texture(
                "capa",
                image,
                egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)),
            );
            self.textures.insert(
                key,
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

    /// Textura da capa no tamanho pedido (`big` = a de 512px, senão a de
    /// 96px), pedindo o carregamento se ainda não estiver pronta. Devolver
    /// `None` é normal: a faixa pode não ter capa, ou ela ainda estar vindo
    /// do disco.
    pub fn texture(&mut self, hash: &[u8; 32], big: bool) -> Option<egui::TextureId> {
        let key = (*hash, big);
        if let Some(entry) = self.textures.get_mut(&key) {
            entry.last_used = self.frame;
            return Some(entry.texture.id());
        }
        if self.pending.insert(key) {
            let _ = self.requests.send(key);
        }
        None
    }
}
