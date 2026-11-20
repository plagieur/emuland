// En version finale, pas de fenêtre de console derrière l'application.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod controls;
mod i18n;
mod library;
mod menu;
mod player;
mod splash;
mod testrun;
mod theme;

use controls::{Action, Controls, Slot};
use eframe::egui;
use library::Library;
use player::{Player, PlayerAction};
use std::path::PathBuf;
use std::time::{Duration, Instant};

enum Screen {
    Menu,
    Player(Box<Player>),
}

pub struct App {
    screen: Screen,
    library: Library,
    started: Instant,
    splash_done: bool,
    /// Le flux son doit rester vivant tant que l'application tourne.
    _audio_stream: Option<cpal::Stream>,
    sink: Option<audio::Sink>,
    controls: Controls,
    /// Touches du joueur 2 (liaison).
    controls2: Controls,
    /// Joueur dont la fenêtre des contrôles montre les touches (1 ou 2).
    controls_player: u8,
    controls_open: bool,
    fullscreen: bool,
    /// Emplacement d'état sauvegardé choisi (de 1 à `STATE_SLOTS`).
    state_slot: u8,
    /// Emplacement du réglage des touches qui attend une touche ou un clic.
    capture: Option<(Action, Slot)>,
    /// Message affiché en bas de l'écran : (texte, succès ?, instant d'apparition).
    toast: Option<(String, bool, Instant)>,
    /// Mode de test (`--test`) : ROM lancée directement, boutons scriptés, captures automatiques.
    test: Option<testrun::TestRun>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, test: Option<testrun::TestRun>) -> Self {
        theme::apply(&cc.egui_ctx);
        // en mode de test, pas de son : l'application tourne en arrière-plan, hors de l'écran
        let (stream, sink) = match if test.is_some() { None } else { audio::start() } {
            Some((stream, sink)) => (Some(stream), Some(sink)),
            None => (None, None),
        };
        let mut app = Self {
            screen: Screen::Menu,
            library: Library::load(),
            started: Instant::now(),
            splash_done: false,
            _audio_stream: stream,
            sink,
            controls: Controls::load(1),
            controls2: Controls::load(2),
            controls_player: 1,
            controls_open: false,
            fullscreen: false,
            state_slot: 1,
            capture: None,
            toast: None,
            test,
        };
        let menu_test = app.test.as_ref().is_some_and(|t| t.menu);
        if menu_test {
            app.import_demo(); // le menu de test montre la ROM de démonstration (utiliser un APPDATA isolé)
        }
        if let Some(t) = &app.test {
            app.splash_done = true;
            if let Some(rom) = &t.rom {
                match std::fs::read(rom).map_err(|e| e.to_string()).and_then(|bytes| {
                    let title = rom.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    Player::new("test".into(), title, bytes, true, None, None)
                }) {
                    Ok(p) => app.screen = Screen::Player(Box::new(p)),
                    Err(e) => eprintln!("ROM de test impossible à lancer : {e}"),
                }
            }
            // --link <rom> : une deuxième console branchée sur le câble
            if let (Some(link_rom), Screen::Player(p)) = (&t.link, &mut app.screen) {
                let title = link_rom.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                match std::fs::read(link_rom).map_err(|e| e.to_string()).and_then(|bytes| p.link("test2".into(), title, bytes, None)) {
                    Ok(()) => {}
                    Err(e) => eprintln!("2e console de test impossible à brancher : {e}"),
                }
            }
            if t.fullscreen {
                // seule la mise en page change : la fenêtre de test ne passe JAMAIS réellement en plein écran,
                // sinon elle recouvrirait l'écran de l'utilisateur
                app.fullscreen = true;
            }
        }
        app
    }

    pub(crate) fn notify(&mut self, text: impl Into<String>, ok: bool) {
        self.toast = Some((text.into(), ok, Instant::now()));
    }

    /// Importe des fichiers (choisis dans la fenêtre de dialogue ou déposés sur la fenêtre).
    pub(crate) fn import_paths(&mut self, paths: Vec<PathBuf>) {
        let (mut added, mut last_ok) = (0, String::new());
        for path in paths {
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let result = std::fs::read(&path).map_err(|e| format!("{} : {e}", tr!("lecture impossible", "cannot read the file"))).and_then(|b| self.library.import(&name, b));
            match result {
                Ok(title) => {
                    added += 1;
                    last_ok = title;
                }
                Err(e) => {
                    self.notify(format!("{name} : {e}"), false);
                    return;
                }
            }
        }
        match added {
            0 => {}
            1 => self.notify(tr!(format!("« {last_ok} » ajouté à la bibliothèque"), format!("\"{last_ok}\" added to the library")), true),
            n => self.notify(tr!(format!("{n} ROMs ajoutées à la bibliothèque"), format!("{n} ROMs added to the library")), true),
        }
    }

    pub(crate) fn import_demo(&mut self) {
        match self.library.import("emuland demo", gb_core::demo::demo_rom()) {
            Ok(_) => self.notify(tr!("ROM de démonstration ajoutée", "Demo ROM added"), true),
            Err(e) => self.notify(e, false),
        }
    }

    pub(crate) fn launch(&mut self, id: &str) {
        let Some(entry) = self.library.entries.iter().find(|e| e.id == id).cloned() else { return };
        let Some(bytes) = self.library.rom_bytes(id) else {
            self.notify(tr!("fichier de ROM introuvable", "ROM file not found"), false);
            return;
        };
        let save = self.library.load_save(id);
        match Player::new(entry.id.clone(), entry.title.clone(), bytes, entry.supported, save, self.sink.clone()) {
            Ok(p) => self.screen = Screen::Player(Box::new(p)),
            Err(e) => self.notify(i18n::core_error(&e), false),
        }
    }

    fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((text, ok, at)) = self.toast.clone() else { return };
        if at.elapsed() > Duration::from_secs(4) {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        egui::Area::new(egui::Id::unique("toast")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -28.0]).show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::SURFACE)
                .stroke(egui::Stroke::new(1.0, if ok { theme::DIM } else { theme::BAD }))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(text).color(if ok { theme::INK } else { theme::BAD }));
                });
        });
    }
}

impl eframe::App for App {
    /// Appelé régulièrement et à la fermeture de la fenêtre : on y garde la sauvegarde du jeu en cours.
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if self.test.is_some() {
            return; // le mode de test ne touche à aucune sauvegarde
        }
        if let Screen::Player(p) = &mut self.screen {
            if let Some(data) = p.pending_save(true) {
                self.library.write_save(&p.id, &data);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // mode de test : boutons scriptés, captures de la fenêtre, fermeture automatique
        let (mut test_save, mut test_load, mut test_fullscreen, mut test_controls) = (false, false, false, false);
        if let Some(test) = &mut self.test {
            for action in test.due_actions(self.started.elapsed().as_secs_f64()) {
                match action.as_str() {
                    "save" => test_save = true,
                    "load" => test_load = true,
                    "controls" => test_controls = true,
                    _ => test_fullscreen = true,
                }
            }
            let t = self.started.elapsed().as_secs_f64();
            if let Screen::Player(p) = &mut self.screen {
                p.set_scripted(test.mask_at(t), test.mask2_at(t));
            }
            test.poll(&ctx, t);
            if test.finished(t) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
        // fichiers déposés sur la fenêtre
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() && self.splash_done {
            self.import_paths(dropped);
        }

        // F11 bascule le plein écran ; Échap en sort (sauf quand la fenêtre des contrôles attend une touche)
        let in_game = matches!(self.screen, Screen::Player(_));
        let (f11, escape, f5, f8) = ctx.input(|i| (i.key_pressed(egui::Key::F11), i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::F5), i.key_pressed(egui::Key::F8)));
        let (f5, f8) = (f5 || test_save, f8 || test_load);
        if test_controls {
            self.controls_open = true;
        }
        let mut toggle_fullscreen = in_game && (f11 || test_fullscreen);
        if in_game && escape && self.fullscreen && !self.controls_open {
            toggle_fullscreen = true;
        }

        let choices: Vec<(String, String)> = self.library.entries.iter().map(|e| (e.id.clone(), e.title.clone())).collect();
        let mut back = false;
        let mut link_request: Option<Option<String>> = None; // Some(Some(id)) : brancher ; Some(None) : débrancher
        if let Screen::Player(p) = &mut self.screen {
            let mut action = p.show(ui, &self.controls, &self.controls2, &choices, self.controls_open, self.fullscreen, &mut self.state_slot);
            match &action {
                PlayerAction::Link(id) => link_request = Some(Some(id.clone())),
                PlayerAction::Unlink => link_request = Some(None),
                _ => {}
            }
            if !self.controls_open && matches!(action, PlayerAction::None) {
                if f5 {
                    action = PlayerAction::SaveState;
                } else if f8 {
                    action = PlayerAction::LoadState;
                }
            }
            back = matches!(action, PlayerAction::Back);
            if matches!(action, PlayerAction::OpenControls) {
                self.controls_open = true;
            }
            if matches!(action, PlayerAction::ToggleFullscreen) {
                toggle_fullscreen = true;
            }
            if p.is_linked() && matches!(action, PlayerAction::SaveState | PlayerAction::LoadState) {
                // une photo d'une seule console casserait la liaison : les deux doivent rester synchronisées
                self.toast = Some((tr!("Les états ne marchent pas en liaison : débranche le câble d'abord", "States don't work while linked: unplug the cable first").into(), false, Instant::now()));
                action = PlayerAction::None;
            }
            match action {
                PlayerAction::SaveState => {
                    let data = p.snapshot();
                    let slot = self.state_slot;
                    if self.library.write_state(&p.id, slot, &data) {
                        self.toast = Some((tr!(format!("État {slot} enregistré"), format!("State {slot} saved")), true, Instant::now()));
                    }
                }
                PlayerAction::LoadState => {
                    let slot = self.state_slot;
                    let message = match self.library.read_state(&p.id, slot) {
                        None => (tr!(format!("L'état {slot} est vide"), format!("State {slot} is empty")), false),
                        Some(data) => match p.restore(&data) {
                            Ok(()) => (tr!(format!("État {slot} rechargé"), format!("State {slot} loaded")), true),
                            Err(e) => (tr!(format!("État {slot} refusé : {}", i18n::core_error(&e)), format!("State {slot} refused: {e}")), false),
                        },
                    };
                    self.toast = Some((message.0, message.1, Instant::now()));
                }
                _ => {}
            }
            if self.test.is_none() {
                if let Some(data) = p.pending_save(back) {
                    self.library.write_save(&p.id, &data);
                }
                if let (Some(data), Some(id)) = (p.pending_peer_save(back), p.peer_save_id()) {
                    self.library.write_save(&id, &data);
                }
            }
        }
        // brancher ou débrancher le câble link
        if let (Some(request), Screen::Player(p)) = (link_request, &mut self.screen) {
            match request {
                None => {
                    // on enregistre la sauvegarde du joueur 2 avant de le débrancher
                    if let (Some(data), Some(id)) = (p.pending_peer_save(true), p.peer_save_id()) {
                        self.library.write_save(&id, &data);
                    }
                    p.unlink();
                    self.toast = Some((tr!("Câble débranché", "Cable unplugged").into(), true, Instant::now()));
                }
                Some(id) => {
                    let entry = self.library.entries.iter().find(|e| e.id == id).cloned();
                    let message = match (entry, self.library.rom_bytes(&id)) {
                        (Some(entry), Some(bytes)) => {
                            let save = self.library.load_save(&format!("{id}.p2"));
                            match p.link(id.clone(), entry.title.clone(), bytes, save) {
                                Ok(()) => (tr!(format!("Câble branché : joueur 2 avec « {} »", entry.title), format!("Cable plugged in: player 2 with \"{}\"", entry.title)), true),
                                Err(e) => (i18n::core_error(&e), false),
                            }
                        }
                        _ => (tr!("jeu introuvable", "game not found").to_string(), false),
                    };
                    self.toast = Some((message.0, message.1, Instant::now()));
                }
            }
        }
        if back {
            self.screen = Screen::Menu;
            if self.fullscreen {
                toggle_fullscreen = true; // on ne laisse pas le menu en plein écran
            }
        }
        if toggle_fullscreen {
            self.fullscreen = !self.fullscreen;
            if self.test.is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            if self.fullscreen {
                self.notify(tr!("Plein écran : Échap ou F11 pour en sortir", "Full screen: Esc or F11 to leave"), true);
            }
        }
        if matches!(self.screen, Screen::Menu) {
            menu::show(self, ui);
        }
        if self.controls_open {
            let linked = matches!(&self.screen, Screen::Player(p) if p.is_linked());
            let mut switch = self.controls_player;
            let shown = if self.controls_player == 2 && linked { &mut self.controls2 } else { &mut self.controls };
            let keep_open = controls::window(&ctx, shown, &mut self.capture, &mut switch, linked);
            self.controls_player = switch;
            if !keep_open {
                self.controls_open = false;
            }
        }

        // une écriture qui a échoué ne doit jamais passer inaperçue
        if let Some(error) = self.library.save_error.take() {
            self.notify(error, false);
        }

        menu::drop_overlay(&ctx);
        self.draw_toast(&ctx);

        if !self.splash_done {
            let t = self.started.elapsed().as_secs_f32();
            self.splash_done = splash::show(&ctx, t);
        }
    }
}

fn main() -> eframe::Result {
    i18n::load();
    let test = testrun::TestRun::from_args();
    let mut viewport = egui::ViewportBuilder::default().with_title("emuland").with_inner_size([1080.0, 720.0]).with_min_inner_size([820.0, 560.0]);
    if test.is_some() {
        // mode de test : la fenêtre s'ouvre loin hors de l'écran, sans prendre le focus ni apparaître dans la barre des tâches
        viewport = viewport.with_active(false).with_taskbar(false).with_position([-30000.0, -30000.0]);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("emuland", options, Box::new(move |cc| Ok(Box::new(App::new(cc, test)))))
}


