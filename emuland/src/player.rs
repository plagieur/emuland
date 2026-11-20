//! Le lecteur : fait tourner la console, affiche son écran et montre son état.
//! Ce que la ROM envoie sur la liaison série s'affiche dans la console à droite
//! (c'est ainsi que les ROMs de test donnent leur verdict).

use crate::audio::Sink;
use crate::controls::Controls;
use crate::theme::*;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Margin, RichText, Stroke, Vec2};
use gb_core::cpu::{FLAG_C, FLAG_H, FLAG_N, FLAG_Z};
use gb_core::link;
use gb_core::{GameBoy, SCREEN_HEIGHT, SCREEN_WIDTH};
use std::time::Instant;

const FRAMES_PER_SECOND: f64 = 59.7275;

/// L'heure réelle, en secondes depuis 1970.
fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Convertit l'image d'une console (nuances ou couleurs 15 bits) en texture pour l'écran.
fn upload(gb: &GameBoy, screen: &mut Option<egui::TextureHandle>, ctx: &egui::Context, name: &str) {
    let mut rgba = Vec::with_capacity(SCREEN_WIDTH * SCREEN_HEIGHT * 4);
    if gb.is_cgb() {
        // couleurs sur 15 bits (5 bits par couleur) : on étale chaque couleur sur 8 bits
        let expand = |c: u16| ((c << 3) | (c >> 2)) as u8;
        for &c in gb.frame_color() {
            rgba.extend_from_slice(&[expand(c & 31), expand((c >> 5) & 31), expand((c >> 10) & 31), 255]);
        }
    } else {
        for &shade in gb.frame() {
            rgba.extend_from_slice(&SHADES[(shade & 3) as usize]);
        }
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([SCREEN_WIDTH, SCREEN_HEIGHT], &rgba);
    match screen {
        Some(tex) => tex.set(image, egui::TextureOptions::NEAREST),
        None => *screen = Some(ctx.load_texture(name, image, egui::TextureOptions::NEAREST)),
    }
}

/// Les 4 nuances de l'écran, du plus clair au plus foncé.
const SHADES: [[u8; 4]; 4] = [[232, 232, 232, 255], [160, 160, 160, 255], [80, 80, 80, 255], [16, 16, 16, 255]];

/// Ce que l'utilisateur demande en dehors du jeu.
pub enum PlayerAction {
    None,
    Back,
    OpenControls,
    ToggleFullscreen,
    SaveState,
    LoadState,
    /// Brancher un deuxième joueur : l'identifiant du jeu de la bibliothèque à lancer sur l'autre console.
    Link(String),
    Unlink,
}

/// Nombre d'emplacements d'états sauvegardés par jeu.
pub const STATE_SLOTS: u8 = 4;

/// La deuxième console d'une liaison (câble link), avec son écran et sa sauvegarde.
struct Peer {
    gb: GameBoy,
    id: String,
    title: String,
    screen: Option<egui::TextureHandle>,
    saved: Vec<u8>,
    scripted: u8,
}

pub struct Player {
    pub id: String,
    peer: Option<Peer>,
    /// Boutons « appuyés » par un script de test (mode `--test`), en plus du clavier et de la souris.
    scripted: u8,
    sink: Option<Sink>,
    screen_rect: Option<egui::Rect>,
    gb: GameBoy,
    title: String,
    supported: bool,
    running: bool,
    acc: f64,
    last: Instant,
    frames: u64,
    screen: Option<egui::TextureHandle>,
    save_check: Instant,
    saved: Vec<u8>,
}

impl Drop for Player {
    /// En quittant le jeu, le son qui restait en attente ne doit pas continuer à jouer.
    fn drop(&mut self) {
        if let Some(s) = &self.sink {
            s.clear();
        }
    }
}

impl Player {
    pub fn new(id: String, title: String, rom: Vec<u8>, supported: bool, save: Option<Vec<u8>>, sink: Option<Sink>) -> Result<Self, String> {
        let mut gb = GameBoy::new(rom)?;
        if let Some(s) = &sink {
            gb.set_sample_rate(s.sample_rate);
            s.clear();
        }
        if let Some(data) = save {
            gb.load_save(&data, now_unix()); // l'horloge d'une cartouche qui en a une avance du temps réel écoulé
        }
        let saved = gb.save_ram().map(|r| r.to_vec()).unwrap_or_default();
        Ok(Self { id, peer: None, scripted: 0, sink, screen_rect: None, gb, title, supported, running: true, acc: 0.0, last: Instant::now(), frames: 0, screen: None, save_check: Instant::now(), saved })
    }

    /// La sauvegarde à écrire sur le disque, si la RAM de la cartouche a changé depuis la dernière fois.
    /// Sans `force`, ne regarde que toutes les 3 secondes. Avec `force` (on quitte), une cartouche à horloge
    /// est toujours enregistrée : l'heure de la console doit repartir de la bonne valeur.
    pub fn pending_save(&mut self, force: bool) -> Option<Vec<u8>> {
        if !force && self.save_check.elapsed().as_secs() < 3 {
            return None;
        }
        self.save_check = Instant::now();
        let ram = self.gb.save_ram()?;
        let changed = ram != self.saved.as_slice();
        if !changed && !(force && self.gb.has_clock()) {
            return None;
        }
        self.saved = ram.to_vec();
        self.gb.save_data(now_unix())
    }

    /// Boutons à tenir enfoncés de la part d'un script de test (voir `--test`), pour le joueur 1 puis le joueur 2.
    pub fn set_scripted(&mut self, mask: u8, mask2: u8) {
        self.scripted = mask;
        if let Some(peer) = &mut self.peer {
            peer.scripted = mask2;
        }
    }

    // ---------- liaison (câble link) ----------

    pub fn is_linked(&self) -> bool {
        self.peer.is_some()
    }

    /// Branche une deuxième console, avec le jeu `rom`, sur le câble : les deux consoles échangent leurs octets.
    pub fn link(&mut self, id: String, title: String, rom: Vec<u8>, save: Option<Vec<u8>>) -> Result<(), String> {
        let mut gb = GameBoy::new(rom)?;
        if let Some(data) = save {
            gb.load_save(&data, now_unix());
        }
        let saved = gb.save_ram().map(|r| r.to_vec()).unwrap_or_default();
        gb.set_link_connected(true);
        self.gb.set_link_connected(true);
        self.peer = Some(Peer { gb, id, title, screen: None, saved, scripted: 0 });
        Ok(())
    }

    pub fn unlink(&mut self) {
        self.peer = None;
        self.gb.set_link_connected(false);
    }

    /// L'identifiant de sauvegarde de la deuxième console : son jeu, suffixé pour ne pas écraser celle du joueur 1.
    pub fn peer_save_id(&self) -> Option<String> {
        self.peer.as_ref().map(|p| format!("{}.p2", p.id))
    }

    /// La sauvegarde du joueur 2 à écrire sur le disque, si sa RAM a changé (ou à la sortie, pour une cartouche à horloge).
    pub fn pending_peer_save(&mut self, force: bool) -> Option<Vec<u8>> {
        let peer = self.peer.as_mut()?;
        let ram = peer.gb.save_ram()?;
        let changed = ram != peer.saved.as_slice();
        if !changed && !(force && peer.gb.has_clock()) {
            return None;
        }
        peer.saved = ram.to_vec();
        peer.gb.save_data(now_unix())
    }

    /// Une photo complète de la console (voir `GameBoy::save_state`).
    pub fn snapshot(&self) -> Vec<u8> {
        self.gb.save_state()
    }

    /// Recharge une photo : le jeu reprend exactement où il en était. Rien ne change si elle est refusée.
    pub fn restore(&mut self, data: &[u8]) -> Result<(), String> {
        self.gb.load_state(data)?;
        self.gb.set_link_connected(self.peer.is_some());
        if let Some(s) = &self.sink {
            s.clear();
        }
        self.last = Instant::now();
        self.acc = 0.0;
        self.running = true;
        Ok(())
    }

    /// Envoie la dernière image de chaque console vers la carte graphique.
    fn upload_frame(&mut self, ctx: &egui::Context) {
        upload(&self.gb, &mut self.screen, ctx, "lcd");
        if let Some(peer) = &mut self.peer {
            upload(&peer.gb, &mut peer.screen, ctx, "lcd2");
        }
    }

    /// Fait avancer la console selon le temps réellement écoulé (jamais plus de 4 images d'un coup).
    fn advance(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.1);
        self.last = now;
        if !self.running || self.gb.cpu.locked {
            return;
        }
        self.acc += dt * FRAMES_PER_SECOND;
        let mut n = 0;
        while self.acc >= 1.0 && n < 4 {
            match &mut self.peer {
                // deux consoles reliées : elles avancent ensemble, image par image
                Some(peer) => link::run_frame_pair(&mut self.gb, &mut peer.gb),
                None => self.gb.run_frame(),
            }
            self.frames += 1;
            self.acc -= 1.0;
            n += 1;
        }
        if self.acc > 4.0 {
            self.acc = 0.0;
        }
        // le son produit pendant ces images part vers la carte son (ou est jeté s'il n'y en a pas)
        let samples = self.gb.take_audio();
        if let Some(sink) = &self.sink {
            sink.push(&samples); // seul le son du joueur 1 est joué
        }
        if let Some(peer) = &mut self.peer {
            let _ = peer.gb.take_audio();
        }
    }

    /// Dessine le lecteur. `frozen` : le jeu s'arrête et ignore le clavier (fenêtre des contrôles ouverte).
    /// `controls2` : les touches du joueur 2 ; `choices` : les jeux de la bibliothèque qu'on peut brancher sur le câble.
    pub fn show(&mut self, root: &mut egui::Ui, controls: &Controls, controls2: &Controls, choices: &[(String, String)], frozen: bool, fullscreen: bool, slot: &mut u8) -> PlayerAction {
        let ctx = root.ctx().clone();
        if frozen {
            self.last = Instant::now(); // pas de rattrapage du temps perdu en reprenant
            self.gb.set_buttons(0);
            if let Some(peer) = &mut self.peer {
                peer.gb.set_buttons(0);
            }
        } else {
            let pointer = ctx.input(|i| i.pointer.latest_pos());
            let on_screen = match (self.screen_rect, pointer) {
                (Some(rect), Some(pos)) => rect.contains(pos),
                _ => false,
            };
            // en liaison, une touche ne fait jouer qu'un joueur : le joueur 1 ignore les touches du joueur 2
            let p2_keys = if self.peer.is_some() { controls2.keys() } else { Vec::new() };
            self.gb.set_buttons(controls.state(&ctx, on_screen, &p2_keys) | self.scripted);
            if let Some(peer) = &mut self.peer {
                peer.gb.set_buttons(controls2.state(&ctx, false, &[]) | peer.scripted);
            }
            // jeux à accéléromètre (MBC7) : la position de la souris sur l'écran incline la console
            if self.gb.uses_tilt() {
                let (x, y) = match (self.screen_rect, pointer) {
                    (Some(rect), Some(pos)) if on_screen => (((pos.x - rect.center().x) / (rect.width() / 2.0)).clamp(-1.0, 1.0), ((pos.y - rect.center().y) / (rect.height() / 2.0)).clamp(-1.0, 1.0)),
                    _ => (0.0, 0.0),
                };
                self.gb.set_tilt(x, y);
            }
            self.advance();
        }
        self.upload_frame(&ctx);
        ctx.request_repaint();
        let mut action = PlayerAction::None;

        if !fullscreen {
        egui::Panel::top("player-top").frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 12)).stroke(Stroke::new(1.0, LINE))).show(root, |ui| {
            ui.horizontal(|ui| {
                if ui.button(crate::tr!("Bibliothèque", "Library")).clicked() {
                    action = PlayerAction::Back;
                }
                ui.add_space(6.0);
                ui.label(RichText::new(&self.title).font(FontId::new(17.0, bold())));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(crate::tr!("Redémarrer", "Restart")).clicked() {
                        if let Some(s) = &self.sink {
                            s.clear();
                        }
                        self.gb.reset();
                        self.gb.set_link_connected(self.peer.is_some());
                        if let Some(peer) = &mut self.peer {
                            peer.gb.reset();
                            peer.gb.set_link_connected(true);
                        }
                        self.frames = 0;
                        self.running = true;
                    }
                    crate::i18n::menu(ui);
                    if ui.button(if self.running { "Pause" } else { crate::tr!("Reprendre", "Resume") }).clicked() {
                        self.running = !self.running;
                    }
                    if ui.button(crate::tr!("Contrôles", "Controls")).clicked() {
                        action = PlayerAction::OpenControls;
                    }
                    if ui.button(crate::tr!("Plein écran", "Full screen")).clicked() {
                        action = PlayerAction::ToggleFullscreen;
                    }
                    ui.menu_button(if self.peer.is_some() { crate::tr!("Liaison : branchée", "Link: connected") } else { crate::tr!("Liaison", "Link") }, |ui| {
                        if self.peer.is_some() {
                            if ui.button(crate::tr!("Débrancher le câble", "Unplug the cable")).clicked() {
                                action = PlayerAction::Unlink;
                                ui.close();
                            }
                        } else {
                            ui.label(RichText::new(crate::tr!("Joue sur une 2e console, reliée par un câble link :", "Play on a 2nd console, connected by a link cable:")).size(13.0).color(MUTED));
                            for (id, title) in choices {
                                if ui.button(title).clicked() {
                                    action = PlayerAction::Link(id.clone());
                                    ui.close();
                                }
                            }
                            if choices.is_empty() {
                                ui.label(RichText::new(crate::tr!("(aucun jeu dans la bibliothèque)", "(no game in the library)")).size(13.0).color(DIM));
                            }
                            ui.label(RichText::new(crate::tr!("Joueur 2 : flèches, M (A), N (B), J (Start), H (Select) ; réglable dans Contrôles", "Player 2: arrow keys, M (A), N (B), J (Start), H (Select); configurable in Controls")).size(12.0).color(MUTED));
                        }
                    });
                    ui.menu_button(crate::tr!("États", "States"), |ui| {
                        for s in 1..=STATE_SLOTS {
                            ui.horizontal(|ui| {
                                ui.label(crate::tr!(format!("Emplacement {s}"), format!("Slot {s}")));
                                if ui.button(crate::tr!("Sauver", "Save")).clicked() {
                                    *slot = s;
                                    action = PlayerAction::SaveState;
                                    ui.close();
                                }
                                if ui.button(crate::tr!("Charger", "Load")).clicked() {
                                    *slot = s;
                                    action = PlayerAction::LoadState;
                                    ui.close();
                                }
                            });
                        }
                        ui.label(RichText::new(crate::tr!("F5 sauver · F8 charger (dernier emplacement utilisé)", "F5 save · F8 load (last slot used)")).size(12.0).color(MUTED));
                    });
                    if let Some(sink) = &self.sink {
                        let muted = sink.is_muted();
                        if ui.button(if muted { crate::tr!("Son coupé", "Sound off") } else { crate::tr!("Son activé", "Sound on") }).clicked() {
                            sink.set_muted(!muted);
                        }
                    } else {
                        ui.add_enabled(false, egui::Button::new(crate::tr!("Pas de sortie son", "No audio output")));
                    }
                });
            });
        });

        egui::Panel::right("player-side").exact_size(300.0).frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(20)).stroke(Stroke::new(1.0, LINE))).show(root, |ui| {
            self.cpu_panel(ui);
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(6.0);
            self.serial_panel(ui);
        });
        }

        let margin = if fullscreen { 0 } else { 24 };
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(margin))).show(root, |ui| {
            let area = ui.max_rect();
            let linked = self.peer.is_some();
            // en liaison, l'espace est partagé en deux : un écran par console
            let halves = if linked { 2.0 } else { 1.0 };
            let label_room = if linked { 26.0 } else { 0.0 };
            let scale = ((area.width() / halves - if linked { 16.0 } else { 0.0 }) / 160.0).min((area.height() - label_room) / 144.0).floor().max(1.0);
            let size = Vec2::new(160.0, 144.0) * scale;
            let centers: Vec<egui::Pos2> = (0..halves as usize).map(|i| egui::pos2(area.left() + area.width() * (i as f32 + 0.5) / halves, area.center().y + label_room / 2.0)).collect();
            let rect = egui::Rect::from_center_size(centers[0], size);
            self.screen_rect = Some(rect);
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            let p = ui.painter();
            if !fullscreen {
                p.rect_stroke(rect.expand(1.0), CornerRadius::ZERO, Stroke::new(1.0, LINE), egui::StrokeKind::Outside);
            }
            if let Some(tex) = &self.screen {
                p.image(tex.id(), rect, uv, Color32::WHITE);
            }
            if linked {
                p.text(egui::pos2(rect.center().x, rect.top() - 14.0), Align2::CENTER_CENTER, format!("{} 1 · {}", crate::tr!("Joueur", "Player"), self.title), FontId::proportional(13.0), MUTED);
            }
            if let Some(peer) = &self.peer {
                let rect2 = egui::Rect::from_center_size(centers[1], size);
                if !fullscreen {
                    p.rect_stroke(rect2.expand(1.0), CornerRadius::ZERO, Stroke::new(1.0, LINE), egui::StrokeKind::Outside);
                }
                if let Some(tex) = &peer.screen {
                    p.image(tex.id(), rect2, uv, Color32::WHITE);
                }
                p.text(egui::pos2(rect2.center().x, rect2.top() - 14.0), Align2::CENTER_CENTER, format!("{} 2 · {}", crate::tr!("Joueur", "Player"), peer.title), FontId::proportional(13.0), MUTED);
            }
            if !self.running {
                p.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0, 0, 0, 150));
                p.text(rect.center(), Align2::CENTER_CENTER, "Pause", FontId::new(26.0, bold()), INK);
            }
        });
        action
    }

    fn cpu_panel(&mut self, ui: &mut egui::Ui) {
        let cpu = &self.gb.cpu;
        ui.label(RichText::new(crate::tr!("Processeur", "Processor")).font(FontId::new(15.0, bold())));
        let (state, color) = if cpu.locked {
            (crate::tr!("Bloqué (instruction illégale)", "Locked (illegal instruction)"), BAD)
        } else if !self.running {
            (crate::tr!("En pause", "Paused"), WARN)
        } else if cpu.halted {
            (crate::tr!("En attente (HALT)", "Waiting (HALT)"), MUTED)
        } else {
            (crate::tr!("En marche", "Running"), OK)
        };
        ui.label(RichText::new(state).size(13.0).color(color));
        if !self.supported {
            ui.label(RichText::new(crate::tr!("Cartouche pas encore gérée : la ROM peut mal fonctionner.", "Cartridge not supported yet: the ROM may not work properly.")).size(12.0).color(WARN));
        }
        ui.add_space(4.0);
        let mono = |s: String| RichText::new(s).monospace().size(14.0).color(INK);
        egui::Grid::new("regs").num_columns(2).spacing([22.0, 4.0]).show(ui, |ui| {
            ui.label(mono(format!("AF {:04X}", cpu.af())));
            ui.label(mono(format!("BC {:04X}", cpu.bc())));
            ui.end_row();
            ui.label(mono(format!("DE {:04X}", cpu.de())));
            ui.label(mono(format!("HL {:04X}", cpu.hl())));
            ui.end_row();
            ui.label(mono(format!("SP {:04X}", cpu.sp)));
            ui.label(mono(format!("PC {:04X}", cpu.pc)));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            for (name, mask) in [("Z", FLAG_Z), ("N", FLAG_N), ("H", FLAG_H), ("C", FLAG_C)] {
                ui.label(RichText::new(name).monospace().size(14.0).color(if cpu.f & mask != 0 { INK } else { DIM }));
            }
            ui.label(RichText::new("IME").monospace().size(14.0).color(if cpu.ime { INK } else { DIM }));
        });
        ui.label(RichText::new(format!("{} {} · {} cycles", self.frames, crate::tr!("images", "frames"), self.gb.total_cycles)).size(12.0).color(MUTED));
    }

    fn serial_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(crate::tr!("Liaison série", "Serial link")).font(FontId::new(15.0, bold())));
        let text = self.gb.serial_text();
        let shown: String = {
            let chars: Vec<char> = text.chars().collect();
            chars[chars.len().saturating_sub(6000)..].iter().collect()
        };
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if shown.is_empty() {
                ui.label(RichText::new(crate::tr!("(vide)", "(empty)")).monospace().size(13.0).color(DIM));
            } else {
                ui.label(RichText::new(shown).monospace().size(13.0).color(INK));
            }
        });
    }
}
