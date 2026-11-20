//! Le mode de test : `emuland.exe --test <rom> [options]`.
//!
//! L'application s'ouvre hors de l'écran, sans prendre le focus ni la souris, lance la ROM directement,
//! « appuie » sur des boutons selon un script et se photographie elle-même (fichier .bmp), puis se ferme.
//! Ça permet de vérifier l'interface et un jeu sans toucher au clavier ni à la souris de l'utilisateur.
//!
//! Options :
//!   --shot <secondes>:<fichier.bmp>   prend une capture à ce moment (répétable)
//!   --keys <bouton>@<secondes>[+<durée>],...   par exemple `start@1.0,a@2.5+0.3`
//!             (boutons : up down left right a b start select ; durée par défaut 0,15 s)
//!   --fullscreen                       lance directement en plein écran
//!   --quit <secondes>                  ferme au plus tard à ce moment (par défaut : 5 s après la dernière capture)

use eframe::egui;
use gb_core::joypad;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct TestRun {
    /// La ROM à lancer ; `None` avec `--test-menu` (on reste dans le menu, avec la ROM de démonstration importée).
    pub rom: Option<PathBuf>,
    pub menu: bool,
    /// Une deuxième ROM, branchée sur un câble link (`--link <rom>`).
    pub link: Option<PathBuf>,
    pub fullscreen: bool,
    /// Boutons du joueur 2 (`--keys2`, même syntaxe que `--keys`).
    keys2: Vec<(f64, f64, u8)>,
    shots: Vec<(f64, PathBuf)>,
    keys: Vec<(f64, f64, u8)>,
    /// Actions de l'application (sauver, charger, plein écran) à un instant donné.
    actions: Vec<(f64, String)>,
    fired: usize,
    quit_at: f64,
    next_shot: usize,
    /// Une capture a été demandée et son fichier n'est pas encore écrit.
    waiting: bool,
    /// Nombre de captures déjà écrites sur le disque.
    done: Arc<AtomicUsize>,
}

fn button(name: &str) -> Option<u8> {
    Some(match name {
        "up" => joypad::UP,
        "down" => joypad::DOWN,
        "left" => joypad::LEFT,
        "right" => joypad::RIGHT,
        "a" => joypad::A,
        "b" => joypad::B,
        "start" => joypad::START,
        "select" => joypad::SELECT,
        _ => return None,
    })
}

impl TestRun {
    /// Lit la ligne de commande ; `None` si le mode de test n'est pas demandé.
    pub fn from_args() -> Option<Self> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let rom = args.iter().position(|a| a == "--test").and_then(|i| args.get(i + 1)).map(PathBuf::from);
        let menu = args.iter().any(|a| a == "--test-menu");
        if rom.is_none() && !menu {
            return None;
        }
        let mut shots = Vec::new();
        let mut keys = Vec::new();
        let mut keys2 = Vec::new();
        let link = args.iter().position(|a| a == "--link").and_then(|i| args.get(i + 1)).map(PathBuf::from);
        let mut actions: Vec<(f64, String)> = Vec::new();
        let (mut fullscreen, mut quit) = (false, None);
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--shot" => {
                    if let Some((t, path)) = args.get(i + 1).and_then(|v| v.split_once(':')) {
                        if let Ok(t) = t.parse::<f64>() {
                            shots.push((t, PathBuf::from(path)));
                        }
                    }
                    i += 1;
                }
                "--keys" | "--keys2" => {
                    let target = if args[i] == "--keys2" { &mut keys2 } else { &mut keys };
                    for item in args.get(i + 1).map(String::as_str).unwrap_or("").split(',') {
                        let Some((name, when)) = item.split_once('@') else { continue };
                        let (start, dur) = when.split_once('+').unwrap_or((when, "0.15"));
                        if let (Some(mask), Ok(start), Ok(dur)) = (button(name.trim()), start.parse::<f64>(), dur.parse::<f64>()) {
                            target.push((start, dur, mask));
                        }
                    }
                    i += 1;
                }
                "--act" => {
                    // actions de l'application : save, load, fullscreen (par exemple `save@8,load@12`)
                    for item in args.get(i + 1).map(String::as_str).unwrap_or("").split(',') {
                        if let Some((name, when)) = item.split_once('@') {
                            if let (true, Ok(t)) = (matches!(name, "save" | "load" | "fullscreen" | "controls"), when.parse::<f64>()) {
                                actions.push((t, name.to_string()));
                            }
                        }
                    }
                    i += 1;
                }
                "--fullscreen" => fullscreen = true,
                "--quit" => {
                    quit = args.get(i + 1).and_then(|v| v.parse::<f64>().ok());
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        shots.sort_by(|a, b| a.0.total_cmp(&b.0));
        actions.sort_by(|a, b| a.0.total_cmp(&b.0));
        let last = shots.last().map(|s| s.0).unwrap_or(3.0);
        Some(Self { rom, menu, link, fullscreen, keys2, shots, keys, actions, fired: 0, quit_at: quit.unwrap_or(last + 5.0), next_shot: 0, waiting: false, done: Arc::new(AtomicUsize::new(0)) })
    }

    /// Les boutons que le script tient enfoncés à l'instant `t` (en secondes depuis le lancement).
    pub fn mask_at(&self, t: f64) -> u8 {
        self.keys.iter().filter(|(start, dur, _)| t >= *start && t < start + dur).fold(0, |m, (_, _, b)| m | b)
    }

    /// Les boutons que le script tient enfoncés pour le joueur 2 à l'instant `t`.
    pub fn mask2_at(&self, t: f64) -> u8 {
        self.keys2.iter().filter(|(start, dur, _)| t >= *start && t < start + dur).fold(0, |m, (_, _, b)| m | b)
    }

    /// Les actions de l'application dont l'heure est venue (une seule fois chacune).
    pub fn due_actions(&mut self, t: f64) -> Vec<String> {
        let mut due = Vec::new();
        while let Some((when, name)) = self.actions.get(self.fired) {
            if t < *when {
                break;
            }
            due.push(name.clone());
            self.fired += 1;
        }
        due
    }

    /// À appeler à chaque image : demande la capture suivante quand son heure est venue (l'une après l'autre).
    pub fn poll(&mut self, ctx: &egui::Context, t: f64) {
        if self.waiting && self.done.load(Ordering::SeqCst) >= self.next_shot {
            self.waiting = false;
        }
        if self.waiting {
            return;
        }
        if let Some((when, path)) = self.shots.get(self.next_shot) {
            if t >= *when {
                let (path, done) = (path.clone(), self.done.clone());
                self.next_shot += 1;
                self.waiting = true;
                ctx.request_screenshot(move |image| {
                    if let Err(e) = write_bmp(&path, &image) {
                        eprintln!("capture impossible : {e}");
                    }
                    done.fetch_add(1, Ordering::SeqCst);
                });
            }
        }
    }

    /// Faut-il fermer l'application ? (toutes les captures sont écrites, ou le temps maximal est écoulé)
    pub fn finished(&self, t: f64) -> bool {
        (!self.shots.is_empty() && self.done.load(Ordering::SeqCst) >= self.shots.len()) || t >= self.quit_at
    }
}

/// Écrit une image en BMP 24 bits (aucune bibliothèque nécessaire).
fn write_bmp(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    let row = (w * 3 + 3) & !3;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        let start = out.len();
        for x in 0..w {
            let c = image.pixels[y * w + x];
            out.extend_from_slice(&[c.b(), c.g(), c.r()]);
        }
        out.resize(start + row, 0);
    }
    std::fs::write(path, out)
}
