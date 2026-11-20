//! La carte mémoire de la Game Boy : ce qui répond à chaque adresse du bus.
//!
//! Il y a la ROM (via la cartouche), les RAM, l'écran (PPU), le son (APU), la manette, le timer
//! et la liaison série (qui sert aux ROMs de test à « parler »).

use crate::apu::Apu;
use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
use crate::state::{StateReader, StateWriter};
use crate::timer::Timer;

pub struct Mmu {
    pub cart: Cartridge,
    pub ppu: Ppu,
    pub joypad: Joypad,
    pub apu: Apu,
    pub timer: Timer,
    wram: Vec<u8>,
    io: Vec<u8>,
    hram: Vec<u8>,
    ie: u8,
    /// Tout ce que le programme a envoyé sur la liaison série (une ROM de test y écrit son résultat).
    pub serial: Vec<u8>,
    // copie DMA vers l'OAM : 160 octets, un par cycle machine, après un cycle d'attente
    dma_src: u16,
    dma_pos: u16, // 160 = pas de copie en cours
    dma_wait: u32,
    dma_cycles: u32,
    dma_reading: bool,
    // ----- Game Boy Color -----
    cgb: bool,
    /// Banque de WRAM visible en 0xD000–0xDFFF (registre SVBK, 1 à 7).
    svbk: u8,
    double_speed: bool,
    /// Un changement de vitesse est demandé (registre KEY1) et sera fait par la prochaine instruction STOP.
    speed_armed: bool,
    /// Reste de T-cycles quand on divise par deux en vitesse double.
    half_acc: u32,
    // copie HDMA vers la VRAM, par blocs de 16 octets
    hdma_src: u16,
    hdma_dst: u16,
    hdma_blocks: u8, // blocs restants moins un
    hdma_active: bool,
    hdma_hblank: bool,
    /// Registres 0xFF72 à 0xFF75, lisibles et modifiables sans effet.
    cgb_regs: [u8; 4],
    // ----- liaison série (câble link) -----
    /// Un transfert avec l'horloge interne est en cours (cette console « pilote » le câble).
    serial_active: bool,
    serial_cycles: u32,
    /// Un transfert avec l'horloge externe est armé : on attend que l'autre console pilote le câble.
    serial_waiting: bool,
    /// Une autre console est branchée sur le câble.
    link_connected: bool,
    /// Octet envoyé par un transfert terminé, en attente de la réponse de l'autre console.
    link_out: Option<u8>,
}

/// Octets de l'OAM copiés par une DMA.
const DMA_LENGTH: u16 = 160;

impl Mmu {
    pub fn new(cart: Cartridge) -> Self {
        let cgb = cart.info.cgb;
        let mut ppu = Ppu::new();
        ppu.set_cgb(cgb);
        let mut apu = Apu::new();
        apu.set_cgb(cgb);
        Self {
            cart,
            ppu,
            joypad: Joypad::new(),
            apu,
            timer: Timer::new(),
            wram: vec![0; 0x8000],
            io: vec![0; 0x80],
            hram: vec![0; 0x7F],
            ie: 0,
            serial: Vec::new(),
            dma_src: 0,
            dma_pos: DMA_LENGTH,
            dma_wait: 0,
            dma_cycles: 0,
            dma_reading: false,
            cgb,
            svbk: 1,
            double_speed: false,
            speed_armed: false,
            half_acc: 0,
            hdma_src: 0,
            hdma_dst: 0x8000,
            hdma_blocks: 0,
            hdma_active: false,
            hdma_hblank: false,
            cgb_regs: [0; 4],
            serial_active: false,
            serial_cycles: 0,
            serial_waiting: false,
            link_connected: false,
            link_out: None,
        }
    }

    // ---------- liaison série ----------

    /// Branche (ou débranche) cette console sur un câble link relié à une autre console.
    pub fn set_link_connected(&mut self, on: bool) {
        self.link_connected = on;
    }

    /// L'octet qu'un transfert (horloge interne) vient d'envoyer, à remettre à l'autre console. Une seule fois.
    pub fn take_link_out(&mut self) -> Option<u8> {
        self.link_out.take()
    }

    /// Un transfert piloté par cette console se termine : `incoming` est l'octet reçu de l'autre console.
    pub fn link_complete(&mut self, incoming: u8) {
        self.io[0x01] = incoming;
        self.io[0x02] &= !0x80;
        self.io[0x0F] |= 0x08; // interruption série
        self.serial_active = false;
    }

    /// L'autre console pilote le câble et envoie `byte`. Si cette console attend (horloge externe), elle
    /// le reçoit et renvoie l'octet qu'elle avait préparé ; sinon, personne ne répond.
    pub fn link_receive(&mut self, byte: u8) -> Option<u8> {
        if !self.serial_waiting {
            return None;
        }
        let reply = self.io[0x01];
        self.io[0x01] = byte;
        self.io[0x02] &= !0x80;
        self.io[0x0F] |= 0x08;
        self.serial_waiting = false;
        Some(reply)
    }

    /// Avance un transfert série à horloge interne : un bit toutes les 512 T-cycles (8 192 Hz), ou toutes les 16 en mode rapide (Color).
    fn serial_tick(&mut self, cycles: u32) {
        if !self.serial_active || self.link_out.is_some() {
            return;
        }
        self.serial_cycles += cycles;
        let fast = self.cgb && self.io[0x02] & 0x02 != 0;
        let byte_cycles = if fast { 8 * 16 } else { 8 * 512 };
        if self.serial_cycles >= byte_cycles {
            self.serial_cycles = 0;
            let out = self.io[0x01];
            if self.link_connected {
                self.link_out = Some(out); // la réponse viendra de l'autre console
            } else {
                self.link_complete(0xFF); // rien au bout du câble : on reçoit des 1
            }
        }
    }

    pub fn is_cgb(&self) -> bool {
        self.cgb
    }

    pub fn is_double_speed(&self) -> bool {
        self.double_speed
    }

    /// Position dans `wram` d'une adresse de 0xC000 à 0xDFFF (la banque 0xD000–0xDFFF dépend de SVBK en mode Color).
    fn wram_index(&self, addr: u16) -> usize {
        let a = (addr & 0x1FFF) as usize; // 0xC000 -> 0, 0xE000 (écho) -> 0
        if a < 0x1000 {
            a
        } else {
            let bank = if self.cgb { self.svbk.max(1) as usize } else { 1 };
            bank * 0x1000 + (a - 0x1000)
        }
    }

    /// Copie un bloc de 16 octets de la mémoire vers la VRAM (HDMA).
    fn hdma_block(&mut self) {
        let offset = self.ppu.vram_offset();
        for i in 0..16u16 {
            let mut src = self.hdma_src.wrapping_add(i);
            if (0xE000..0xFE00).contains(&src) {
                src -= 0x2000;
            }
            self.dma_reading = true;
            let byte = Bus::read(self, src);
            self.dma_reading = false;
            let dst = offset + ((self.hdma_dst.wrapping_add(i) as usize - 0x8000) & 0x1FFF);
            self.ppu.vram[dst] = byte;
        }
        self.hdma_src = self.hdma_src.wrapping_add(16);
        self.hdma_dst = 0x8000 | (self.hdma_dst.wrapping_add(16) & 0x1FF0);
        if self.hdma_blocks == 0 {
            self.hdma_active = false;
        } else {
            self.hdma_blocks -= 1;
        }
    }

    /// Vrai pendant que l'OAM est occupée par une copie DMA.
    fn dma_busy(&self) -> bool {
        self.dma_pos < DMA_LENGTH && self.dma_wait == 0
    }

    /// Avance la copie DMA : un octet par cycle machine.
    fn dma_tick(&mut self, cycles: u32) {
        if self.dma_pos >= DMA_LENGTH {
            return;
        }
        self.dma_cycles += cycles;
        while self.dma_cycles >= 4 && self.dma_pos < DMA_LENGTH {
            self.dma_cycles -= 4;
            if self.dma_wait > 0 {
                self.dma_wait -= 1;
                continue;
            }
            // les adresses 0xE000 et au-dessus relisent la WRAM (écho)
            let mut src = self.dma_src.wrapping_add(self.dma_pos);
            if src >= 0xE000 {
                src -= 0x2000;
            }
            self.dma_reading = true;
            let byte = Bus::read(self, src);
            self.dma_reading = false;
            self.ppu.oam[self.dma_pos as usize] = byte;
            self.dma_pos += 1;
        }
    }

    /// Met à jour les boutons enfoncés (voir les constantes de `joypad`) et demande l'interruption si besoin.
    pub fn set_buttons(&mut self, state: u8) {
        if self.joypad.set_state(state) {
            self.io[0x0F] |= 0x10;
        }
    }

    /// Fait avancer le temps de `cycles` T-cycles (timer, écran et son).
    pub fn tick(&mut self, cycles: u32) {
        // en vitesse double, le processeur et le timer vont deux fois plus vite : l'écran, le son et la DMA gardent leur rythme
        if self.timer.tick(cycles) {
            self.io[0x0F] |= 0x04; // interruption timer
        }
        let slow = if self.double_speed {
            self.half_acc += cycles;
            let n = self.half_acc / 2;
            self.half_acc %= 2;
            n
        } else {
            cycles
        };
        self.io[0x0F] |= self.ppu.tick(slow);
        if self.cgb && self.hdma_active && self.hdma_hblank && self.ppu.take_hblank() {
            self.hdma_block(); // une copie HDMA par HBlank
        }
        self.apu.tick(slow);
        self.cart.tick(slow);
        self.dma_tick(slow);
        self.serial_tick(cycles);
    }
}

impl Bus for Mmu {
    fn tick(&mut self, cycles: u32) {
        Mmu::tick(self, cycles);
    }

    /// Instruction STOP avec un changement de vitesse demandé : bascule en vitesse double (ou revient).
    fn speed_switch(&mut self) -> bool {
        if self.cgb && self.speed_armed {
            self.speed_armed = false;
            self.double_speed = !self.double_speed;
            self.timer.write(0xFF04, 0); // le diviseur repart de zéro
            true
        } else {
            false
        }
    }

    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cart.read_rom(addr),
            // l'écran verrouille la mémoire vidéo quand il la lit : la VRAM pendant le dessin (mode 3),
            // l'OAM pendant la recherche des sprites et le dessin (modes 2 et 3), et pendant une copie DMA
            0x8000..=0x9FFF => {
                if !self.dma_reading && !self.ppu.vram_accessible() { 0xFF } else { self.ppu.vram[self.ppu.vram_offset() + addr as usize - 0x8000] }
            }
            0xA000..=0xBFFF => self.cart.read_ram(addr),
            0xC000..=0xDFFF => self.wram[self.wram_index(addr)],
            0xE000..=0xFDFF => self.wram[self.wram_index(addr)], // écho de la WRAM
            0xFE00..=0xFE9F => {
                if !self.dma_reading && (!self.ppu.oam_accessible() || self.dma_busy()) { 0xFF } else { self.ppu.oam[addr as usize - 0xFE00] }
            }
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00 => self.joypad.read(),
            0xFF02 => self.io[0x02] | if self.cgb { 0x7C } else { 0x7E }, // bits inutilisés lus à 1
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.io[0x0F] | 0xE0,
            0xFF10..=0xFF3F => self.apu.read(addr),
            // registres de la Game Boy Color
            0xFF4D if self.cgb => 0x7E | (self.double_speed as u8) << 7 | self.speed_armed as u8,
            0xFF4F | 0xFF68..=0xFF6C if self.cgb => self.ppu.read_reg(addr),
            0xFF51..=0xFF54 if self.cgb => 0xFF,
            0xFF55 if self.cgb => if self.hdma_active { self.hdma_blocks } else { 0xFF },
            0xFF70 if self.cgb => 0xF8 | self.svbk,
            0xFF72..=0xFF75 if self.cgb => self.cgb_regs[addr as usize - 0xFF72],
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.read_reg(addr),
            0xFF00..=0xFF7F => self.io[addr as usize - 0xFF00],
            0xFF80..=0xFFFE => self.hram[addr as usize - 0xFF80],
            0xFFFF => self.ie,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7FFF => self.cart.write_rom(addr, value),
            0x8000..=0x9FFF => {
                if self.ppu.vram_accessible() {
                    let i = self.ppu.vram_offset() + addr as usize - 0x8000;
                    self.ppu.vram[i] = value;
                }
            }
            0xA000..=0xBFFF => self.cart.write_ram(addr, value),
            0xC000..=0xDFFF | 0xE000..=0xFDFF => {
                let i = self.wram_index(addr);
                self.wram[i] = value;
            }
            0xFE00..=0xFE9F => {
                if self.ppu.oam_accessible() && !self.dma_busy() {
                    self.ppu.oam[addr as usize - 0xFE00] = value;
                }
            }
            0xFEA0..=0xFEFF => {}
            0xFF00 => self.joypad.write(value),
            0xFF02 => {
                self.io[0x02] = value & if self.cgb { 0x83 } else { 0x81 };
                self.serial_active = false;
                self.serial_waiting = false;
                if value & 0x80 != 0 {
                    if value & 0x01 != 0 {
                        // horloge interne : cette console pilote le câble. L'octet part maintenant (c'est ce que
                        // les ROMs de test écrivent pour « parler »), la réponse arrive à la fin du transfert.
                        self.serial.push(self.io[0x01]);
                        self.serial_active = true;
                        self.serial_cycles = 0;
                    } else {
                        self.serial_waiting = true; // horloge externe : on attend l'autre console
                    }
                }
            }
            0xFF04..=0xFF07 => self.timer.write(addr, value),
            0xFF46 => {
                // DMA : copie 160 octets vers l'OAM, un par cycle machine, après un cycle d'attente
                self.dma_src = (value as u16) << 8;
                self.dma_pos = 0;
                self.dma_wait = 1;
                self.dma_cycles = 0;
                self.io[0x46] = value;
            }
            0xFF10..=0xFF3F => self.apu.write(addr, value),
            // registres de la Game Boy Color
            0xFF4D if self.cgb => self.speed_armed = value & 1 != 0,
            0xFF4F | 0xFF68..=0xFF6C if self.cgb => self.ppu.write_reg(addr, value),
            0xFF51 if self.cgb => self.hdma_src = (self.hdma_src & 0x00FF) | (value as u16) << 8,
            0xFF52 if self.cgb => self.hdma_src = (self.hdma_src & 0xFF00) | (value & 0xF0) as u16,
            0xFF53 if self.cgb => self.hdma_dst = 0x8000 | ((value as u16 & 0x1F) << 8) | (self.hdma_dst & 0x00FF),
            0xFF54 if self.cgb => self.hdma_dst = (self.hdma_dst & 0xFF00) | (value & 0xF0) as u16,
            0xFF55 if self.cgb => {
                if self.hdma_active && self.hdma_hblank && value & 0x80 == 0 {
                    self.hdma_active = false; // une copie HBlank en cours est annulée
                } else {
                    self.hdma_blocks = value & 0x7F;
                    self.hdma_active = true;
                    self.hdma_hblank = value & 0x80 != 0;
                    if !self.hdma_hblank {
                        while self.hdma_active {
                            self.hdma_block(); // copie immédiate : tous les blocs d'un coup
                        }
                    }
                }
            }
            0xFF70 if self.cgb => self.svbk = value & 7,
            0xFF72..=0xFF75 if self.cgb => self.cgb_regs[addr as usize - 0xFF72] = value,
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.write_reg(addr, value),
            0xFF00..=0xFF7F => self.io[addr as usize - 0xFF00] = value,
            0xFF80..=0xFFFE => self.hram[addr as usize - 0xFF80] = value,
            0xFFFF => self.ie = value,
        }
    }
}

impl Mmu {
    pub(crate) fn save_state(&self, w: &mut StateWriter) {
        w.bytes(&self.wram);
        w.bytes(&self.io);
        w.bytes(&self.hram);
        w.u8(self.ie);
        w.u16(self.dma_src);
        w.u16(self.dma_pos);
        w.u32(self.dma_wait);
        w.u32(self.dma_cycles);
        w.u8(self.svbk);
        w.bool(self.double_speed);
        w.bool(self.speed_armed);
        w.u32(self.half_acc);
        w.u16(self.hdma_src);
        w.u16(self.hdma_dst);
        w.u8(self.hdma_blocks);
        w.bool(self.hdma_active);
        w.bool(self.hdma_hblank);
        w.bytes(&self.cgb_regs);
        w.bool(self.serial_active);
        w.u32(self.serial_cycles);
        w.bool(self.serial_waiting);
        w.bool(self.link_out.is_some());
        w.u8(self.link_out.unwrap_or(0));
        w.bytes(&self.serial);
        self.joypad.save_state(w);
        self.timer.save_state(w);
        self.ppu.save_state(w);
        self.apu.save_state(w);
        self.cart.save_state(w);
    }

    pub(crate) fn load_state(&mut self, r: &mut StateReader) -> Result<(), String> {
        r.bytes_into(&mut self.wram)?;
        r.bytes_into(&mut self.io)?;
        r.bytes_into(&mut self.hram)?;
        self.ie = r.u8()?;
        self.dma_src = r.u16()?;
        self.dma_pos = r.u16()?.min(DMA_LENGTH);
        self.dma_wait = r.u32()?;
        self.dma_cycles = r.u32()?;
        self.svbk = r.u8()? & 7;
        self.double_speed = r.bool()?;
        self.speed_armed = r.bool()?;
        self.half_acc = r.u32()? & 1;
        self.hdma_src = r.u16()?;
        self.hdma_dst = 0x8000 | (r.u16()? & 0x1FF0);
        self.hdma_blocks = r.u8()? & 0x7F;
        self.hdma_active = r.bool()?;
        self.hdma_hblank = r.bool()?;
        r.bytes_into(&mut self.cgb_regs)?;
        self.serial_active = r.bool()?;
        self.serial_cycles = r.u32()?;
        self.serial_waiting = r.bool()?;
        let has_out = r.bool()?;
        let out = r.u8()?;
        self.link_out = if has_out { Some(out) } else { None };
        self.serial = r.vec()?; // la liaison série garde un texte de longueur variable
        self.joypad.load_state(r)?;
        self.timer.load_state(r)?;
        self.ppu.load_state(r)?;
        self.apu.load_state(r)?;
        self.cart.load_state(r)
    }
}
