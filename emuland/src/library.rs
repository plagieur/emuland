//! La bibliothèque : les ROMs importées, copiées dans le dossier de données d'emuland.

use gb_core::CartridgeInfo;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const MAX_ROM_BYTES: usize = 8 * 1024 * 1024; // une cartouche Game Boy ne dépasse pas 8 Mio

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub mapper: String,
    pub rom_kib: usize,
    pub supported: bool,
    /// Jeu Game Boy Color (graphismes en couleurs).
    #[serde(default)]
    pub color: bool,
}

pub struct Library {
    pub entries: Vec<Entry>,
    dir: PathBuf,
    /// Dernière erreur d'écriture de la liste ou d'une sauvegarde : l'application l'affiche puis la vide.
    pub save_error: Option<String>,
}

/// Écrit un fichier de façon sûre : d'abord dans un fichier temporaire, puis renommé à sa place.
/// Réessaie quelques fois, car un antivirus ou l'indexeur Windows peut tenir le fichier ouvert un instant.
fn write_atomic(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut last = String::new();
    for attempt in 0..6 {
        match fs::write(&tmp, data).and_then(|_| fs::rename(&tmp, path)) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(std::time::Duration::from_millis(60 * (attempt + 1)));
    }
    Err(last)
}

/// Empreinte FNV-1a 64 bits : sert de nom de fichier et détecte les doublons.
fn fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Lit seulement l'en-tête (les 336 premiers octets) d'une ROM déjà importée.
fn read_header(path: &std::path::Path) -> Option<CartridgeInfo> {
    use std::io::Read;
    let mut buf = [0u8; 0x150];
    fs::File::open(path).ok()?.read_exact(&mut buf).ok()?;
    CartridgeInfo::parse(&buf).ok()
}

impl Library {
    pub fn load() -> Self {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        Self::load_from(base.join("emuland"))
    }

    fn load_from(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(dir.join("roms"));
        let mut entries: Vec<Entry> = fs::read_to_string(dir.join("library.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Entry>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| dir.join("roms").join(format!("{}.gb", e.id)).exists())
            .map(|mut e| {
                // on relit l'en-tête à chaque démarrage : une ROM importée avant une mise à jour d'emuland
                // (par exemple une MBC3 qui n'était pas encore gérée) doit être reconnue maintenant
                if let Some(info) = read_header(&dir.join("roms").join(format!("{}.gb", e.id))) {
                    e.mapper = info.mapper_name.to_string();
                    e.supported = info.supported;
                    e.color = info.cgb;
                }
                e
            })
            .collect();
        // une ROM copiée dans le dossier mais absente de la liste (liste abîmée, import interrompu) est reprise
        let mut adopted = false;
        if let Ok(files) = fs::read_dir(dir.join("roms")) {
            for file in files.flatten() {
                let path = file.path();
                let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if path.extension().is_some_and(|e| e == "gb") && !entries.iter().any(|e| e.id == id) {
                    if let Some(info) = read_header(&path) {
                        let kib = file.metadata().map(|m| m.len() as usize / 1024).unwrap_or(info.rom_kib);
                        let title = if info.title == "Untitled" { id.clone() } else { info.title.clone() };
                        entries.push(Entry { id, title, mapper: info.mapper_name.to_string(), rom_kib: kib, supported: info.supported, color: info.cgb });
                        adopted = true;
                    }
                }
            }
        }
        let mut library = Self { entries, dir, save_error: None };
        if adopted {
            library.save();
        }
        library
    }

    /// Écrit la liste. En cas d'échec, garde le message dans `save_error` et dans `error.log`.
    fn save(&mut self) {
        let result = serde_json::to_string_pretty(&self.entries)
            .map_err(|e| e.to_string())
            .and_then(|json| write_atomic(&self.dir.join("library.json"), json.as_bytes()));
        if let Err(e) = result {
            self.report(format!("{} : {e}", crate::tr!("impossible d'enregistrer la liste des ROMs", "cannot save the ROM list")));
        }
    }

    fn report(&mut self, message: String) {
        use std::io::Write;
        if let Ok(mut log) = fs::OpenOptions::new().create(true).append(true).open(self.dir.join("error.log")) {
            let _ = writeln!(log, "{message}");
        }
        self.save_error = Some(message);
    }

    fn rom_path(&self, id: &str) -> PathBuf {
        self.dir.join("roms").join(format!("{id}.gb"))
    }

    /// Ajoute une ROM. `name_hint` (nom du fichier) sert de titre si l'en-tête n'en contient pas.
    pub fn import(&mut self, name_hint: &str, bytes: Vec<u8>) -> Result<String, String> {
        if bytes.len() > MAX_ROM_BYTES {
            return Err(crate::tr!("fichier trop gros pour une ROM Game Boy (8 Mio maximum)", "file too big for a Game Boy ROM (8 MiB maximum)").into());
        }
        let info = CartridgeInfo::parse(&bytes).map_err(|e| crate::i18n::core_error(&e))?;
        let id = fingerprint(&bytes);
        if self.entries.iter().any(|e| e.id == id) {
            return Err(crate::tr!("cette ROM est déjà dans ta bibliothèque", "this ROM is already in your library").into());
        }
        let title = if info.title == "Untitled" && !name_hint.is_empty() { name_hint.to_string() } else { info.title.clone() };
        fs::write(self.rom_path(&id), &bytes).map_err(|e| format!("{} : {e}", crate::tr!("impossible de copier la ROM", "cannot copy the ROM")))?;
        self.entries.push(Entry { id, title: title.clone(), mapper: info.mapper_name.to_string(), rom_kib: info.rom_kib, supported: info.supported, color: info.cgb });
        self.save();
        Ok(title)
    }

    /// Ajoute une ROM sans passer par `load` (pour les tests).
    #[cfg(test)]
    fn in_dir(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(dir.join("roms"));
        Self { entries: Vec::new(), dir, save_error: None }
    }

    pub fn remove(&mut self, id: &str) {
        let _ = fs::remove_file(self.rom_path(id)); // la sauvegarde, elle, est gardée : on la retrouve si tu réimportes la ROM
        self.entries.retain(|e| e.id != id);
        self.save();
    }

    pub fn rom_bytes(&self, id: &str) -> Option<Vec<u8>> {
        fs::read(self.rom_path(id)).ok()
    }

    fn save_path(&self, id: &str) -> PathBuf {
        let dir = self.dir.join("saves");
        let _ = fs::create_dir_all(&dir);
        dir.join(format!("{id}.sav"))
    }

    fn state_path(&self, id: &str, slot: u8) -> PathBuf {
        let dir = self.dir.join("states");
        let _ = fs::create_dir_all(&dir);
        dir.join(format!("{id}.{slot}.state"))
    }

    /// Un état sauvegardé (photo complète de la console) dans l'emplacement `slot`, s'il existe.
    pub fn read_state(&self, id: &str, slot: u8) -> Option<Vec<u8>> {
        fs::read(self.state_path(id, slot)).ok()
    }

    pub fn write_state(&mut self, id: &str, slot: u8, data: &[u8]) -> bool {
        match write_atomic(&self.state_path(id, slot), data) {
            Ok(()) => true,
            Err(e) => {
                self.report(format!("{} : {e}", crate::tr!("impossible d'enregistrer l'état", "cannot save the state")));
                false
            }
        }
    }

    /// La sauvegarde (RAM de la cartouche) d'un jeu, si elle existe.
    pub fn load_save(&self, id: &str) -> Option<Vec<u8>> {
        fs::read(self.save_path(id)).ok()
    }

    /// Écrit la sauvegarde dans un fichier temporaire puis le renomme : une coupure en plein milieu ne l'abîme pas.
    pub fn write_save(&mut self, id: &str, data: &[u8]) {
        let path = self.save_path(id);
        if let Err(e) = write_atomic(&path, data) {
            self.report(format!("{} : {e}", crate::tr!("impossible d'enregistrer la sauvegarde du jeu", "cannot save the game")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("emuland-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_import_is_written_to_library_json_immediately() {
        let dir = temp_dir("import");
        let mut lib = Library::in_dir(dir.clone());
        let title = lib.import("demo", gb_core::demo::demo_rom()).unwrap();
        assert_eq!(title, "EMULAND DEMO");
        let json = fs::read_to_string(dir.join("library.json")).expect("library.json doit exister après un import");
        assert!(json.contains("EMULAND DEMO"), "{json}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn importing_the_same_rom_twice_is_refused() {
        let dir = temp_dir("dup");
        let mut lib = Library::in_dir(dir.clone());
        lib.import("demo", gb_core::demo::demo_rom()).unwrap();
        assert!(lib.import("demo", gb_core::demo::demo_rom()).is_err());
        assert_eq!(lib.entries.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rom_missing_from_the_list_is_adopted_on_load() {
        let dir = temp_dir("adopt");
        // une ROM copiée dans le dossier mais jamais inscrite dans library.json
        fs::create_dir_all(dir.join("roms")).unwrap();
        fs::write(dir.join("roms").join("abc.gb"), gb_core::demo::demo_rom()).unwrap();
        let lib = Library::load_from(dir.clone());
        assert_eq!(lib.entries.len(), 1);
        assert_eq!(lib.entries[0].title, "EMULAND DEMO");
        let json = fs::read_to_string(dir.join("library.json")).unwrap();
        assert!(json.contains("EMULAND DEMO"));
        let _ = fs::remove_dir_all(&dir);
    }
}
