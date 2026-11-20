//! Les contrôles : quelle touche du clavier ou quel bouton de la souris correspond à quel bouton de la Game Boy.
//! Les réglages sont enregistrés dans `%APPDATA%\emuland\controls.json`.

use crate::theme::*;
use eframe::egui::{self, Color32, Key, PointerButton, RichText, Stroke};
use gb_core::joypad as pad;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    Start,
    Select,
}

pub const ACTIONS: [Action; 8] = [Action::Up, Action::Down, Action::Left, Action::Right, Action::A, Action::B, Action::Start, Action::Select];

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Up => crate::tr!("Haut", "Up"),
            Action::Down => crate::tr!("Bas", "Down"),
            Action::Left => crate::tr!("Gauche", "Left"),
            Action::Right => crate::tr!("Droite", "Right"),
            Action::A => crate::tr!("Bouton A", "A button"),
            Action::B => crate::tr!("Bouton B", "B button"),
            Action::Start => "Start",
            Action::Select => "Select",
        }
    }

    fn mask(self) -> u8 {
        match self {
            Action::Up => pad::UP,
            Action::Down => pad::DOWN,
            Action::Left => pad::LEFT,
            Action::Right => pad::RIGHT,
            Action::A => pad::A,
            Action::B => pad::B,
            Action::Start => pad::START,
            Action::Select => pad::SELECT,
        }
    }
}

/// Les trois emplacements d'une action : deux touches et un bouton de souris.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Key1,
    Key2,
    Mouse,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Bind {
    pub keys: [Option<Key>; 2],
    pub mouse: Option<PointerButton>,
}

pub struct Controls {
    pub binds: [Bind; 8],
    path: PathBuf,
    player: u8,
}

/// Forme écrite dans le fichier : des textes, plus faciles à relire et à modifier à la main.
#[derive(Serialize, Deserialize)]
struct SavedBind {
    action: String,
    key1: Option<String>,
    key2: Option<String>,
    mouse: Option<String>,
}

fn mouse_to_text(b: PointerButton) -> &'static str {
    match b {
        PointerButton::Primary => "left",
        PointerButton::Secondary => "right",
        PointerButton::Middle => "middle",
        PointerButton::Extra1 => "extra1",
        PointerButton::Extra2 => "extra2",
    }
}

fn mouse_from_text(s: &str) -> Option<PointerButton> {
    Some(match s {
        "left" => PointerButton::Primary,
        "right" => PointerButton::Secondary,
        "middle" => PointerButton::Middle,
        "extra1" => PointerButton::Extra1,
        "extra2" => PointerButton::Extra2,
        _ => return None,
    })
}

pub fn key_label(k: Key) -> String {
    match k.name() {
        "Up" | "ArrowUp" => "↑".into(),
        "Down" | "ArrowDown" => "↓".into(),
        "Left" | "ArrowLeft" => "←".into(),
        "Right" | "ArrowRight" => "→".into(),
        "Enter" => crate::tr!("Entrée", "Enter").into(),
        "Backspace" => crate::tr!("Retour arrière", "Backspace").into(),
        "Space" => crate::tr!("Espace", "Space").into(),
        "Escape" => crate::tr!("Échap", "Esc").into(),
        other => other.into(),
    }
}

pub fn mouse_label(b: PointerButton) -> &'static str {
    match b {
        PointerButton::Primary => crate::tr!("Clic gauche", "Left click"),
        PointerButton::Secondary => crate::tr!("Clic droit", "Right click"),
        PointerButton::Middle => crate::tr!("Clic molette", "Middle click"),
        PointerButton::Extra1 => crate::tr!("Bouton 4", "Button 4"),
        PointerButton::Extra2 => crate::tr!("Bouton 5", "Button 5"),
    }
}

fn defaults(player: u8) -> [Bind; 8] {
    let k = |a: Key, b: Option<Key>| [Some(a), b];
    if player == 2 {
        // joueur 2 (liaison) : les flèches, et des lettres que le joueur 1 n'utilise pas
        return [
            Bind { keys: k(Key::ArrowUp, None), mouse: None },
            Bind { keys: k(Key::ArrowDown, None), mouse: None },
            Bind { keys: k(Key::ArrowLeft, None), mouse: None },
            Bind { keys: k(Key::ArrowRight, None), mouse: None },
            Bind { keys: k(Key::M, None), mouse: None }, // A
            Bind { keys: k(Key::N, None), mouse: None }, // B
            Bind { keys: k(Key::J, None), mouse: None }, // start
            Bind { keys: k(Key::H, None), mouse: None }, // select
        ];
    }
    [
        Bind { keys: k(Key::Z, Some(Key::ArrowUp)), mouse: None },                 // haut
        Bind { keys: k(Key::S, Some(Key::ArrowDown)), mouse: None },               // bas
        Bind { keys: k(Key::Q, Some(Key::ArrowLeft)), mouse: None },               // gauche
        Bind { keys: k(Key::D, Some(Key::ArrowRight)), mouse: None },              // droite
        Bind { keys: k(Key::L, Some(Key::X)), mouse: Some(PointerButton::Primary) },   // A
        Bind { keys: k(Key::K, Some(Key::C)), mouse: Some(PointerButton::Secondary) }, // B
        Bind { keys: k(Key::Enter, Some(Key::P)), mouse: None },                   // start
        Bind { keys: k(Key::Backspace, Some(Key::O)), mouse: None },               // select
    ]
}

impl Controls {
    /// Les touches du joueur 1 (`controls.json`) ou du joueur 2 (`controls2.json`, utilisé en liaison).
    pub fn load(player: u8) -> Self {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("emuland");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(if player == 2 { "controls2.json" } else { "controls.json" });
        let mut binds = defaults(player);
        if let Some(saved) = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Vec<SavedBind>>(&s).ok()) {
            for s in saved {
                if let Some(i) = ACTIONS.iter().position(|a| format!("{a:?}") == s.action) {
                    binds[i] = Bind {
                        keys: [s.key1.as_deref().and_then(Key::from_name), s.key2.as_deref().and_then(Key::from_name)],
                        mouse: s.mouse.as_deref().and_then(mouse_from_text),
                    };
                }
            }
        }
        Self { binds, path, player }
    }

    pub fn player(&self) -> u8 {
        self.player
    }

    /// Toutes les touches du clavier utilisées par ces réglages.
    pub fn keys(&self) -> Vec<Key> {
        self.binds.iter().flat_map(|b| b.keys).flatten().collect()
    }

    pub fn save(&self) {
        let saved: Vec<SavedBind> = ACTIONS
            .iter()
            .zip(&self.binds)
            .map(|(a, b)| SavedBind {
                action: format!("{a:?}"),
                key1: b.keys[0].map(|k| k.name().to_string()),
                key2: b.keys[1].map(|k| k.name().to_string()),
                mouse: b.mouse.map(|m| mouse_to_text(m).to_string()),
            })
            .collect();
        if let Ok(json) = serde_json::to_string_pretty(&saved) {
            let _ = fs::write(&self.path, json);
        }
    }

    pub fn reset(&mut self) {
        self.binds = defaults(self.player);
        self.save();
    }

    pub fn set(&mut self, action: Action, slot: Slot, key: Option<Key>, mouse: Option<PointerButton>) {
        let b = &mut self.binds[ACTIONS.iter().position(|a| *a == action).unwrap()];
        match slot {
            Slot::Key1 => b.keys[0] = key,
            Slot::Key2 => b.keys[1] = key,
            Slot::Mouse => b.mouse = mouse,
        }
        self.save();
    }

    /// Les boutons de la Game Boy actuellement enfoncés. La souris ne compte que si le pointeur
    /// est sur l'écran du jeu (sinon cliquer sur « Pause » appuierait aussi sur A).
    /// `exclude` : des touches à ignorer (celles de l'autre joueur, pour qu'une touche ne fasse jouer qu'une console).
    pub fn state(&self, ctx: &egui::Context, pointer_on_screen: bool, exclude: &[Key]) -> u8 {
        let mut state = ctx.input(|i| {
            let mut s = 0u8;
            for (action, bind) in ACTIONS.iter().zip(&self.binds) {
                let key = bind.keys.iter().flatten().any(|k| !exclude.contains(k) && i.key_down(*k));
                let mouse = pointer_on_screen && bind.mouse.is_some_and(|m| i.pointer.button_down(m));
                if key || mouse {
                    s |= action.mask();
                }
            }
            s
        });
        // gauche + droite (ou haut + bas) en même temps est impossible sur la vraie croix : on annule
        if state & (pad::LEFT | pad::RIGHT) == (pad::LEFT | pad::RIGHT) {
            state &= !(pad::LEFT | pad::RIGHT);
        }
        if state & (pad::UP | pad::DOWN) == (pad::UP | pad::DOWN) {
            state &= !(pad::UP | pad::DOWN);
        }
        state
    }
}

/// Fenêtre de réglage des touches. `capture` est l'emplacement en attente d'une touche ou d'un clic.
/// Renvoie `false` quand l'utilisateur la ferme.
pub fn window(ctx: &egui::Context, controls: &mut Controls, capture: &mut Option<(Action, Slot)>, switch: &mut u8, two_players: bool) -> bool {
    // une touche ou un clic pendant la capture devient le nouveau réglage
    if let Some((action, slot)) = *capture {
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Key { key, pressed: true, .. } if slot != Slot::Mouse => {
                    if key == Key::Escape {
                        *capture = None; // Échap annule
                    } else {
                        controls.set(action, slot, Some(key), None);
                        *capture = None;
                    }
                    break;
                }
                egui::Event::PointerButton { button, pressed: true, .. } if slot == Slot::Mouse => {
                    controls.set(action, slot, None, Some(button));
                    *capture = None;
                    break;
                }
                egui::Event::Key { key: Key::Escape, pressed: true, .. } => {
                    *capture = None;
                    break;
                }
                _ => {}
            }
        }
    }

    let mut open = true;
    let mut close = false;
    egui::Window::new(crate::tr!("Contrôles", "Controls"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0, LINE)).corner_radius(egui::CornerRadius::same(6)).inner_margin(egui::Margin::same(20)))
        .show(ctx, |ui| {
            if two_players {
                ui.horizontal(|ui| {
                    for p in [1u8, 2] {
                        let selected = controls.player() == p;
                        let button = egui::Button::new(RichText::new(format!("{} {p}", crate::tr!("Joueur", "Player"))).color(if selected { INK } else { MUTED })).fill(if selected { HOVER } else { BG });
                        if ui.add(button).clicked() {
                            *switch = p;
                            *capture = None;
                        }
                    }
                });
                ui.add_space(4.0);
            }
            ui.label(RichText::new(crate::tr!("Clique sur une case, puis appuie sur la touche ou le bouton de souris voulu. Clic droit sur une case pour la vider.", "Click a box, then press the key or mouse button you want. Right-click a box to clear it.")).size(13.0).color(MUTED));
            ui.label(RichText::new(crate::tr!("La souris ne compte que quand le pointeur est sur l'écran du jeu.", "The mouse only counts while the pointer is over the game screen.")).size(13.0).color(MUTED));
            ui.add_space(8.0);
            egui::Grid::new("binds").num_columns(4).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label(RichText::new("").size(12.0));
                for head in [crate::tr!("Touche", "Key"), crate::tr!("Autre touche", "Other key"), crate::tr!("Souris", "Mouse")] {
                    ui.label(RichText::new(head).size(12.0).color(MUTED));
                }
                ui.end_row();
                for (action, bind) in ACTIONS.iter().zip(controls.binds.clone()) {
                    ui.label(RichText::new(action.label()).size(15.0));
                    for slot in [Slot::Key1, Slot::Key2, Slot::Mouse] {
                        let waiting = *capture == Some((*action, slot));
                        let text = if waiting {
                            crate::tr!("Appuie…", "Press…").to_string()
                        } else {
                            match slot {
                                Slot::Key1 => bind.keys[0].map(key_label),
                                Slot::Key2 => bind.keys[1].map(key_label),
                                Slot::Mouse => bind.mouse.map(|m| mouse_label(m).to_string()),
                            }
                            .unwrap_or_else(|| "—".to_string())
                        };
                        let color = if waiting { INK } else { MUTED };
                        let button = egui::Button::new(RichText::new(text).size(14.0).color(color))
                            .min_size(egui::vec2(120.0, 28.0))
                            .stroke(Stroke::new(1.0, if waiting { INK } else { LINE }));
                        let response = ui.add(button);
                        if response.clicked() {
                            *capture = Some((*action, slot));
                        }
                        if response.secondary_clicked() {
                            controls.set(*action, slot, None, None);
                            *capture = None;
                        }
                    }
                    ui.end_row();
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(crate::tr!("Rétablir par défaut", "Reset to defaults")).clicked() {
                    controls.reset();
                    *capture = None;
                }
                if ui.add(egui::Button::new(RichText::new(crate::tr!("Fermer", "Close")).color(BG)).fill(INK).stroke(Stroke::NONE)).clicked() {
                    close = true;
                }
            });
        });
    let _ = Color32::TRANSPARENT;
    if close || !open {
        *capture = None;
        return false;
    }
    true
}
