//! Janela egui: o QR grande e um log de atividade. Só um verniz sobre a mesma
//! [`HostSession`] da CLI.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use yasmine_sync::ServerEvent;

use crate::host::{HostSession, Options};

pub fn run(opts: Options) -> Result<(), String> {
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = log.clone();
    let session = HostSession::start(&opts, move |ev| {
        if let Ok(mut l) = sink.lock() {
            l.push(format_event(&ev));
            if l.len() > 300 {
                l.remove(0);
            }
        }
    })?;

    let qr = QrPixels::build(&session.pair_url);
    let app = HostApp {
        name: opts.name,
        addr: session.server.addr().to_string(),
        pair_url: session.pair_url.clone(),
        _session: session,
        log,
        qr,
        qr_tex: None,
    };

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([540.0, 680.0])
            .with_min_inner_size([420.0, 520.0])
            .with_title("Yasmine · sync host"),
        ..Default::default()
    };
    eframe::run_native(
        "yasmine-sync-host",
        native,
        Box::new(|_cc| Ok(Box::new(app))),
    )
    .map_err(|e| e.to_string())
}

struct HostApp {
    name: String,
    addr: String,
    pair_url: String,
    _session: HostSession,
    log: Arc<Mutex<Vec<String>>>,
    qr: QrPixels,
    qr_tex: Option<egui::TextureHandle>,
}

impl eframe::App for HostApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let side = self.qr.side;
        let tex = self.qr_tex.get_or_insert_with(|| {
            let img = egui::ColorImage::from_rgba_unmultiplied([side, side], &self.qr.rgba);
            ui.ctx()
                .load_texture("qr", img, egui::TextureOptions::NEAREST)
        });
        let tex_id = tex.id();

        ui.heading(format!("Yasmine · {}", self.name));
        ui.label(format!("escutando em {}", self.addr));
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.image(egui::load::SizedTexture::new(
                tex_id,
                egui::vec2(side as f32, side as f32),
            ));
        });
        ui.add_space(6.0);
        ui.label("No celular: aba Parear → escaneie.");
        ui.horizontal_wrapped(|ui| ui.monospace(&self.pair_url));
        ui.separator();
        ui.label("Atividade");
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Ok(l) = self.log.lock() {
                    for line in l.iter().rev() {
                        ui.monospace(line);
                    }
                }
            });

        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
}

fn format_event(ev: &ServerEvent) -> String {
    match ev {
        ServerEvent::Listening(a) => format!("escutando {a}"),
        ServerEvent::PeerConnected { name, .. } => format!("{name} conectou"),
        ServerEvent::Sending { peer, done, total } => {
            let pct = if *total > 0 { done * 100 / total } else { 100 };
            format!("enviando pra {peer}: {pct}% ({done}/{total} bytes)")
        }
        ServerEvent::PeerFinished { .. } => "transferência concluída".to_string(),
        ServerEvent::ConnectionError(e) => format!("erro: {e}"),
    }
}

/// QR rasterizado em RGBA, com zona de silêncio e escala inteira.
struct QrPixels {
    side: usize,
    rgba: Vec<u8>,
}

impl QrPixels {
    fn build(data: &str) -> Self {
        use qrcode::{Color, QrCode};
        let code = match QrCode::new(data.as_bytes()) {
            Ok(c) => c,
            Err(_) => {
                return Self {
                    side: 1,
                    rgba: vec![0, 0, 0, 255],
                };
            }
        };
        let w = code.width();
        let quiet = 4usize;
        let modules = w + quiet * 2;
        let scale = (460 / modules).max(1);
        let side = modules * scale;
        let colors = code.to_colors();

        let mut rgba = vec![255u8; side * side * 4];
        for my in 0..modules {
            for mx in 0..modules {
                let dark = mx >= quiet
                    && my >= quiet
                    && mx < quiet + w
                    && my < quiet + w
                    && colors[(my - quiet) * w + (mx - quiet)] == Color::Dark;
                if !dark {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        let i = ((my * scale + sy) * side + (mx * scale + sx)) * 4;
                        rgba[i] = 0;
                        rgba[i + 1] = 0;
                        rgba[i + 2] = 0;
                        rgba[i + 3] = 255;
                    }
                }
            }
        }
        Self { side, rgba }
    }
}
