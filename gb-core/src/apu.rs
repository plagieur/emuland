//! L'APU (Audio Processing Unit) : les 4 voix sonores de la Game Boy.
//!
//! - voix 1 : onde carrée avec balayage de fréquence (sweep)
//! - voix 2 : onde carrée
//! - voix 3 : onde libre, lue dans une petite mémoire de 32 échantillons (la « wave RAM »)
//! - voix 4 : bruit (un registre à décalage)
//!
//! Un séquenceur à 512 Hz déclenche la durée, le balayage et l'enveloppe de volume. L'APU produit des
//! échantillons stéréo à la fréquence demandée (`set_sample_rate`) : `take_samples` les récupère.

const CPU_HZ: u64 = 4_194_304;
const FRAME_SEQUENCER_PERIOD: u32 = 8192; // 512 Hz

const DUTY: [[u8; 8]; 4] = [[0, 0, 0, 0, 0, 0, 0, 1], [1, 0, 0, 0, 0, 0, 0, 1], [1, 0, 0, 0, 0, 1, 1, 1], [0, 1, 1, 1, 1, 1, 1, 0]];
const NOISE_DIVISORS: [u32; 8] = [8, 16, 32, 48, 64, 80, 96, 112];

/// Bits toujours lus à 1 pour chaque registre de 0xFF10 à 0xFF26.
const READ_MASK: [u8; 0x17] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // voix 1
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // voix 2 (0xFF15 n'existe pas)
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // voix 3
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // voix 4
    0x00, 0x00, 0x70, // NR50, NR51, NR52
];

#[derive(Default, Clone, Copy)]
struct Envelope {
    initial: u8,
    add: bool,
    period: u8,
    volume: u8,
    timer: u8,
}

impl Envelope {
    fn load(&mut self, v: u8) {
        self.initial = v >> 4;
        self.add = v & 0x08 != 0;
        self.period = v & 0x07;
    }
    fn trigger(&mut self) {
        self.volume = self.initial;
        self.timer = if self.period == 0 { 8 } else { self.period };
    }
    fn clock(&mut self) {
        if self.period == 0 {
            return;
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.period;
            if self.add && self.volume < 15 {
                self.volume += 1;
            } else if !self.add && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
}

#[derive(Default)]
struct Pulse {
    has_sweep: bool,
    enabled: bool,
    dac: bool,
    duty: u8,
    length: u16,
    length_enable: bool,
    env: Envelope,
    freq: u16,
    timer: u32,
    pos: u8,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_timer: u8,
    sweep_enabled: bool,
    shadow: u16,
    /// Un calcul de balayage a été fait en mode « soustraction » depuis le dernier déclenchement.
    neg_used: bool,
}

impl Pulse {
    fn period(&self) -> u32 {
        (2048 - self.freq as u32) * 4
    }

    /// Calcule la nouvelle fréquence du balayage (et note l'usage du mode soustraction).
    fn sweep_calc(&mut self) -> i32 {
        let delta = (self.shadow >> self.sweep_shift) as i32;
        if self.sweep_negate {
            self.neg_used = true;
            self.shadow as i32 - delta
        } else {
            self.shadow as i32 + delta
        }
    }

    /// `fs_odd` : la prochaine étape du séquenceur ne compte pas la durée (on est dans la première moitié d'une période de durée).
    fn write(&mut self, idx: usize, v: u8, fs_odd: bool) {
        match idx {
            0 => {
                let negate = v & 0x08 != 0;
                if self.sweep_negate && !negate && self.neg_used {
                    self.enabled = false; // quitter le mode soustraction après un calcul éteint la voix
                }
                self.sweep_period = (v >> 4) & 7;
                self.sweep_negate = negate;
                self.sweep_shift = v & 7;
            }
            1 => {
                self.duty = v >> 6;
                self.length = 64 - (v & 0x3F) as u16;
            }
            2 => {
                self.env.load(v);
                self.dac = v & 0xF8 != 0;
                if !self.dac {
                    self.enabled = false;
                }
            }
            3 => self.freq = (self.freq & 0x700) | v as u16,
            _ => {
                self.freq = (self.freq & 0x0FF) | ((v as u16 & 7) << 8);
                let length_enable = v & 0x40 != 0;
                // activer la durée dans la première moitié d'une période la fait compter une fois tout de suite
                if !self.length_enable && length_enable && self.length > 0 && fs_odd {
                    self.length -= 1;
                    if self.length == 0 {
                        self.enabled = false;
                    }
                }
                self.length_enable = length_enable;
                if v & 0x80 != 0 {
                    self.trigger(fs_odd);
                }
            }
        }
    }

    fn trigger(&mut self, fs_odd: bool) {
        self.enabled = self.dac;
        if self.length == 0 {
            self.length = 64;
            if self.length_enable && fs_odd {
                self.length -= 1;
            }
        }
        self.timer = self.period();
        self.env.trigger();
        if self.has_sweep {
            self.shadow = self.freq;
            self.neg_used = false;
            self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
            self.sweep_enabled = self.sweep_period > 0 || self.sweep_shift > 0;
            if self.sweep_shift > 0 && self.sweep_calc() > 2047 {
                self.enabled = false;
            }
        }
    }

    fn tick(&mut self) {
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.period();
            self.pos = (self.pos + 1) & 7;
        }
    }

    fn clock_length(&mut self) {
        if self.length_enable && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }

    fn clock_sweep(&mut self) {
        if !self.has_sweep {
            return;
        }
        self.sweep_timer = self.sweep_timer.saturating_sub(1);
        if self.sweep_timer == 0 {
            self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
            if self.sweep_enabled && self.sweep_period > 0 {
                let target = self.sweep_calc();
                if target > 2047 {
                    self.enabled = false;
                } else if self.sweep_shift > 0 {
                    self.freq = target as u16;
                    self.shadow = target as u16;
                    if self.sweep_calc() > 2047 {
                        self.enabled = false;
                    }
                }
            }
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && self.dac { DUTY[self.duty as usize][self.pos as usize] * self.env.volume } else { 0 }
    }
}

#[derive(Default)]
struct Wave {
    enabled: bool,
    dac: bool,
    length: u16,
    length_enable: bool,
    volume_code: u8,
    freq: u16,
    timer: u32,
    pos: u8,
    ram: [u8; 16],
    /// Cycles restants pendant lesquels la voix « tient » la wave RAM : sur la console d'origine, la lire ou
    /// l'écrire pendant que la voix joue ne marche qu'à ce moment précis, et donne l'octet en cours de lecture.
    window: u8,
}

impl Wave {
    fn period(&self) -> u32 {
        (2048 - self.freq as u32) * 2
    }

    fn write(&mut self, idx: usize, v: u8, fs_odd: bool, cgb: bool) {
        match idx {
            0 => {
                self.dac = v & 0x80 != 0;
                if !self.dac {
                    self.enabled = false;
                }
            }
            1 => self.length = 256 - v as u16,
            2 => self.volume_code = (v >> 5) & 3,
            3 => self.freq = (self.freq & 0x700) | v as u16,
            _ => {
                self.freq = (self.freq & 0x0FF) | ((v as u16 & 7) << 8);
                let length_enable = v & 0x40 != 0;
                if !self.length_enable && length_enable && self.length > 0 && fs_odd {
                    self.length -= 1;
                    if self.length == 0 {
                        self.enabled = false;
                    }
                }
                self.length_enable = length_enable;
                if v & 0x80 != 0 {
                    // déclencher la voix alors qu'elle est sur le point de lire un échantillon abîme le début de la wave RAM
                    if self.enabled && self.timer == 1 && !cgb {
                        let next = (((self.pos + 1) & 31) / 2) as usize;
                        if next < 4 {
                            self.ram[0] = self.ram[next];
                        } else {
                            let base = next & !3;
                            for i in 0..4 {
                                self.ram[i] = self.ram[base + i];
                            }
                        }
                    }
                    self.enabled = self.dac;
                    if self.length == 0 {
                        self.length = 256;
                        if self.length_enable && fs_odd {
                            self.length -= 1;
                        }
                    }
                    self.timer = self.period() + 3; // la première lecture arrive un peu plus tard
                    self.pos = 0;
                }
            }
        }
    }

    fn tick(&mut self) {
        self.window = self.window.saturating_sub(1);
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.period();
            self.pos = (self.pos + 1) & 31;
            self.window = 2;
        }
    }

    /// L'octet de la wave RAM que la voix est en train de lire.
    fn current_byte(&self) -> usize {
        (self.pos / 2) as usize
    }

    fn clock_length(&mut self) {
        if self.length_enable && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }

    fn output(&self) -> u8 {
        if !(self.enabled && self.dac) {
            return 0;
        }
        let byte = self.ram[(self.pos / 2) as usize];
        let sample = if self.pos % 2 == 0 { byte >> 4 } else { byte & 0x0F };
        match self.volume_code {
            0 => 0,
            1 => sample,
            2 => sample >> 1,
            _ => sample >> 2,
        }
    }
}

struct Noise {
    enabled: bool,
    dac: bool,
    length: u16,
    length_enable: bool,
    env: Envelope,
    shift: u8,
    width7: bool,
    divisor: u8,
    timer: u32,
    lfsr: u16,
}

impl Noise {
    fn new() -> Self {
        Self { enabled: false, dac: false, length: 0, length_enable: false, env: Envelope::default(), shift: 0, width7: false, divisor: 0, timer: 0, lfsr: 0x7FFF }
    }

    fn period(&self) -> u32 {
        NOISE_DIVISORS[self.divisor as usize] << self.shift
    }

    fn write(&mut self, idx: usize, v: u8, fs_odd: bool) {
        match idx {
            1 => self.length = 64 - (v & 0x3F) as u16,
            2 => {
                self.env.load(v);
                self.dac = v & 0xF8 != 0;
                if !self.dac {
                    self.enabled = false;
                }
            }
            3 => {
                self.shift = v >> 4;
                self.width7 = v & 0x08 != 0;
                self.divisor = v & 7;
            }
            4 => {
                let length_enable = v & 0x40 != 0;
                if !self.length_enable && length_enable && self.length > 0 && fs_odd {
                    self.length -= 1;
                    if self.length == 0 {
                        self.enabled = false;
                    }
                }
                self.length_enable = length_enable;
                if v & 0x80 != 0 {
                    self.enabled = self.dac;
                    if self.length == 0 {
                        self.length = 64;
                        if self.length_enable && fs_odd {
                            self.length -= 1;
                        }
                    }
                    self.timer = self.period();
                    self.env.trigger();
                    self.lfsr = 0x7FFF;
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self) {
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.period();
            if self.shift < 14 {
                let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
                self.lfsr = (self.lfsr >> 1) | (bit << 14);
                if self.width7 {
                    self.lfsr = (self.lfsr & !0x40) | (bit << 6);
                }
            }
        }
    }

    fn clock_length(&mut self) {
        if self.length_enable && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && self.dac { ((!self.lfsr) & 1) as u8 * self.env.volume } else { 0 }
    }
}

pub struct Apu {
    /// Game Boy Color : quelques comportements diffèrent de la console d'origine (voir `set_cgb`).
    cgb: bool,
    power: bool,
    regs: [u8; 0x17], // dernières valeurs écrites, de 0xFF10 à 0xFF26
    ch1: Pulse,
    ch2: Pulse,
    ch3: Wave,
    ch4: Noise,
    fs_counter: u32,
    fs_step: u8,
    // production des échantillons
    rate: u32,
    sample_acc: u64,
    sum_l: f32,
    sum_r: f32,
    sum_n: u32,
    cap_l: f32,
    cap_r: f32,
    charge: f32,
    samples: Vec<f32>,
}

impl Apu {
    pub fn new() -> Self {
        let mut apu = Self {
            cgb: false,
            power: true,
            regs: [0; 0x17],
            ch1: Pulse { has_sweep: true, ..Pulse::default() },
            ch2: Pulse::default(),
            ch3: Wave::default(),
            ch4: Noise::new(),
            fs_counter: 0,
            fs_step: 0,
            rate: 48_000,
            sample_acc: 0,
            sum_l: 0.0,
            sum_r: 0.0,
            sum_n: 0,
            cap_l: 0.0,
            cap_r: 0.0,
            charge: 0.0,
            samples: Vec::new(),
        };
        apu.regs[0x14] = 0x77; // NR50 après la ROM de démarrage
        apu.regs[0x15] = 0xF3; // NR51
        apu.set_sample_rate(48_000);
        apu
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Mode Game Boy Color. Différences avec la console d'origine : éteint, les compteurs de durée sont remis
    /// à zéro et aucune écriture n'est acceptée ; la wave RAM reste lisible et modifiable pendant que la voix joue
    /// (elle donne l'octet en cours de lecture) et déclencher la voix ne l'abîme pas.
    pub fn set_cgb(&mut self, on: bool) {
        self.cgb = on;
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.rate = rate.max(8_000);
        // filtre qui retire la composante continue, comme le condensateur de sortie de la vraie console
        self.charge = 0.999958f32.powf(CPU_HZ as f32 / self.rate as f32);
        self.samples.clear();
    }

    /// Récupère (et vide) les échantillons produits : paires gauche/droite entre -1 et 1.
    pub fn take_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples)
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF10..=0xFF26 => {
                let i = (addr - 0xFF10) as usize;
                if addr == 0xFF26 {
                    let status = self.ch1.enabled as u8 | (self.ch2.enabled as u8) << 1 | (self.ch3.enabled as u8) << 2 | (self.ch4.enabled as u8) << 3;
                    return 0x70 | (self.power as u8) << 7 | status;
                }
                self.regs[i] | READ_MASK[i]
            }
            0xFF27..=0xFF2F => 0xFF,
            0xFF30..=0xFF3F => {
                if self.ch3.enabled {
                    // pendant que la voix joue, la wave RAM ne répond qu'au moment où la voix la lit
                    // (sur la Color, elle répond toujours, avec l'octet en cours de lecture)
                    if self.ch3.window > 0 || self.cgb { self.ch3.ram[self.ch3.current_byte()] } else { 0xFF }
                } else {
                    self.ch3.ram[(addr - 0xFF30) as usize]
                }
            }
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        match addr {
            0xFF26 => {
                let on = v & 0x80 != 0;
                if self.power && !on {
                    self.power_off();
                } else if !self.power && on {
                    self.power = true;
                    self.fs_step = 0;
                    self.fs_counter = 0;
                }
            }
            0xFF30..=0xFF3F => {
                if self.ch3.enabled {
                    if self.ch3.window > 0 || self.cgb {
                        let i = self.ch3.current_byte();
                        self.ch3.ram[i] = v;
                    } // sinon l'écriture est perdue
                } else {
                    self.ch3.ram[(addr - 0xFF30) as usize] = v;
                }
            }
            0xFF10..=0xFF25 if self.power => {
                let i = (addr - 0xFF10) as usize;
                let fs_odd = self.fs_step % 2 == 1;
                self.regs[i] = v;
                match i {
                    0..=4 => self.ch1.write(i, v, fs_odd),
                    5..=9 => {
                        if i != 5 {
                            self.ch2.write(i - 5, v, fs_odd)
                        }
                    }
                    10..=14 => self.ch3.write(i - 10, v, fs_odd, self.cgb),
                    15..=19 => self.ch4.write(i - 15, v, fs_odd),
                    _ => {}
                }
            }
            // éteint, l'APU ignore presque tout, mais les compteurs de durée restent réglables (console d'origine)
            0xFF11 if !self.power && !self.cgb => self.ch1.length = 64 - (v & 0x3F) as u16,
            0xFF16 if !self.power && !self.cgb => self.ch2.length = 64 - (v & 0x3F) as u16,
            0xFF1B if !self.power && !self.cgb => self.ch3.length = 256 - v as u16,
            0xFF20 if !self.power && !self.cgb => self.ch4.length = 64 - (v & 0x3F) as u16,
            _ => {}
        }
    }

    /// Éteindre l'APU remet tous ses registres à zéro (la wave RAM et les compteurs de durée restent).
    fn power_off(&mut self) {
        let ram = self.ch3.ram;
        // console d'origine : les compteurs de durée gardent leur valeur ; Color : ils repartent de zéro
        let lengths = if self.cgb { [0; 4] } else { [self.ch1.length, self.ch2.length, self.ch3.length, self.ch4.length] };
        self.ch1 = Pulse { has_sweep: true, length: lengths[0], ..Pulse::default() };
        self.ch2 = Pulse { length: lengths[1], ..Pulse::default() };
        self.ch3 = Wave { ram, length: lengths[2], ..Wave::default() };
        self.ch4 = Noise { length: lengths[3], ..Noise::new() };
        self.regs = [0; 0x17];
        self.power = false;
    }

    fn clock_frame_sequencer(&mut self) {
        let step = self.fs_step;
        if step % 2 == 0 {
            self.ch1.clock_length();
            self.ch2.clock_length();
            self.ch3.clock_length();
            self.ch4.clock_length();
        }
        if step == 2 || step == 6 {
            self.ch1.clock_sweep();
        }
        if step == 7 {
            self.ch1.env.clock();
            self.ch2.env.clock();
            self.ch4.env.clock();
        }
        self.fs_step = (step + 1) & 7;
    }

    /// Le mélange stéréo actuel (entre 0 et 1) avant filtrage.
    fn mix(&self) -> (f32, f32) {
        let outs = [self.ch1.output(), self.ch2.output(), self.ch3.output(), self.ch4.output()];
        let (nr50, nr51) = (self.regs[0x14], self.regs[0x15]);
        let (mut left, mut right) = (0.0f32, 0.0f32);
        for (i, &o) in outs.iter().enumerate() {
            let v = o as f32 / 15.0;
            if nr51 >> (4 + i) & 1 != 0 {
                left += v;
            }
            if nr51 >> i & 1 != 0 {
                right += v;
            }
        }
        let vol_l = (((nr50 >> 4) & 7) + 1) as f32 / 8.0;
        let vol_r = ((nr50 & 7) + 1) as f32 / 8.0;
        (left / 4.0 * vol_l, right / 4.0 * vol_r)
    }

    pub fn tick(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.ch1.tick();
            self.ch2.tick();
            self.ch3.tick();
            self.ch4.tick();
            self.fs_counter += 1;
            if self.fs_counter >= FRAME_SEQUENCER_PERIOD {
                self.fs_counter = 0;
                self.clock_frame_sequencer();
            }
            let (l, r) = self.mix();
            self.sum_l += l;
            self.sum_r += r;
            self.sum_n += 1;
            self.sample_acc += self.rate as u64;
            if self.sample_acc >= CPU_HZ {
                self.sample_acc -= CPU_HZ;
                let n = self.sum_n.max(1) as f32;
                let (inl, inr) = (self.sum_l / n, self.sum_r / n);
                let (outl, outr) = (inl - self.cap_l, inr - self.cap_r);
                self.cap_l = inl - outl * self.charge;
                self.cap_r = inr - outr * self.charge;
                self.samples.push(outl);
                self.samples.push(outr);
                self.sum_l = 0.0;
                self.sum_r = 0.0;
                self.sum_n = 0;
            }
        }
        // si personne ne récupère les échantillons (tests, jeu en pause), on évite que la liste grossisse sans fin
        let cap = self.rate as usize * 2; // une seconde en stéréo
        if self.samples.len() > cap * 2 {
            self.samples.drain(..cap);
        }
    }

    #[cfg(test)]
    fn output(&self, ch: usize) -> u8 {
        [self.ch1.output(), self.ch2.output(), self.ch3.output(), self.ch4.output()][ch]
    }

    #[cfg(test)]
    fn volume(&self, ch: usize) -> u8 {
        [self.ch1.env.volume, self.ch2.env.volume, 0, self.ch4.env.volume][ch]
    }
}

use crate::state::{StateReader, StateWriter};

impl Envelope {
    fn save(&self, w: &mut StateWriter) {
        w.u8(self.initial);
        w.bool(self.add);
        w.u8(self.period);
        w.u8(self.volume);
        w.u8(self.timer);
    }
    fn load_from(&mut self, r: &mut StateReader) -> Result<(), String> {
        self.initial = r.u8()? & 15;
        self.add = r.bool()?;
        self.period = r.u8()? & 7;
        self.volume = r.u8()? & 15;
        self.timer = r.u8()?;
        Ok(())
    }
}

impl Pulse {
    fn save(&self, w: &mut StateWriter) {
        w.bool(self.enabled);
        w.bool(self.dac);
        w.u8(self.duty);
        w.u16(self.length);
        w.bool(self.length_enable);
        self.env.save(w);
        w.u16(self.freq);
        w.u32(self.timer);
        w.u8(self.pos);
        w.u8(self.sweep_period);
        w.bool(self.sweep_negate);
        w.u8(self.sweep_shift);
        w.u8(self.sweep_timer);
        w.bool(self.sweep_enabled);
        w.u16(self.shadow);
        w.bool(self.neg_used);
    }
    fn load_from(&mut self, r: &mut StateReader) -> Result<(), String> {
        self.enabled = r.bool()?;
        self.dac = r.bool()?;
        self.duty = r.u8()? & 3;
        self.length = r.u16()?;
        self.length_enable = r.bool()?;
        self.env.load_from(r)?;
        self.freq = r.u16()? & 0x7FF;
        self.timer = r.u32()?;
        self.pos = r.u8()? & 7;
        self.sweep_period = r.u8()? & 7;
        self.sweep_negate = r.bool()?;
        self.sweep_shift = r.u8()? & 7;
        self.sweep_timer = r.u8()?;
        self.sweep_enabled = r.bool()?;
        self.shadow = r.u16()? & 0x7FF;
        self.neg_used = r.bool()?;
        Ok(())
    }
}

impl Wave {
    fn save(&self, w: &mut StateWriter) {
        w.bool(self.enabled);
        w.bool(self.dac);
        w.u16(self.length);
        w.bool(self.length_enable);
        w.u8(self.volume_code);
        w.u16(self.freq);
        w.u32(self.timer);
        w.u8(self.pos);
        w.bytes(&self.ram);
    }
    fn load_from(&mut self, r: &mut StateReader) -> Result<(), String> {
        self.enabled = r.bool()?;
        self.dac = r.bool()?;
        self.length = r.u16()?;
        self.length_enable = r.bool()?;
        self.volume_code = r.u8()? & 3;
        self.freq = r.u16()? & 0x7FF;
        self.timer = r.u32()?;
        self.pos = r.u8()? & 31;
        r.bytes_into(&mut self.ram)
    }
}

impl Noise {
    fn save(&self, w: &mut StateWriter) {
        w.bool(self.enabled);
        w.bool(self.dac);
        w.u16(self.length);
        w.bool(self.length_enable);
        self.env.save(w);
        w.u8(self.shift);
        w.bool(self.width7);
        w.u8(self.divisor);
        w.u32(self.timer);
        w.u16(self.lfsr);
    }
    fn load_from(&mut self, r: &mut StateReader) -> Result<(), String> {
        self.enabled = r.bool()?;
        self.dac = r.bool()?;
        self.length = r.u16()?;
        self.length_enable = r.bool()?;
        self.env.load_from(r)?;
        self.shift = r.u8()? & 15;
        self.width7 = r.bool()?;
        self.divisor = r.u8()? & 7;
        self.timer = r.u32()?;
        self.lfsr = r.u16()? & 0x7FFF;
        Ok(())
    }
}

impl Apu {
    pub(crate) fn save_state(&self, w: &mut StateWriter) {
        w.bool(self.power);
        w.bytes(&self.regs);
        self.ch1.save(w);
        self.ch2.save(w);
        self.ch3.save(w);
        self.ch4.save(w);
        w.u32(self.fs_counter);
        w.u8(self.fs_step);
    }

    /// Les échantillons déjà produits et les filtres de sortie ne font pas partie de l'état : on repart d'un son propre.
    pub(crate) fn load_state(&mut self, r: &mut StateReader) -> Result<(), String> {
        self.power = r.bool()?;
        r.bytes_into(&mut self.regs)?;
        self.ch1.load_from(r)?;
        self.ch2.load_from(r)?;
        self.ch3.load_from(r)?;
        self.ch4.load_from(r)?;
        self.fs_counter = r.u32()?;
        self.fs_step = r.u8()? & 7;
        self.samples.clear();
        self.sample_acc = 0;
        (self.sum_l, self.sum_r, self.sum_n, self.cap_l, self.cap_r) = (0.0, 0.0, 0, 0.0, 0.0);
        Ok(())
    }
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nr52(apu: &Apu) -> u8 {
        apu.read(0xFF26)
    }

    /// Règle la voix 1 : volume max, durée infinie, onde carrée à 50 %, fréquence `freq`.
    fn start_ch1(apu: &mut Apu, freq: u16) {
        apu.write(0xFF12, 0xF0);
        apu.write(0xFF11, 0x80);
        apu.write(0xFF13, freq as u8);
        apu.write(0xFF14, 0x80 | (freq >> 8) as u8);
    }

    #[test]
    fn square_wave_follows_its_duty_cycle() {
        let mut apu = Apu::new();
        start_ch1(&mut apu, 1920); // 512 cycles par pas
        let mut highs = 0;
        for _ in 0..8 {
            apu.tick(512);
            if apu.output(0) == 15 {
                highs += 1;
            }
        }
        assert_eq!(highs, 4, "un rapport cyclique de 50 % : 4 pas sur 8");
    }

    #[test]
    fn trigger_sets_status_and_length_stops_the_channel() {
        let mut apu = Apu::new();
        apu.write(0xFF12, 0xF0);
        apu.write(0xFF11, 0x3F); // durée = 1
        apu.write(0xFF14, 0xC0); // déclenche + durée active
        assert_eq!(nr52(&apu) & 1, 1);
        apu.tick(FRAME_SEQUENCER_PERIOD); // un pas du séquenceur : la durée tombe à 0
        assert_eq!(nr52(&apu) & 1, 0);
    }

    #[test]
    fn a_channel_with_its_dac_off_cannot_be_triggered() {
        let mut apu = Apu::new();
        apu.write(0xFF12, 0x00); // DAC éteint
        apu.write(0xFF14, 0x80);
        assert_eq!(nr52(&apu) & 1, 0);
    }

    #[test]
    fn envelope_decays_every_eight_sequencer_steps() {
        let mut apu = Apu::new();
        apu.write(0xFF12, 0xF1); // volume 15, descend, période 1
        apu.write(0xFF14, 0x80);
        assert_eq!(apu.volume(0), 15);
        apu.tick(FRAME_SEQUENCER_PERIOD * 8);
        assert_eq!(apu.volume(0), 14);
        apu.tick(FRAME_SEQUENCER_PERIOD * 8);
        assert_eq!(apu.volume(0), 13);
    }

    #[test]
    fn sweep_overflow_disables_the_channel() {
        let mut apu = Apu::new();
        apu.write(0xFF10, 0x11); // période 1, vers le haut, décalage 1
        start_ch1(&mut apu, 2047);
        assert_eq!(nr52(&apu) & 1, 0, "2047 + 1023 dépasse 2047 : la voix s'éteint au déclenchement");
    }

    #[test]
    fn sweep_raises_the_frequency() {
        let mut apu = Apu::new();
        apu.write(0xFF10, 0x11); // période 1, vers le haut, décalage 1
        start_ch1(&mut apu, 1000);
        apu.tick(FRAME_SEQUENCER_PERIOD * 3); // pas 0, 1, 2 : le balayage joue au pas 2
        assert_eq!(apu.ch1.freq, 1500);
    }

    #[test]
    fn wave_channel_plays_its_ram_with_volume_shift() {
        let mut apu = Apu::new();
        apu.write(0xFF30, 0xAB); // échantillons 0 et 1 : A puis B
        apu.write(0xFF1A, 0x80); // DAC allumé
        apu.write(0xFF1C, 0x20); // volume 100 %
        apu.write(0xFF1E, 0x80 | 0x07); // déclenche, fréquence haute
        assert_eq!(apu.output(2), 0x0A);
        apu.write(0xFF1C, 0x40); // volume 50 %
        assert_eq!(apu.output(2), 0x05);
        apu.write(0xFF1C, 0x00); // muet
        assert_eq!(apu.output(2), 0);
    }

    #[test]
    fn noise_changes_over_time() {
        let mut apu = Apu::new();
        apu.write(0xFF21, 0xF0);
        apu.write(0xFF22, 0x00); // le plus rapide
        apu.write(0xFF23, 0x80);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            apu.tick(8);
            seen.insert(apu.output(3));
        }
        assert!(seen.len() >= 2, "le bruit alterne entre silence et volume");
    }

    #[test]
    fn power_off_clears_registers_and_blocks_writes() {
        let mut apu = Apu::new();
        apu.write(0xFF12, 0xF0);
        apu.write(0xFF26, 0x00);
        assert_eq!(apu.read(0xFF26), 0x70);
        assert_eq!(apu.read(0xFF12), 0x00);
        apu.write(0xFF12, 0xF0); // ignoré
        assert_eq!(apu.read(0xFF12), 0x00);
        apu.write(0xFF30, 0x12); // la wave RAM reste accessible
        assert_eq!(apu.read(0xFF30), 0x12);
        apu.write(0xFF26, 0x80);
        apu.write(0xFF12, 0xF0);
        assert_eq!(apu.read(0xFF12), 0xF0);
    }

    #[test]
    fn unused_bits_read_back_as_ones() {
        let mut apu = Apu::new();
        apu.write(0xFF11, 0x00);
        assert_eq!(apu.read(0xFF11), 0x3F); // la durée ne se relit pas
        assert_eq!(apu.read(0xFF15), 0xFF);
    }

    #[test]
    fn panning_routes_channels_to_each_side() {
        let mut apu = Apu::new();
        apu.write(0xFF24, 0x77);
        apu.write(0xFF25, 0x10); // voix 1 à gauche uniquement
        start_ch1(&mut apu, 1920);
        apu.tick(512 * 5); // 5 pas plus loin : l'onde à 50 % est à l'état haut du pas 5 au pas 7
        let (l, r) = apu.mix();
        assert!(l > 0.0 && r == 0.0, "gauche {l}, droite {r}");
    }

    #[test]
    fn produces_samples_at_the_requested_rate() {
        let mut apu = Apu::new();
        apu.set_sample_rate(48_000);
        apu.tick(4_194_304); // une seconde
        let samples = apu.take_samples();
        let frames = samples.len() / 2;
        assert!((47_990..=48_010).contains(&frames), "{frames} échantillons");
        assert!(apu.take_samples().is_empty());
    }

    #[test]
    fn silence_stays_silent_and_a_tone_makes_sound() {
        let mut apu = Apu::new();
        apu.tick(100_000);
        assert!(apu.take_samples().iter().all(|s| s.abs() < 1e-6));
        start_ch1(&mut apu, 1920);
        apu.tick(100_000);
        let peak = apu.take_samples().iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.05, "pic {peak}");
    }
}
