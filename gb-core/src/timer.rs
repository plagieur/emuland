//! Le timer : un compteur que le jeu peut régler pour être prévenu à intervalles réguliers.
//!
//! Un compteur interne de 16 bits avance à chaque T-cycle. Le registre DIV en montre l'octet du haut.
//! TIMA (0xFF05) avance quand un bit choisi de ce compteur passe de 1 à 0, tant que le timer est activé ;
//! à son débordement il se recharge avec TMA (0xFF06) et demande l'interruption timer.

/// Le bit du compteur interne surveillé selon les 2 bits du bas de TAC : 4 096, 262 144, 65 536, 16 384 Hz.
const WATCHED_BIT: [u16; 4] = [9, 3, 5, 7];

/// Le matériel recharge TIMA un cycle machine (4 T-cycles) après son débordement.
const RELOAD_DELAY: u8 = 4;

#[derive(Default)]
pub struct Timer {
    sysclk: u16,
    tima: u8,
    tma: u8,
    tac: u8,
    /// Cycles restants avant le rechargement de TIMA après un débordement (0 = rien en attente).
    reload_in: u8,
}

impl Timer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Le signal dont les chutes font avancer TIMA : le bit surveillé, si le timer est activé.
    fn signal(&self) -> bool {
        self.tac & 0x04 != 0 && (self.sysclk >> WATCHED_BIT[(self.tac & 3) as usize]) & 1 != 0
    }

    fn increment(&mut self) {
        let (value, overflow) = self.tima.overflowing_add(1);
        self.tima = value;
        if overflow {
            // TIMA reste à 0 pendant un cycle machine, puis se recharge
            self.reload_in = RELOAD_DELAY;
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF04 => (self.sysclk >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            _ => 0xF8 | self.tac,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF04 => {
                // écrire dans DIV remet le compteur à zéro, ce qui peut faire tomber le signal et avancer TIMA
                let before = self.signal();
                self.sysclk = 0;
                if before && !self.signal() {
                    self.increment();
                }
            }
            0xFF05 => {
                self.tima = value;
                self.reload_in = 0; // écrire TIMA pendant l'attente annule le rechargement
            }
            0xFF06 => self.tma = value,
            _ => {
                let before = self.signal();
                self.tac = value & 0x07;
                if before && !self.signal() {
                    self.increment();
                }
            }
        }
    }

    /// Avance de `cycles` T-cycles. Renvoie vrai si l'interruption timer doit être demandée.
    pub fn tick(&mut self, cycles: u32) -> bool {
        let mut irq = false;
        for _ in 0..cycles {
            // d'abord le rechargement en attente : il part 4 cycles après le débordement
            if self.reload_in > 0 {
                self.reload_in -= 1;
                if self.reload_in == 0 {
                    self.tima = self.tma;
                    irq = true;
                }
            }
            let before = self.signal();
            self.sysclk = self.sysclk.wrapping_add(1);
            if before && !self.signal() {
                self.increment();
            }
        }
        irq
    }
}

impl Timer {
    pub(crate) fn save_state(&self, w: &mut crate::state::StateWriter) {
        w.u16(self.sysclk);
        w.u8(self.tima);
        w.u8(self.tma);
        w.u8(self.tac);
        w.u8(self.reload_in);
    }

    pub(crate) fn load_state(&mut self, r: &mut crate::state::StateReader) -> Result<(), String> {
        self.sysclk = r.u16()?;
        self.tima = r.u8()?;
        self.tma = r.u8()?;
        self.tac = r.u8()? & 7;
        self.reload_in = r.u8()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    use crate::cartridge::Cartridge;
    use crate::demo::demo_rom;
    use crate::mmu::Mmu;

    #[test]
    fn div_counts_up_every_256_cycles_and_resets_on_write() {
        let mut t = Timer::new();
        t.tick(255);
        assert_eq!(t.read(0xFF04), 0);
        t.tick(1);
        assert_eq!(t.read(0xFF04), 1);
        t.tick(256 * 10);
        assert_eq!(t.read(0xFF04), 11);
        t.write(0xFF04, 0x99); // la valeur écrite n'a aucune importance
        assert_eq!(t.read(0xFF04), 0);
    }

    #[test]
    fn div_wraps_around() {
        let mut t = Timer::new();
        t.tick(256 * 256);
        assert_eq!(t.read(0xFF04), 0);
    }

    #[test]
    fn tima_runs_at_the_four_selectable_speeds() {
        // TAC : activé + vitesse -> cycles par incrément
        for (tac, period) in [(0b100u8, 1024u32), (0b101, 16), (0b110, 64), (0b111, 256)] {
            let mut t = Timer::new();
            t.write(0xFF07, tac);
            t.tick(period * 5);
            assert_eq!(t.read(0xFF05), 5, "TAC {tac:#05b}");
        }
    }

    #[test]
    fn a_disabled_timer_does_not_count_but_div_does() {
        let mut t = Timer::new();
        t.write(0xFF07, 0b001); // vitesse choisie mais pas activé
        t.tick(1000);
        assert_eq!(t.read(0xFF05), 0);
        assert_eq!(t.read(0xFF04), 3);
    }

    #[test]
    fn overflow_reloads_tma_and_raises_the_interrupt_after_a_delay() {
        let mut t = Timer::new();
        t.write(0xFF06, 0xAB);
        t.write(0xFF05, 0xFF);
        t.write(0xFF07, 0b101); // un incrément toutes les 16 cycles
        assert!(!t.tick(15));
        assert!(!t.tick(1)); // débordement : TIMA vaut 0, rechargement en attente
        assert_eq!(t.read(0xFF05), 0x00);
        assert!(!t.tick(3));
        assert!(t.tick(1)); // 4 cycles plus tard : rechargé et interruption
        assert_eq!(t.read(0xFF05), 0xAB);
    }

    #[test]
    fn writing_tima_during_the_delay_cancels_the_reload() {
        let mut t = Timer::new();
        t.write(0xFF06, 0xAB);
        t.write(0xFF05, 0xFF);
        t.write(0xFF07, 0b101);
        t.tick(16);
        t.write(0xFF05, 0x42);
        assert!(!t.tick(8));
        assert_eq!(t.read(0xFF05), 0x42);
    }

    #[test]
    fn writing_div_can_cause_an_extra_tick() {
        let mut t = Timer::new();
        t.write(0xFF07, 0b101); // surveille le bit 3
        t.tick(8); // le bit 3 passe à 1
        assert_eq!(t.read(0xFF05), 0);
        t.write(0xFF04, 0); // le compteur retombe à 0 : le signal chute
        assert_eq!(t.read(0xFF05), 1);
    }

    #[test]
    fn tac_reads_back_with_unused_bits_set() {
        let mut t = Timer::new();
        t.write(0xFF07, 0xFF);
        assert_eq!(t.read(0xFF07), 0xFF);
        t.write(0xFF07, 0x05);
        assert_eq!(t.read(0xFF07), 0xFD);
    }

    #[test]
    fn registers_are_reachable_through_the_memory_map_and_raise_if() {
        let mut mmu = Mmu::new(Cartridge::new(demo_rom()).unwrap());
        mmu.write(0xFF06, 0x10);
        mmu.write(0xFF05, 0xFF);
        mmu.write(0xFF07, 0b101);
        mmu.write(0xFF0F, 0x00);
        mmu.tick(16 + 4);
        assert_eq!(mmu.read(0xFF05), 0x10);
        assert_eq!(mmu.read(0xFF0F) & 0x04, 0x04, "l'interruption timer est demandée dans IF");
    }
}
