//! Le câble link : deux consoles reliées par leur port série, comme pour échanger des Pokémon ou se battre.
//!
//! Sur le câble, une console « pilote » (elle a choisi son horloge interne) et envoie un octet pendant que
//! l'autre (horloge externe) en renvoie un en même temps. Chaque console exécute son propre jeu ;
//! `LinkedPair` les fait avancer ensemble et se charge de l'échange des octets.

use crate::gameboy::{GameBoy, CYCLES_PER_FRAME};

/// Remet à chaque console l'octet envoyé par l'autre quand un transfert se termine.
pub fn exchange(a: &mut GameBoy, b: &mut GameBoy) {
    if let Some(byte) = a.mmu.take_link_out() {
        let reply = b.mmu.link_receive(byte).unwrap_or(0xFF); // personne n'attend : on reçoit des 1
        a.mmu.link_complete(reply);
    }
    if let Some(byte) = b.mmu.take_link_out() {
        let reply = a.mmu.link_receive(byte).unwrap_or(0xFF);
        b.mmu.link_complete(reply);
    }
}

/// Fait avancer d'une instruction la console qui a le moins avancé, puis échange les octets du câble.
pub fn step_pair(a: &mut GameBoy, b: &mut GameBoy) {
    if a.total_cycles <= b.total_cycles {
        a.step();
    } else {
        b.step();
    }
    exchange(a, b);
}

/// Fait avancer deux consoles reliées jusqu'à ce que chacune ait terminé une image.
pub fn run_frame_pair(a: &mut GameBoy, b: &mut GameBoy) {
    let (start_a, start_b) = (a.total_cycles, b.total_cycles);
    let (mut a_done, mut b_done) = (false, false);
    let limit = CYCLES_PER_FRAME as u64 * 4;
    while !(a_done && b_done) && a.total_cycles - start_a < limit && b.total_cycles - start_b < limit {
        step_pair(a, b);
        a_done |= a.mmu.ppu.take_frame_ready() || (!a.mmu.ppu.lcd_on() && a.total_cycles - start_a >= CYCLES_PER_FRAME as u64);
        b_done |= b.mmu.ppu.take_frame_ready() || (!b.mmu.ppu.lcd_on() && b.total_cycles - start_b >= CYCLES_PER_FRAME as u64);
    }
}

pub struct LinkedPair {
    pub a: GameBoy,
    pub b: GameBoy,
}

impl LinkedPair {
    pub fn new(mut a: GameBoy, mut b: GameBoy) -> Self {
        a.mmu.set_link_connected(true);
        b.mmu.set_link_connected(true);
        Self { a, b }
    }

    pub fn step(&mut self) {
        step_pair(&mut self.a, &mut self.b);
    }

    /// Fait avancer les deux consoles jusqu'à ce que chacune ait terminé une image.
    pub fn run_frame(&mut self) {
        run_frame_pair(&mut self.a, &mut self.b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    /// Une ROM de 32 Kio qui exécute `code` à partir de 0x0150.
    fn rom(code: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x103].copy_from_slice(&[0xC3, 0x50, 0x01]); // JP 0x0150
        rom[0x150..0x150 + code.len()].copy_from_slice(code);
        rom
    }

    /// Envoie `byte` avec l'horloge interne (ou attend avec l'externe), puis range l'octet reçu en 0xC000.
    fn program(byte: u8, master: bool) -> Vec<u8> {
        vec![
            0x3E, byte, //       LD A,byte
            0xE0, 0x01, //       LDH (SB),A
            0x3E, if master { 0x81 } else { 0x80 }, // LD A,start (+ horloge interne pour le maître)
            0xE0, 0x02, //       LDH (SC),A
            0xF0, 0x02, //       attente : LDH A,(SC)
            0xCB, 0x7F, //       BIT 7,A
            0x20, 0xFA, //       JR NZ,attente
            0xF0, 0x01, //       LDH A,(SB) : l'octet reçu
            0xEA, 0x00, 0xC0, // LD (0xC000),A
            0x18, 0xFE, //       boucle infinie
        ]
    }

    fn console(byte: u8, master: bool) -> GameBoy {
        GameBoy::new(rom(&program(byte, master))).unwrap()
    }

    #[test]
    fn two_linked_consoles_swap_their_bytes() {
        let mut pair = LinkedPair::new(console(0x42, true), console(0x99, false));
        for _ in 0..3 {
            pair.run_frame();
            let _ = (pair.a.take_audio(), pair.b.take_audio());
        }
        assert_eq!(pair.a.mmu.read(0xC000), 0x99, "le maître reçoit l'octet de l'esclave");
        assert_eq!(pair.b.mmu.read(0xC000), 0x42, "l'esclave reçoit l'octet du maître");
    }

    #[test]
    fn the_transfer_takes_about_four_thousand_cycles() {
        let mut master = console(0x42, true);
        let mut cycles = 0;
        while cycles < 3000 {
            cycles += master.step(); // le transfert n'est pas encore fini
        }
        assert_eq!(master.mmu.read(0xFF02) & 0x80, 0x80);
        while cycles < 5000 {
            cycles += master.step();
        }
        assert_eq!(master.mmu.read(0xFF02) & 0x80, 0, "8 bits à 8 192 Hz = 4 096 cycles : c'est fini");
    }

    #[test]
    fn a_console_alone_on_the_cable_receives_ones() {
        let mut alone = console(0x42, true);
        for _ in 0..2 {
            alone.run_frame();
        }
        assert_eq!(alone.mmu.read(0xC000), 0xFF);
    }

    #[test]
    fn serial_interrupt_is_requested_when_the_transfer_ends() {
        let mut master = console(0x42, true);
        let mut cycles = 0;
        while cycles < 6000 {
            cycles += master.step();
        }
        assert_eq!(master.mmu.read(0xFF0F) & 0x08, 0x08);
    }

    #[test]
    fn both_consoles_can_be_driven_from_a_saved_state() {
        let mut pair = LinkedPair::new(console(0x42, true), console(0x99, false));
        pair.run_frame();
        let state = pair.a.save_state();
        pair.a.load_state(&state).unwrap();
        for _ in 0..2 {
            pair.run_frame();
        }
        assert_eq!(pair.a.mmu.read(0xC000), 0x99);
    }
}
