//! Le menu principal : la liste des ROMs, l'import (bouton ou glisser-déposer) et le lancement.

use crate::theme::*;
use crate::tr;
use crate::App;
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, RichText, Stroke};

fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).size(14.0).color(BG)).fill(INK).stroke(Stroke::NONE).corner_radius(CornerRadius::same(4)))
}

fn plain_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).size(14.0).color(INK)))
}

fn text_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).size(14.0).color(MUTED)).fill(Color32::TRANSPARENT).stroke(Stroke::NONE))
}

pub fn show(app: &mut App, root: &mut egui::Ui) {
    let mut to_launch: Option<String> = None;
    let mut to_remove: Option<String> = None;
    let mut do_import = false;
    let mut do_demo = false;

    egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(48, 36))).show(root, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("emuland").font(FontId::new(24.0, bold())));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if plain_button(ui, tr!("Importer une ROM", "Import a ROM")).clicked() {
                    do_import = true;
                }
                if plain_button(ui, tr!("Contrôles", "Controls")).clicked() {
                    app.controls_open = true;
                }
                crate::i18n::menu(ui);
            });
        });
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(6.0);

        if app.library.entries.is_empty() {
            ui.add_space(24.0);
            ui.label(RichText::new(tr!("Aucune ROM.", "No ROMs yet.")).size(18.0));
            ui.label(RichText::new(tr!("Glisse un fichier .gb ou .gbc sur la fenêtre, ou clique sur « Importer une ROM ».", "Drop a .gb or .gbc file on the window, or click \"Import a ROM\".")).size(14.0).color(MUTED));
            ui.add_space(8.0);
            if text_button(ui, tr!("Ajouter la ROM de démonstration", "Add the demo ROM")).clicked() {
                do_demo = true;
            }
        } else {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for e in app.library.entries.clone() {
                    ui.horizontal(|ui| {
                        ui.set_min_height(54.0);
                        ui.vertical(|ui| {
                            ui.add_space(6.0);
                            ui.label(RichText::new(&e.title).size(17.0));
                            if e.supported {
                                let kind = if e.color { " · Color" } else { "" };
                                let mapper = match e.mapper.as_str() {
                                    "ROM seule" => tr!("ROM seule", "ROM only"),
                                    "MBC1 multi-jeux" => tr!("MBC1 multi-jeux", "MBC1 multicart"),
                                    "inconnu" => tr!("inconnu", "unknown"),
                                    other => other,
                                };
                                let unit = tr!("Kio", "KiB");
                                ui.label(RichText::new(format!("{} · {} {unit}{kind}", mapper, e.rom_kib)).size(13.0).color(MUTED));
                            } else {
                                ui.label(RichText::new(format!("{} · {}", e.mapper, tr!("cartouche pas encore gérée", "cartridge not supported yet"))).size(13.0).color(WARN));
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if text_button(ui, tr!("Retirer", "Remove")).clicked() {
                                to_remove = Some(e.id.clone());
                            }
                            if primary_button(ui, tr!("Jouer", "Play")).clicked() {
                                to_launch = Some(e.id.clone());
                            }
                        });
                    });
                    ui.separator();
                }
            });
        }

        ui.painter().text(
            egui::pos2(ui.max_rect().left(), ui.max_rect().bottom()),
            Align2::LEFT_BOTTOM,
            tr!("emuland ne fournit aucun jeu : importe uniquement des ROMs que tu possèdes légalement.", "emuland provides no games: only import ROMs you legally own."),
            FontId::proportional(12.0),
            DIM,
        );
    });

    if do_import {
        if let Some(paths) = rfd::FileDialog::new().set_title(tr!("Importer des ROMs Game Boy", "Import Game Boy ROMs")).add_filter(tr!("ROMs Game Boy", "Game Boy ROMs"), &["gb", "gbc"]).pick_files() {
            app.import_paths(paths);
        }
    }
    if do_demo {
        app.import_demo();
    }
    if let Some(id) = to_remove {
        app.library.remove(&id);
    }
    if let Some(id) = to_launch {
        app.launch(&id);
    }
}

/// Voile « dépose pour importer » quand un fichier survole la fenêtre.
pub fn drop_overlay(ctx: &egui::Context) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("drop")));
    painter.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 230));
    painter.rect_stroke(screen.shrink(24.0), CornerRadius::same(4), Stroke::new(1.0, MUTED), egui::StrokeKind::Inside);
    painter.text(screen.center(), Align2::CENTER_CENTER, tr!("Dépose la ROM pour l'importer", "Drop the ROM to import it"), FontId::new(22.0, bold()), INK);
}
