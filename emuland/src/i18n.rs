//! Les langues de l'interface : français et anglais. Le choix est enregistré dans `%APPDATA%\emuland\settings.json`.
//! Au premier lancement, la langue de Windows décide (français si c'est du français, anglais sinon).
//!
//! Usage : `tr!("texte français", "english text")` renvoie le bon des deux selon la langue en cours.
//! Les deux côtés doivent avoir le même type (deux `&str`, ou deux `String`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

static LANG: AtomicU8 = AtomicU8::new(0); // 0 = français, 1 = anglais

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::Fr => "fr",
            Lang::En => "en",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Fr => "Français",
            Lang::En => "English",
        }
    }
}

#[macro_export]
macro_rules! tr {
    ($fr:expr, $en:expr) => {
        if $crate::i18n::is_en() { $en } else { $fr }
    };
}

pub fn get() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 { Lang::En } else { Lang::Fr }
}

pub fn is_en() -> bool {
    get() == Lang::En
}

pub fn set(lang: Lang) {
    LANG.store(if lang == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
}

fn settings_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("emuland").join("settings.json")
}

/// La langue de l'interface de Windows : français ou non.
#[cfg(windows)]
fn system_is_french() -> bool {
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // SAFETY : fonction de kernel32 sans argument, qui renvoie un identifiant de langue
    let id = unsafe { GetUserDefaultUILanguage() };
    id & 0x3FF == 0x0C // 0x0C = langue principale « français »
}

#[cfg(not(windows))]
fn system_is_french() -> bool {
    std::env::var("LANG").is_ok_and(|l| l.starts_with("fr"))
}

/// Choisit la langue au démarrage : celle qui a été enregistrée, sinon celle du système.
pub fn load() {
    let saved = std::fs::read_to_string(settings_path()).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).and_then(|v| v["lang"].as_str().map(str::to_owned));
    let lang = match saved.as_deref() {
        Some("en") => Lang::En,
        Some("fr") => Lang::Fr,
        _ => {
            if system_is_french() { Lang::Fr } else { Lang::En }
        }
    };
    set(lang);
}

/// Change de langue et l'enregistre.
pub fn choose(lang: Lang) {
    set(lang);
    let path = settings_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::json!({ "lang": lang.code() }).to_string());
}

/// Traduit un message d'erreur du cœur de l'émulateur (écrit en anglais) quand l'interface est en français.
pub fn core_error(message: &str) -> String {
    if is_en() {
        return message.to_string();
    }
    const TABLE: [(&str, &str); 10] = [
        ("this file is too small to be a Game Boy ROM", "ce fichier est trop petit pour être une ROM Game Boy"),
        ("this file is not an emuland state", "ce fichier n'est pas un état emuland"),
        ("this state comes from another emuland version", "cet état vient d'une autre version d'emuland"),
        ("this state was saved with another ROM", "cet état a été enregistré avec une autre ROM"),
        ("this state comes from another mode", "cet état vient d'un autre mode (Game Boy ou Game Boy Color)"),
        ("the state has extra data", "l'état contient des données en trop : il est abîmé"),
        ("the state is truncated or damaged", "l'état est tronqué ou abîmé"),
        ("unexpected size in the state", "taille inattendue dans l'état : il est abîmé"),
        ("the state is damaged", "l'état est abîmé"),
        ("invalid screen state", "état de l'écran invalide"),
    ];
    for (english, french) in TABLE {
        if message.starts_with(english) {
            return french.to_string();
        }
    }
    message.to_string()
}

/// Le menu « Langue » : à placer dans une barre d'outils.
pub fn menu(ui: &mut eframe::egui::Ui) {
    ui.menu_button(tr!("Langue", "Language"), |ui| {
        for lang in [Lang::Fr, Lang::En] {
            let label = if get() == lang { format!("• {}", lang.name()) } else { format!("  {}", lang.name()) };
            if ui.button(label).clicked() {
                choose(lang);
                ui.close();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr_picks_the_current_language() {
        set(Lang::Fr);
        assert_eq!(tr!("Jouer", "Play"), "Jouer");
        set(Lang::En);
        assert_eq!(tr!("Jouer", "Play"), "Play");
        set(Lang::Fr);
    }

    #[test]
    fn core_errors_are_translated_only_into_french() {
        set(Lang::Fr);
        assert!(core_error("this state was saved with another ROM").contains("autre ROM"));
        assert!(core_error("this state comes from another emuland version (format 2, expected 5)").contains("autre version"));
        assert_eq!(core_error("something unknown"), "something unknown");
        set(Lang::En);
        assert_eq!(core_error("this state was saved with another ROM"), "this state was saved with another ROM");
        set(Lang::Fr);
    }
}
