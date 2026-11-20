//! Le bus mémoire : tout ce que le processeur lit ou écrit passe par ici.
//!
//! Le CPU ne sait pas ce qu'il y a derrière une adresse (ROM, RAM, écran, manettes…).
//! Il parle uniquement à un `Bus`, ce qui permet de le tester avec une simple mémoire plate
//! et, plus tard, de le brancher sur la vraie carte mémoire de la Game Boy.

pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, value: u8);

    /// Le temps passe : `cycles` T-cycles viennent de s'écouler. Le processeur appelle cette méthode après chaque
    /// accès mémoire (4 T-cycles) pour que le timer, l'écran et le son avancent au milieu même d'une instruction.
    fn tick(&mut self, _cycles: u32) {}

    /// Appelé par l'instruction STOP : renvoie vrai si elle a servi à changer de vitesse (Game Boy Color).
    fn speed_switch(&mut self) -> bool {
        false
    }
}

/// 64 Kio de mémoire d'un seul bloc, sans aucune logique de matériel. Sert aux tests.
pub struct FlatRam {
    pub mem: Vec<u8>,
}

impl FlatRam {
    pub fn new() -> Self {
        Self { mem: vec![0; 0x1_0000] }
    }

    /// Copie `bytes` en mémoire à partir de `addr`.
    pub fn load(&mut self, addr: u16, bytes: &[u8]) {
        let start = addr as usize;
        self.mem[start..start + bytes.len()].copy_from_slice(bytes);
    }
}

impl Default for FlatRam {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for FlatRam {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.mem[addr as usize] = value;
    }
}
