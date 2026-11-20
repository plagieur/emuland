//! Le processeur de la Game Boy : un Sharp LR35902 (proche d'un Z80, en plus simple).
//!
//! `Cpu::step` exécute UNE instruction (ou une interruption) et renvoie le nombre de
//! cycles machine consommés, exprimé en « T-cycles » (4 T-cycles = 1 cycle machine).
//! C'est ce nombre qui servira plus tard à synchroniser l'écran et le son.

use crate::bus::Bus;
use crate::state::{StateReader, StateWriter};

pub const FLAG_Z: u8 = 0x80; // résultat nul
pub const FLAG_N: u8 = 0x40; // dernière opération = soustraction
pub const FLAG_H: u8 = 0x20; // retenue du demi-octet (bit 3 -> 4)
pub const FLAG_C: u8 = 0x10; // retenue

const IE: u16 = 0xFFFF; // interruptions autorisées
const IF: u16 = 0xFF0F; // interruptions demandées

#[derive(Debug, Clone)]
pub struct Cpu {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    /// Interrupt Master Enable : interrupteur général des interruptions.
    pub ime: bool,
    pub halted: bool,
    /// Opcode illégal rencontré : le vrai processeur se fige, nous aussi.
    pub locked: bool,
    /// `EI` n'active les interruptions qu'après l'instruction suivante.
    ei_pending: bool,
    /// Après un `HALT` exécuté avec IME coupé alors qu'une interruption est déjà en attente, le processeur
    /// ne se met pas en pause et relit deux fois l'octet suivant (le fameux « bug du HALT »).
    halt_bug: bool,
    /// `STOP` : le processeur dort jusqu'à ce qu'on appuie sur une touche.
    stopped: bool,
    /// T-cycles déjà écoulés (et transmis au bus) dans l'instruction en cours.
    ticked: u32,
}

impl Cpu {
    /// État des registres juste après la ROM de démarrage d'une Game Boy classique.
    pub fn new() -> Self {
        Self {
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
            ime: false,
            halted: false,
            locked: false,
            ei_pending: false,
            halt_bug: false,
            stopped: false,
            ticked: 0,
        }
    }

    /// État des registres juste après la ROM de démarrage d'une Game Boy Color : A vaut 0x11, ce qui permet
    /// à un jeu de savoir qu'il tourne sur une Color.
    pub fn new_cgb() -> Self {
        Self { a: 0x11, f: 0x80, b: 0x00, c: 0x00, d: 0xFF, e: 0x56, h: 0x00, l: 0x0D, ..Self::new() }
    }

    // ---------- registres 16 bits ----------
    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f])
    }
    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }
    pub fn set_af(&mut self, v: u16) {
        let [a, f] = v.to_be_bytes();
        self.a = a;
        self.f = f & 0xF0; // les 4 bits bas de F n'existent pas
    }
    pub fn set_bc(&mut self, v: u16) {
        [self.b, self.c] = v.to_be_bytes();
    }
    pub fn set_de(&mut self, v: u16) {
        [self.d, self.e] = v.to_be_bytes();
    }
    pub fn set_hl(&mut self, v: u16) {
        [self.h, self.l] = v.to_be_bytes();
    }

    /// Paire de registres désignée par 2 bits d'opcode : BC, DE, HL, SP.
    fn get_rr(&self, i: u8) -> u16 {
        match i & 3 {
            0 => self.bc(),
            1 => self.de(),
            2 => self.hl(),
            _ => self.sp,
        }
    }
    fn set_rr(&mut self, i: u8, v: u16) {
        match i & 3 {
            0 => self.set_bc(v),
            1 => self.set_de(v),
            2 => self.set_hl(v),
            _ => self.sp = v,
        }
    }

    // ---------- drapeaux ----------
    fn flag(&self, mask: u8) -> bool {
        self.f & mask != 0
    }
    fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.f |= mask;
        } else {
            self.f &= !mask;
        }
    }
    fn set_flags(&mut self, z: bool, n: bool, h: bool, c: bool) {
        self.f = (z as u8) << 7 | (n as u8) << 6 | (h as u8) << 5 | (c as u8) << 4;
    }

    // ---------- registres 8 bits désignés par 3 bits d'opcode : B C D E H L (HL) A ----------
    fn get_r<B: Bus>(&mut self, bus: &mut B, i: u8) -> u8 {
        match i & 7 {
            0 => self.b,
            1 => self.c,
            2 => self.d,
            3 => self.e,
            4 => self.h,
            5 => self.l,
            6 => self.rd(bus,self.hl()),
            _ => self.a,
        }
    }
    fn set_r<B: Bus>(&mut self, bus: &mut B, i: u8, v: u8) {
        match i & 7 {
            0 => self.b = v,
            1 => self.c = v,
            2 => self.d = v,
            3 => self.e = v,
            4 => self.h = v,
            5 => self.l = v,
            6 => self.wr(bus,self.hl(), v),
            _ => self.a = v,
        }
    }

    // ---------- accès à la mémoire, cycle machine par cycle machine ----------
    //
    // Chaque lecture ou écriture dure 4 T-cycles pendant lesquels le reste de la console (timer, écran, son)
    // avance. C'est ce qui permet à un jeu de lire le timer ou l'écran « au bon moment » au milieu d'une instruction.

    /// Un cycle machine sans accès à la mémoire (calcul interne du processeur).
    fn internal<B: Bus>(&mut self, bus: &mut B) {
        bus.tick(4);
        self.ticked += 4;
    }

    fn rd<B: Bus>(&mut self, bus: &mut B, addr: u16) -> u8 {
        let v = bus.read(addr);
        bus.tick(4);
        self.ticked += 4;
        v
    }

    fn wr<B: Bus>(&mut self, bus: &mut B, addr: u16, v: u8) {
        bus.write(addr, v);
        bus.tick(4);
        self.ticked += 4;
    }

    // ---------- lecture de l'instruction et pile ----------
    fn fetch8<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let v = self.rd(bus,self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }
    fn fetch16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.fetch8(bus);
        let hi = self.fetch8(bus);
        u16::from_le_bytes([lo, hi])
    }
    fn push16<B: Bus>(&mut self, bus: &mut B, v: u16) {
        self.internal(bus); // PUSH, CALL et RST commencent par un cycle de calcul avant d'écrire
        let [lo, hi] = v.to_le_bytes();
        self.sp = self.sp.wrapping_sub(1);
        self.wr(bus,self.sp, hi);
        self.sp = self.sp.wrapping_sub(1);
        self.wr(bus,self.sp, lo);
    }
    fn pop16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.rd(bus,self.sp);
        self.sp = self.sp.wrapping_add(1);
        let hi = self.rd(bus,self.sp);
        self.sp = self.sp.wrapping_add(1);
        u16::from_le_bytes([lo, hi])
    }

    /// Condition d'un saut : 0 = NZ, 1 = Z, 2 = NC, 3 = C.
    fn cond(&self, i: u8) -> bool {
        match i & 3 {
            0 => !self.flag(FLAG_Z),
            1 => self.flag(FLAG_Z),
            2 => !self.flag(FLAG_C),
            _ => self.flag(FLAG_C),
        }
    }

    // ---------- arithmétique ----------
    fn add8(&mut self, v: u8, with_carry: bool) {
        let c = (with_carry && self.flag(FLAG_C)) as u8;
        let a = self.a;
        let res = a as u16 + v as u16 + c as u16;
        self.set_flags(res & 0xFF == 0, false, (a & 0xF) + (v & 0xF) + c > 0xF, res > 0xFF);
        self.a = res as u8;
    }

    /// Soustraction ; `store = false` pour CP (compare sans modifier A).
    fn sub8(&mut self, v: u8, with_carry: bool, store: bool) {
        let c = (with_carry && self.flag(FLAG_C)) as i16;
        let a = self.a;
        let res = a as i16 - v as i16 - c;
        let half = (a & 0xF) as i16 - (v & 0xF) as i16 - c < 0;
        self.set_flags(res as u8 == 0, true, half, res < 0);
        if store {
            self.a = res as u8;
        }
    }

    /// Les 8 opérations arithmétiques et logiques, dans l'ordre de l'opcode.
    fn alu(&mut self, op: u8, v: u8) {
        match op & 7 {
            0 => self.add8(v, false),
            1 => self.add8(v, true),
            2 => self.sub8(v, false, true),
            3 => self.sub8(v, true, true),
            4 => {
                self.a &= v;
                self.set_flags(self.a == 0, false, true, false);
            }
            5 => {
                self.a ^= v;
                self.set_flags(self.a == 0, false, false, false);
            }
            6 => {
                self.a |= v;
                self.set_flags(self.a == 0, false, false, false);
            }
            _ => self.sub8(v, false, false),
        }
    }

    fn inc8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.set_flag(FLAG_Z, r == 0);
        self.set_flag(FLAG_N, false);
        self.set_flag(FLAG_H, v & 0xF == 0xF);
        r
    }
    fn dec8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.set_flag(FLAG_Z, r == 0);
        self.set_flag(FLAG_N, true);
        self.set_flag(FLAG_H, v & 0xF == 0);
        r
    }

    fn add_hl(&mut self, v: u16) {
        let hl = self.hl();
        let res = hl as u32 + v as u32;
        self.set_flag(FLAG_N, false);
        self.set_flag(FLAG_H, (hl & 0xFFF) + (v & 0xFFF) > 0xFFF);
        self.set_flag(FLAG_C, res > 0xFFFF);
        self.set_hl(res as u16);
    }

    /// SP + entier signé de 8 bits : sert à `ADD SP,e8` et `LD HL,SP+e8`.
    fn sp_plus_e8<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let e = self.fetch8(bus) as i8 as i16 as u16;
        let sp = self.sp;
        self.set_flags(false, false, (sp & 0xF) + (e & 0xF) > 0xF, (sp & 0xFF) + (e & 0xFF) > 0xFF);
        sp.wrapping_add(e)
    }

    /// Ajustement décimal après une addition ou soustraction de nombres BCD.
    fn daa(&mut self) {
        let mut a = self.a;
        let mut carry = self.flag(FLAG_C);
        if !self.flag(FLAG_N) {
            let mut adjust = 0;
            if self.flag(FLAG_H) || a & 0xF > 9 {
                adjust |= 0x06;
            }
            if carry || a > 0x99 {
                adjust |= 0x60;
                carry = true;
            }
            a = a.wrapping_add(adjust);
        } else {
            let mut adjust = 0;
            if self.flag(FLAG_H) {
                adjust |= 0x06;
            }
            if carry {
                adjust |= 0x60;
            }
            a = a.wrapping_sub(adjust);
        }
        self.a = a;
        self.set_flag(FLAG_Z, a == 0);
        self.set_flag(FLAG_H, false);
        self.set_flag(FLAG_C, carry);
    }

    /// Rotations et décalages : 0 RLC, 1 RRC, 2 RL, 3 RR, 4 SLA, 5 SRA, 6 SWAP, 7 SRL.
    fn rot_shift(&mut self, op: u8, v: u8) -> u8 {
        let old_c = self.flag(FLAG_C) as u8;
        let (res, carry) = match op & 7 {
            0 => (v.rotate_left(1), v >> 7),
            1 => (v.rotate_right(1), v & 1),
            2 => ((v << 1) | old_c, v >> 7),
            3 => ((v >> 1) | (old_c << 7), v & 1),
            4 => (v << 1, v >> 7),
            5 => ((v >> 1) | (v & 0x80), v & 1),
            6 => ((v << 4) | (v >> 4), 0),
            _ => (v >> 1, v & 1),
        };
        self.set_flags(res == 0, false, false, carry != 0);
        res
    }

    // ---------- exécution ----------

    /// Exécute une instruction (ou déclenche une interruption) et renvoie les T-cycles utilisés.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> u32 {
        self.ticked = 0;
        if self.locked {
            bus.tick(4);
            return 4;
        }
        let pending = bus.read(IE) & bus.read(IF) & 0x1F;
        if self.stopped {
            if bus.read(0xFF00) & 0x0F == 0x0F {
                bus.tick(4);
                return 4; // toujours endormi
            }
            self.stopped = false; // une touche réveille le processeur
        }
        if self.halted {
            if pending == 0 {
                bus.tick(4);
                return 4;
            }
            self.halted = false; // une interruption réveille le CPU même si IME est coupé
        }
        if self.ime && pending != 0 {
            // l'interruption dure 5 cycles machine : 2 d'attente, 2 écritures (PC sur la pile), 1 pour sauter
            self.ime = false;
            self.ei_pending = false;
            self.internal(bus);
            self.internal(bus);
            let [lo, hi] = self.pc.to_le_bytes();
            self.sp = self.sp.wrapping_sub(1);
            self.wr(bus, self.sp, hi);
            // le vecteur se décide APRÈS l'écriture de l'octet haut : si l'interruption a disparu entre-temps
            // (la pile a pu écraser IE ou IF), le processeur saute en 0x0000
            let still = bus.read(IE) & bus.read(IF) & 0x1F;
            self.sp = self.sp.wrapping_sub(1);
            self.wr(bus, self.sp, lo);
            if still == 0 {
                self.pc = 0;
            } else {
                let bit = still.trailing_zeros() as u16;
                let iff = bus.read(IF);
                bus.write(IF, iff & !(1 << bit));
                self.pc = 0x40 + 8 * bit;
            }
            self.internal(bus);
            return 20;
        }
        let enable_ime = self.ei_pending;
        self.ei_pending = false;
        let op = if self.halt_bug {
            // bug du HALT : l'octet suivant est lu sans que PC avance, donc exécuté deux fois
            self.halt_bug = false;
            self.rd(bus, self.pc)
        } else {
            self.fetch8(bus)
        };
        let cycles = self.exec(bus, op);
        if cycles > self.ticked {
            bus.tick(cycles - self.ticked); // les cycles internes restants de l'instruction
        }
        if enable_ime && op != 0xF3 {
            self.ime = true;
        }
        cycles
    }

    fn exec<B: Bus>(&mut self, bus: &mut B, op: u8) -> u32 {
        match op {
            0x00 => 4,
            0x10 => {
                self.fetch8(bus); // STOP est suivi d'un octet ignoré
                if bus.speed_switch() {
                    // changement de vitesse (Game Boy Color) : le processeur reprend aussitôt
                } else if bus.read(0xFF00) & 0x0F == 0x0F {
                    self.stopped = true; // aucune touche enfoncée : on dort
                }
                4
            }
            0x76 => {
                let pending = bus.read(IE) & bus.read(IF) & 0x1F;
                if !self.ime && pending != 0 {
                    self.halt_bug = true; // HALT sans effet, mais l'octet suivant sera lu deux fois
                } else {
                    self.halted = true;
                }
                4
            }
            0xCB => self.exec_cb(bus),

            // LD r,r'
            0x40..=0x7F => {
                let v = self.get_r(bus, op & 7);
                self.set_r(bus, (op >> 3) & 7, v);
                if op & 7 == 6 || (op >> 3) & 7 == 6 {
                    8
                } else {
                    4
                }
            }
            // ADD / ADC / SUB / SBC / AND / XOR / OR / CP avec un registre
            0x80..=0xBF => {
                let v = self.get_r(bus, op & 7);
                self.alu((op >> 3) & 7, v);
                if op & 7 == 6 {
                    8
                } else {
                    4
                }
            }

            // chargements 16 bits et via adresse
            0x01 | 0x11 | 0x21 | 0x31 => {
                let v = self.fetch16(bus);
                self.set_rr(op >> 4, v);
                12
            }
            0x02 => {
                self.wr(bus,self.bc(), self.a);
                8
            }
            0x12 => {
                self.wr(bus,self.de(), self.a);
                8
            }
            0x22 => {
                let hl = self.hl();
                self.wr(bus,hl, self.a);
                self.set_hl(hl.wrapping_add(1));
                8
            }
            0x32 => {
                let hl = self.hl();
                self.wr(bus,hl, self.a);
                self.set_hl(hl.wrapping_sub(1));
                8
            }
            0x0A => {
                self.a = self.rd(bus,self.bc());
                8
            }
            0x1A => {
                self.a = self.rd(bus,self.de());
                8
            }
            0x2A => {
                let hl = self.hl();
                self.a = self.rd(bus,hl);
                self.set_hl(hl.wrapping_add(1));
                8
            }
            0x3A => {
                let hl = self.hl();
                self.a = self.rd(bus,hl);
                self.set_hl(hl.wrapping_sub(1));
                8
            }
            0x08 => {
                let addr = self.fetch16(bus);
                let [lo, hi] = self.sp.to_le_bytes();
                self.wr(bus,addr, lo);
                self.wr(bus,addr.wrapping_add(1), hi);
                20
            }
            0xE0 => {
                let n = self.fetch8(bus);
                self.wr(bus,0xFF00 | n as u16, self.a);
                12
            }
            0xF0 => {
                let n = self.fetch8(bus);
                self.a = self.rd(bus,0xFF00 | n as u16);
                12
            }
            0xE2 => {
                self.wr(bus,0xFF00 | self.c as u16, self.a);
                8
            }
            0xF2 => {
                self.a = self.rd(bus,0xFF00 | self.c as u16);
                8
            }
            0xEA => {
                let addr = self.fetch16(bus);
                self.wr(bus,addr, self.a);
                16
            }
            0xFA => {
                let addr = self.fetch16(bus);
                self.a = self.rd(bus,addr);
                16
            }
            0xF9 => {
                self.sp = self.hl();
                8
            }
            0xE8 => {
                self.sp = self.sp_plus_e8(bus);
                16
            }
            0xF8 => {
                let v = self.sp_plus_e8(bus);
                self.set_hl(v);
                12
            }

            // INC / DEC 16 bits, ADD HL
            0x03 | 0x13 | 0x23 | 0x33 => {
                let i = op >> 4;
                self.set_rr(i, self.get_rr(i).wrapping_add(1));
                8
            }
            0x0B | 0x1B | 0x2B | 0x3B => {
                let i = op >> 4;
                self.set_rr(i, self.get_rr(i).wrapping_sub(1));
                8
            }
            0x09 | 0x19 | 0x29 | 0x39 => {
                let v = self.get_rr(op >> 4);
                self.add_hl(v);
                8
            }

            // INC r / DEC r / LD r,d8
            x if x & 0xC7 == 0x04 => {
                let r = (x >> 3) & 7;
                let v = self.get_r(bus, r);
                let res = self.inc8(v);
                self.set_r(bus, r, res);
                if r == 6 {
                    12
                } else {
                    4
                }
            }
            x if x & 0xC7 == 0x05 => {
                let r = (x >> 3) & 7;
                let v = self.get_r(bus, r);
                let res = self.dec8(v);
                self.set_r(bus, r, res);
                if r == 6 {
                    12
                } else {
                    4
                }
            }
            x if x & 0xC7 == 0x06 => {
                let r = (x >> 3) & 7;
                let v = self.fetch8(bus);
                self.set_r(bus, r, v);
                if r == 6 {
                    12
                } else {
                    8
                }
            }

            // rotations de A, ajustements
            0x07 | 0x0F | 0x17 | 0x1F => {
                let a = self.a;
                self.a = self.rot_shift(op >> 3, a);
                self.set_flag(FLAG_Z, false);
                4
            }
            0x27 => {
                self.daa();
                4
            }
            0x2F => {
                self.a = !self.a;
                self.set_flag(FLAG_N, true);
                self.set_flag(FLAG_H, true);
                4
            }
            0x37 => {
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, false);
                self.set_flag(FLAG_C, true);
                4
            }
            0x3F => {
                let c = self.flag(FLAG_C);
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, false);
                self.set_flag(FLAG_C, !c);
                4
            }

            // sauts relatifs
            0x18 => {
                let e = self.fetch8(bus) as i8;
                self.pc = self.pc.wrapping_add(e as u16);
                12
            }
            x if x & 0xE7 == 0x20 => {
                let e = self.fetch8(bus) as i8;
                if self.cond(x >> 3) {
                    self.pc = self.pc.wrapping_add(e as u16);
                    12
                } else {
                    8
                }
            }

            // sauts, appels, retours
            0xC3 => {
                self.pc = self.fetch16(bus);
                16
            }
            0xE9 => {
                self.pc = self.hl();
                4
            }
            x if x & 0xE7 == 0xC2 => {
                let addr = self.fetch16(bus);
                if self.cond(x >> 3) {
                    self.pc = addr;
                    16
                } else {
                    12
                }
            }
            0xCD => {
                let addr = self.fetch16(bus);
                let pc = self.pc;
                self.push16(bus, pc);
                self.pc = addr;
                24
            }
            x if x & 0xE7 == 0xC4 => {
                let addr = self.fetch16(bus);
                if self.cond(x >> 3) {
                    let pc = self.pc;
                    self.push16(bus, pc);
                    self.pc = addr;
                    24
                } else {
                    12
                }
            }
            0xC9 => {
                self.pc = self.pop16(bus);
                16
            }
            0xD9 => {
                self.pc = self.pop16(bus);
                self.ime = true;
                16
            }
            x if x & 0xE7 == 0xC0 => {
                if self.cond(x >> 3) {
                    self.pc = self.pop16(bus);
                    20
                } else {
                    8
                }
            }
            x if x & 0xC7 == 0xC7 => {
                let pc = self.pc;
                self.push16(bus, pc);
                self.pc = (x & 0x38) as u16;
                16
            }

            // pile
            x if x & 0xCF == 0xC1 => {
                let v = self.pop16(bus);
                match x >> 4 & 3 {
                    3 => self.set_af(v),
                    i => self.set_rr(i, v),
                }
                12
            }
            x if x & 0xCF == 0xC5 => {
                let v = match x >> 4 & 3 {
                    3 => self.af(),
                    i => self.get_rr(i),
                };
                self.push16(bus, v);
                16
            }

            // opérations avec une valeur immédiate
            x if x & 0xC7 == 0xC6 => {
                let v = self.fetch8(bus);
                self.alu((x >> 3) & 7, v);
                8
            }

            // interruptions
            0xF3 => {
                self.ime = false;
                self.ei_pending = false;
                4
            }
            0xFB => {
                self.ei_pending = true;
                4
            }

            // opcodes inexistants : le processeur se fige
            _ => {
                self.locked = true;
                4
            }
        }
    }

    /// Instructions préfixées par 0xCB : rotations, décalages, BIT / RES / SET.
    fn exec_cb<B: Bus>(&mut self, bus: &mut B) -> u32 {
        let op = self.fetch8(bus);
        let r = op & 7;
        let bit = (op >> 3) & 7;
        let v = self.get_r(bus, r);
        match op >> 6 {
            0 => {
                let res = self.rot_shift(bit, v);
                self.set_r(bus, r, res);
                if r == 6 {
                    16
                } else {
                    8
                }
            }
            1 => {
                self.set_flag(FLAG_Z, v & (1 << bit) == 0);
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, true);
                if r == 6 {
                    12
                } else {
                    8
                }
            }
            2 => {
                self.set_r(bus, r, v & !(1 << bit));
                if r == 6 {
                    16
                } else {
                    8
                }
            }
            _ => {
                self.set_r(bus, r, v | (1 << bit));
                if r == 6 {
                    16
                } else {
                    8
                }
            }
        }
    }
}

impl Cpu {
    pub(crate) fn save_state(&self, w: &mut StateWriter) {
        for v in [self.a, self.f, self.b, self.c, self.d, self.e, self.h, self.l] {
            w.u8(v);
        }
        w.u16(self.sp);
        w.u16(self.pc);
        w.bool(self.ime);
        w.bool(self.halted);
        w.bool(self.locked);
        w.bool(self.ei_pending);
        w.bool(self.halt_bug);
        w.bool(self.stopped);
    }

    pub(crate) fn load_state(&mut self, r: &mut StateReader) -> Result<(), String> {
        self.a = r.u8()?;
        self.f = r.u8()? & 0xF0;
        self.b = r.u8()?;
        self.c = r.u8()?;
        self.d = r.u8()?;
        self.e = r.u8()?;
        self.h = r.u8()?;
        self.l = r.u8()?;
        self.sp = r.u16()?;
        self.pc = r.u16()?;
        self.ime = r.bool()?;
        self.halted = r.bool()?;
        self.locked = r.bool()?;
        self.ei_pending = r.bool()?;
        self.halt_bug = r.bool()?;
        self.stopped = r.bool()?;
        self.ticked = 0;
        Ok(())
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatRam;

    /// CPU prêt à exécuter `prog` placé à l'adresse 0, drapeaux et registres à zéro.
    fn setup(prog: &[u8]) -> (Cpu, FlatRam) {
        let mut ram = FlatRam::new();
        ram.load(0, prog);
        let mut cpu = Cpu::new();
        cpu.pc = 0;
        cpu.f = 0;
        cpu.a = 0;
        cpu.set_bc(0);
        cpu.set_de(0);
        cpu.set_hl(0);
        (cpu, ram)
    }

    #[test]
    fn add_sets_zero_half_and_carry() {
        let (mut cpu, mut ram) = setup(&[0xC6, 0xC6]); // ADD A,0xC6
        cpu.a = 0x3A;
        assert_eq!(cpu.step(&mut ram), 8);
        assert_eq!(cpu.a, 0x00);
        assert_eq!(cpu.f, FLAG_Z | FLAG_H | FLAG_C);
    }

    #[test]
    fn adc_uses_the_carry() {
        let (mut cpu, mut ram) = setup(&[0xCE, 0x0F]); // ADC A,0x0F
        cpu.a = 0xE1;
        cpu.f = FLAG_C;
        cpu.step(&mut ram);
        assert_eq!(cpu.a, 0xF1);
        assert_eq!(cpu.f, FLAG_H);
    }

    #[test]
    fn sub_to_zero_and_with_borrow() {
        let (mut cpu, mut ram) = setup(&[0xD6, 0x3E, 0xD6, 0x40]); // SUB 0x3E ; SUB 0x40
        cpu.a = 0x3E;
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x00, FLAG_Z | FLAG_N));
        cpu.a = 0x3E;
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0xFE, FLAG_N | FLAG_C));
    }

    #[test]
    fn sbc_uses_the_carry() {
        let (mut cpu, mut ram) = setup(&[0xDE, 0x4F]); // SBC A,0x4F
        cpu.a = 0x3B;
        cpu.f = FLAG_C;
        cpu.step(&mut ram);
        assert_eq!(cpu.a, 0xEB);
        assert_eq!(cpu.f, FLAG_N | FLAG_H | FLAG_C);
    }

    #[test]
    fn cp_compares_without_changing_a() {
        let (mut cpu, mut ram) = setup(&[0xFE, 0x2F]); // CP 0x2F
        cpu.a = 0x3C;
        cpu.step(&mut ram);
        assert_eq!(cpu.a, 0x3C);
        assert_eq!(cpu.f, FLAG_N | FLAG_H);
    }

    #[test]
    fn logic_operations_flags() {
        let (mut cpu, mut ram) = setup(&[0xE6, 0x3F, 0xAF, 0xF6, 0x00]); // AND 0x3F ; XOR A ; OR 0
        cpu.a = 0x5A;
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x1A, FLAG_H));
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x00, FLAG_Z));
        cpu.a = 0x5A;
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x5A, 0));
    }

    #[test]
    fn inc_dec_keep_the_carry() {
        let (mut cpu, mut ram) = setup(&[0x04, 0x05, 0x05]); // INC B ; DEC B ; DEC B
        cpu.b = 0xFF;
        cpu.f = FLAG_C;
        assert_eq!(cpu.step(&mut ram), 4);
        assert_eq!((cpu.b, cpu.f), (0x00, FLAG_Z | FLAG_H | FLAG_C));
        cpu.step(&mut ram); // 0x00 - 1
        assert_eq!((cpu.b, cpu.f), (0xFF, FLAG_N | FLAG_H | FLAG_C));
        cpu.b = 0x01;
        cpu.step(&mut ram);
        assert_eq!((cpu.b, cpu.f), (0x00, FLAG_Z | FLAG_N | FLAG_C));
    }

    #[test]
    fn inc_at_hl_takes_12_cycles() {
        let (mut cpu, mut ram) = setup(&[0x34]); // INC (HL)
        cpu.set_hl(0xC000);
        ram.mem[0xC000] = 0x0F;
        assert_eq!(cpu.step(&mut ram), 12);
        assert_eq!(ram.mem[0xC000], 0x10);
        assert_eq!(cpu.f, FLAG_H);
    }

    #[test]
    fn add_hl_flags() {
        let (mut cpu, mut ram) = setup(&[0x09, 0x29]); // ADD HL,BC ; ADD HL,HL
        cpu.set_hl(0x8A23);
        cpu.set_bc(0x0605);
        cpu.f = FLAG_Z;
        assert_eq!(cpu.step(&mut ram), 8);
        assert_eq!(cpu.hl(), 0x9028);
        assert_eq!(cpu.f, FLAG_Z | FLAG_H);
        cpu.set_hl(0x8A23);
        cpu.f = 0;
        cpu.step(&mut ram);
        assert_eq!(cpu.hl(), 0x1446);
        assert_eq!(cpu.f, FLAG_H | FLAG_C);
    }

    #[test]
    fn add_sp_signed_and_ld_hl_sp_plus_e8() {
        let (mut cpu, mut ram) = setup(&[0xE8, 0x02, 0xE8, 0x01, 0xF8, 0xFE]);
        cpu.sp = 0xFFF8;
        assert_eq!(cpu.step(&mut ram), 16);
        assert_eq!((cpu.sp, cpu.f), (0xFFFA, 0));
        cpu.sp = 0x00FF;
        cpu.step(&mut ram);
        assert_eq!((cpu.sp, cpu.f), (0x0100, FLAG_H | FLAG_C));
        cpu.sp = 0xFFF8;
        assert_eq!(cpu.step(&mut ram), 12); // LD HL,SP-2
        assert_eq!((cpu.hl(), cpu.sp, cpu.f), (0xFFF6, 0xFFF8, FLAG_H | FLAG_C));
    }

    #[test]
    fn relative_jumps() {
        let (mut cpu, mut ram) = setup(&[0x18, 0xFE]); // JR -2 : boucle infinie
        assert_eq!(cpu.step(&mut ram), 12);
        assert_eq!(cpu.pc, 0);
        let (mut cpu, mut ram) = setup(&[0x20, 0x05]); // JR NZ,+5
        cpu.f = FLAG_Z;
        assert_eq!(cpu.step(&mut ram), 8);
        assert_eq!(cpu.pc, 2);
        let (mut cpu, mut ram) = setup(&[0x20, 0x05]);
        assert_eq!(cpu.step(&mut ram), 12);
        assert_eq!(cpu.pc, 7);
    }

    #[test]
    fn call_and_ret_use_the_stack() {
        let (mut cpu, mut ram) = setup(&[0xCD, 0x10, 0x00]); // CALL 0x0010
        ram.mem[0x10] = 0xC9; // RET
        cpu.sp = 0xFFFE;
        assert_eq!(cpu.step(&mut ram), 24);
        assert_eq!((cpu.pc, cpu.sp), (0x0010, 0xFFFC));
        assert_eq!((ram.mem[0xFFFC], ram.mem[0xFFFD]), (0x03, 0x00));
        assert_eq!(cpu.step(&mut ram), 16);
        assert_eq!((cpu.pc, cpu.sp), (0x0003, 0xFFFE));
    }

    #[test]
    fn conditional_call_and_ret_cycles() {
        let (mut cpu, mut ram) = setup(&[0xC4, 0x00, 0x20, 0xC0]); // CALL NZ ; RET NZ
        cpu.f = FLAG_Z;
        assert_eq!(cpu.step(&mut ram), 12); // pas pris
        assert_eq!(cpu.pc, 3);
        assert_eq!(cpu.step(&mut ram), 8); // RET NZ pas pris
    }

    #[test]
    fn push_pop_af_masks_the_low_nibble() {
        let (mut cpu, mut ram) = setup(&[0xF5, 0xC1, 0xF1]); // PUSH AF ; POP BC ; POP AF
        cpu.sp = 0xFFFE;
        cpu.a = 0x12;
        cpu.f = FLAG_Z | FLAG_C;
        assert_eq!(cpu.step(&mut ram), 16);
        assert_eq!(cpu.step(&mut ram), 12);
        assert_eq!((cpu.b, cpu.c), (0x12, 0x90));
        ram.mem[0xFFFC] = 0xFF; // octet bas de F avec des bits « fantômes »
        ram.mem[0xFFFD] = 0x77;
        cpu.sp = 0xFFFC;
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x77, 0xF0));
    }

    #[test]
    fn daa_after_addition_and_subtraction() {
        let (mut cpu, mut ram) = setup(&[0xC6, 0x27, 0x27, 0xD6, 0x15, 0x27]); // ADD 0x27 ; DAA ; SUB 0x15 ; DAA
        cpu.a = 0x15;
        cpu.step(&mut ram);
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x42, 0));
        cpu.step(&mut ram); // 0x42 - 0x15 = 0x2D
        assert_eq!(cpu.a, 0x2D);
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x27, FLAG_N));
    }

    #[test]
    fn rotate_a_clears_zero() {
        let (mut cpu, mut ram) = setup(&[0x07, 0x17, 0x1F]); // RLCA ; RLA ; RRA
        cpu.a = 0x85;
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x0B, FLAG_C));
        cpu.a = 0x95;
        cpu.f = FLAG_C | FLAG_Z;
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x2B, FLAG_C));
        cpu.a = 0x00;
        cpu.f = 0;
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.f), (0x00, 0)); // RRA ne met jamais Z
    }

    #[test]
    fn cb_shifts_and_swap() {
        let (mut cpu, mut ram) = setup(&[0xCB, 0x37, 0xCB, 0x27, 0xCB, 0x2F, 0xCB, 0x3F, 0xCB, 0x17]);
        cpu.a = 0xF0;
        assert_eq!(cpu.step(&mut ram), 8); // SWAP A
        assert_eq!((cpu.a, cpu.f), (0x0F, 0));
        cpu.a = 0x80;
        cpu.step(&mut ram); // SLA A
        assert_eq!((cpu.a, cpu.f), (0x00, FLAG_Z | FLAG_C));
        cpu.a = 0x81;
        cpu.step(&mut ram); // SRA A garde le bit de signe
        assert_eq!((cpu.a, cpu.f), (0xC0, FLAG_C));
        cpu.a = 0x01;
        cpu.step(&mut ram); // SRL A
        assert_eq!((cpu.a, cpu.f), (0x00, FLAG_Z | FLAG_C));
        cpu.a = 0x80;
        cpu.f = 0;
        cpu.step(&mut ram); // RL A sans retenue entrante
        assert_eq!((cpu.a, cpu.f), (0x00, FLAG_Z | FLAG_C));
    }

    #[test]
    fn cb_bit_set_res() {
        let (mut cpu, mut ram) = setup(&[0xCB, 0x7C, 0xCB, 0x46, 0xCB, 0xD8, 0xCB, 0x80, 0xCB, 0xC6]);
        cpu.h = 0x80;
        cpu.f = FLAG_C;
        assert_eq!(cpu.step(&mut ram), 8); // BIT 7,H
        assert_eq!(cpu.f, FLAG_H | FLAG_C);
        cpu.set_hl(0xC000);
        assert_eq!(cpu.step(&mut ram), 12); // BIT 0,(HL) : 0xC000 contient 0 => Z
        assert!(cpu.f & FLAG_Z != 0);
        cpu.b = 0;
        cpu.step(&mut ram); // SET 3,B
        assert_eq!(cpu.b, 0x08);
        cpu.b = 0xFF;
        cpu.step(&mut ram); // RES 0,B
        assert_eq!(cpu.b, 0xFE);
        assert_eq!(cpu.step(&mut ram), 16); // SET 0,(HL)
        assert_eq!(ram.mem[0xC000], 0x01);
    }

    #[test]
    fn loads_through_hl_plus_minus_and_high_ram() {
        let (mut cpu, mut ram) = setup(&[0x22, 0x3A, 0xE0, 0x80, 0xFA, 0x00, 0xC1]);
        cpu.set_hl(0xC000);
        cpu.a = 0x42;
        cpu.step(&mut ram); // LD (HL+),A
        assert_eq!((ram.mem[0xC000], cpu.hl()), (0x42, 0xC001));
        ram.mem[0xC001] = 0x99;
        cpu.step(&mut ram); // LD A,(HL-)
        assert_eq!((cpu.a, cpu.hl()), (0x99, 0xC000));
        cpu.step(&mut ram); // LDH (0x80),A
        assert_eq!(ram.mem[0xFF80], 0x99);
        ram.mem[0xC100] = 0x07;
        assert_eq!(cpu.step(&mut ram), 16); // LD A,(0xC100)
        assert_eq!(cpu.a, 0x07);
    }

    #[test]
    fn inc_dec_16_bit_leave_flags_alone() {
        let (mut cpu, mut ram) = setup(&[0x2B, 0x03]); // DEC HL ; INC BC
        cpu.f = FLAG_Z | FLAG_C;
        cpu.set_bc(0xFFFF);
        cpu.step(&mut ram);
        cpu.step(&mut ram);
        assert_eq!((cpu.hl(), cpu.bc(), cpu.f), (0xFFFF, 0x0000, FLAG_Z | FLAG_C));
    }

    #[test]
    fn interrupt_is_dispatched_and_cleared() {
        let (mut cpu, mut ram) = setup(&[0x00]);
        cpu.pc = 0x0100;
        cpu.sp = 0xFFFE;
        cpu.ime = true;
        ram.mem[0xFFFF] = 0x04; // IE : timer
        ram.mem[0xFF0F] = 0x04; // IF : timer
        assert_eq!(cpu.step(&mut ram), 20);
        assert_eq!(cpu.pc, 0x50);
        assert!(!cpu.ime);
        assert_eq!(ram.mem[0xFF0F], 0);
        assert_eq!((ram.mem[0xFFFC], ram.mem[0xFFFD]), (0x00, 0x01));
    }

    #[test]
    fn halt_waits_for_an_interrupt() {
        let (mut cpu, mut ram) = setup(&[0x76, 0x3C]); // HALT ; INC A
        cpu.step(&mut ram);
        assert!(cpu.halted);
        assert_eq!(cpu.step(&mut ram), 4);
        assert_eq!(cpu.pc, 1); // il attend
        ram.mem[0xFFFF] = 0x01;
        ram.mem[0xFF0F] = 0x01; // une interruption arrive, IME coupé : le CPU repart sans la traiter
        cpu.step(&mut ram);
        assert!(!cpu.halted);
        assert_eq!(cpu.a, 1);
    }

    #[test]
    fn ei_takes_effect_after_the_next_instruction() {
        let (mut cpu, mut ram) = setup(&[0xFB, 0x00, 0x00]); // EI ; NOP ; NOP
        cpu.step(&mut ram);
        assert!(!cpu.ime);
        cpu.step(&mut ram);
        assert!(cpu.ime);
    }

    #[test]
    fn ei_then_di_leaves_interrupts_off() {
        let (mut cpu, mut ram) = setup(&[0xFB, 0xF3, 0x00]);
        cpu.step(&mut ram);
        cpu.step(&mut ram);
        cpu.step(&mut ram);
        assert!(!cpu.ime);
    }

    #[test]
    fn illegal_opcode_locks_the_cpu() {
        let (mut cpu, mut ram) = setup(&[0xD3, 0x3C]);
        cpu.step(&mut ram);
        assert!(cpu.locked);
        cpu.step(&mut ram);
        assert_eq!((cpu.a, cpu.pc), (0, 1)); // plus rien ne s'exécute
    }

    #[test]
    fn every_opcode_runs_without_panicking() {
        for op in 0..=0xFFu8 {
            for second in [0x00u8, 0x7F, 0xFF] {
                let (mut cpu, mut ram) = setup(&[op, second, 0x00, 0x00]);
                cpu.sp = 0xC100;
                cpu.set_hl(0xC000);
                let cycles = cpu.step(&mut ram);
                assert!(cycles >= 4 && cycles % 4 == 0, "opcode {op:#04x} : {cycles} cycles");
            }
        }
        for op in 0..=0xFFu8 {
            let (mut cpu, mut ram) = setup(&[0xCB, op]);
            cpu.set_hl(0xC000);
            let cycles = cpu.step(&mut ram);
            assert!([8, 12, 16].contains(&cycles), "CB {op:#04x} : {cycles} cycles");
        }
    }

    #[test]
    fn rst_pushes_and_jumps_to_the_vector() {
        let (mut cpu, mut ram) = setup(&[0xEF]); // RST 0x28
        cpu.sp = 0xFFFE;
        assert_eq!(cpu.step(&mut ram), 16);
        assert_eq!((cpu.pc, cpu.sp), (0x28, 0xFFFC));
    }
}
