//! La console entière : un processeur branché sur une carte mémoire.

use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::mmu::Mmu;
use crate::state::{StateReader, StateWriter};

const STATE_MAGIC: &[u8; 4] = b"EMUS";
const STATE_VERSION: u8 = 5;

/// Une image dure 70 224 T-cycles (environ 59,7 images par seconde).
pub const CYCLES_PER_FRAME: u32 = 70_224;

pub struct GameBoy {
    pub cpu: Cpu,
    pub mmu: Mmu,
    pub total_cycles: u64,
    rom: Vec<u8>,
}

impl GameBoy {
    pub fn new(rom: Vec<u8>) -> Result<Self, String> {
        let cart = Cartridge::new(rom.clone())?;
        // un jeu qui a des graphismes en couleurs tourne en mode Game Boy Color
        let cpu = if cart.info.cgb { Cpu::new_cgb() } else { Cpu::new() };
        Ok(Self { cpu, mmu: Mmu::new(cart), total_cycles: 0, rom })
    }

    /// Branche (ou débranche) le câble link : à brancher sur les deux consoles avant `gb_core::link::run_frame_pair`.
    pub fn set_link_connected(&mut self, on: bool) {
        self.mmu.set_link_connected(on);
    }

    /// Inclinaison de la console pour les jeux à accéléromètre (de -1 à +1 sur chaque axe). Sans effet sur les autres jeux.
    pub fn set_tilt(&mut self, x: f32, y: f32) {
        self.mmu.cart.set_tilt(x, y);
    }

    /// Vrai si le jeu utilise l'inclinaison (cartouche MBC7).
    pub fn uses_tilt(&self) -> bool {
        self.mmu.cart.has_tilt()
    }

    /// Fournit l'image de la Pocket Camera : 128 × 112 pixels en gris.
    pub fn set_camera_image(&mut self, gray: &[u8]) {
        self.mmu.cart.set_camera_image(gray);
    }

    /// Vrai si le jeu tourne en mode Game Boy Color (images en couleurs).
    pub fn is_cgb(&self) -> bool {
        self.mmu.is_cgb()
    }

    /// L'image en couleurs (15 bits par pixel : rouge en bas, puis vert, puis bleu), en mode Game Boy Color.
    pub fn frame_color(&self) -> &[u16] {
        &self.mmu.ppu.frame_color
    }

    /// Redémarre la console avec la même ROM.
    pub fn reset(&mut self) {
        let save = self.mmu.cart.save_bytes(0);
        let rate = self.mmu.apu.sample_rate();
        if let Ok(mut fresh) = Self::new(self.rom.clone()) {
            fresh.mmu.apu.set_sample_rate(rate);
            fresh.mmu.cart.load_save(&save, 0); // la pile de la cartouche garde la sauvegarde (et l'heure), comme sur la vraie console
            *self = fresh;
        }
    }

    /// Fréquence des échantillons sonores demandée (celle de la carte son, par exemple 48 000 Hz).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.mmu.apu.set_sample_rate(rate);
    }

    /// Les échantillons sonores produits depuis le dernier appel : paires gauche/droite entre -1 et 1.
    pub fn take_audio(&mut self) -> Vec<f32> {
        self.mmu.apu.take_samples()
    }

    /// Boutons actuellement enfoncés : un OU des constantes de `gb_core::joypad`.
    pub fn set_buttons(&mut self, state: u8) {
        self.mmu.set_buttons(state);
    }

    /// Recharge une sauvegarde. `now_unix` est l'heure réelle actuelle (secondes depuis 1970) : l'horloge
    /// d'une cartouche qui en a une avance du temps écoulé depuis l'enregistrement.
    pub fn load_save(&mut self, data: &[u8], now_unix: u64) -> bool {
        self.mmu.cart.load_save(data, now_unix)
    }

    /// Vrai si la cartouche garde des données sur pile (donc à sauvegarder) : RAM ou horloge.
    pub fn has_save(&self) -> bool {
        let cart = &self.mmu.cart;
        cart.info.has_battery && (!cart.ram().is_empty() || cart.info.has_rtc)
    }

    /// Vrai si la cartouche contient une horloge temps réel.
    pub fn has_clock(&self) -> bool {
        self.mmu.cart.info.has_rtc
    }

    /// La RAM sauvegardée de la cartouche, pour repérer si elle a changé (sans l'horloge, qui change à chaque seconde).
    pub fn save_ram(&self) -> Option<&[u8]> {
        if self.has_save() { Some(self.mmu.cart.ram()) } else { None }
    }

    /// Le fichier de sauvegarde complet à écrire sur le disque (RAM, puis horloge et heure réelle `now_unix`).
    pub fn save_data(&self, now_unix: u64) -> Option<Vec<u8>> {
        if self.has_save() { Some(self.mmu.cart.save_bytes(now_unix)) } else { None }
    }

    /// Une empreinte de la ROM (son en-tête et sa taille) : un état ne se recharge que sur la ROM qui l'a produit.
    fn rom_id(&self) -> u32 {
        let mut h: u32 = 0x811C_9DC5;
        for &b in self.rom[0x100..0x150].iter().chain(&(self.rom.len() as u32).to_le_bytes()) {
            h = (h ^ b as u32).wrapping_mul(0x0100_0193);
        }
        h
    }

    /// Une photo complète de la console : à recharger avec `load_state`.
    pub fn save_state(&self) -> Vec<u8> {
        let mut w = StateWriter::new();
        w.bytes(STATE_MAGIC);
        w.u8(STATE_VERSION);
        w.u32(self.rom_id());
        w.u64(self.total_cycles);
        self.cpu.save_state(&mut w);
        self.mmu.save_state(&mut w);
        w.finish()
    }

    /// Recharge un état. En cas d'erreur (autre ROM, fichier abîmé, ancienne version), la console n'est pas modifiée.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let mut r = StateReader::new(data);
        let mut magic = [0u8; 4];
        r.bytes_into(&mut magic).map_err(|_| "this file is not an emuland state".to_string())?;
        if magic != *STATE_MAGIC {
            return Err("this file is not an emuland state".into());
        }
        let version = r.u8()?;
        if version != STATE_VERSION {
            return Err(format!("this state comes from another emuland version (format {version}, expected {STATE_VERSION})"));
        }
        if r.u32()? != self.rom_id() {
            return Err("this state was saved with another ROM".into());
        }
        // on charge dans une console neuve : si quelque chose cloche en route, l'ancienne reste intacte
        let mut fresh = Self::new(self.rom.clone())?;
        fresh.total_cycles = r.u64()?;
        fresh.cpu.load_state(&mut r)?;
        fresh.mmu.load_state(&mut r)?;
        if !r.is_finished() {
            return Err("the state has extra data: it is damaged".into());
        }
        fresh.mmu.apu.set_sample_rate(self.mmu.apu.sample_rate());
        *self = fresh;
        Ok(())
    }

    /// Exécute une instruction et fait avancer le temps. Renvoie les T-cycles écoulés.
    pub fn step(&mut self) -> u32 {
        // le processeur fait avancer le reste de la console lui-même, accès mémoire par accès mémoire
        let cycles = self.cpu.step(&mut self.mmu);
        self.total_cycles += cycles as u64;
        cycles
    }

    /// Exécute jusqu'à ce qu'une image soit terminée (ou, écran éteint, l'équivalent d'une image).
    pub fn run_frame(&mut self) {
        // en vitesse double (Game Boy Color), le processeur fait deux fois plus de cycles par image
        let speed = if self.mmu.is_double_speed() { 2 } else { 1 };
        let mut elapsed = 0;
        while elapsed < CYCLES_PER_FRAME * 2 * speed {
            elapsed += self.step();
            if self.mmu.ppu.take_frame_ready() || (!self.mmu.ppu.lcd_on() && elapsed >= CYCLES_PER_FRAME * speed) {
                break;
            }
        }
    }

    /// La dernière image terminée : 160 × 144 nuances de 0 (clair) à 3 (foncé).
    pub fn frame(&self) -> &[u8] {
        &self.mmu.ppu.frame
    }

    /// Texte envoyé sur la liaison série jusqu'ici.
    pub fn serial_text(&self) -> String {
        String::from_utf8_lossy(&self.mmu.serial).into_owned()
    }
}
