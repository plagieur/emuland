//! La manette : 8 boutons lus par le jeu à travers le registre 0xFF00.
//!
//! Le jeu choisit un groupe en écrivant dans les bits 4 (croix directionnelle) et 5 (boutons),
//! puis lit les 4 bits du bas. Sur la machine, un bouton enfoncé se lit « 0 ».

pub const RIGHT: u8 = 0x01;
pub const LEFT: u8 = 0x02;
pub const UP: u8 = 0x04;
pub const DOWN: u8 = 0x08;
pub const A: u8 = 0x10;
pub const B: u8 = 0x20;
pub const SELECT: u8 = 0x40;
pub const START: u8 = 0x80;

#[derive(Default, Clone, Copy)]
pub struct Joypad {
    /// Boutons enfoncés (1 = enfoncé) : bits 0-3 croix, bits 4-7 boutons.
    state: u8,
    /// Les bits 4 et 5 écrits par le jeu (0 = groupe choisi).
    select: u8,
}

impl Joypad {
    pub fn new() -> Self {
        Self { state: 0, select: 0x30 }
    }

    /// Valeur du registre 0xFF00.
    pub fn read(&self) -> u8 {
        let mut low = 0x0F;
        if self.select & 0x10 == 0 {
            low &= !(self.state & 0x0F); // croix
        }
        if self.select & 0x20 == 0 {
            low &= !(self.state >> 4); // boutons
        }
        0xC0 | self.select | low
    }

    pub fn write(&mut self, value: u8) {
        self.select = value & 0x30;
    }

    /// Met à jour les boutons enfoncés. Renvoie vrai si une ligne choisie vient de passer à « enfoncé »
    /// (le matériel demande alors l'interruption manette).
    pub fn set_state(&mut self, new: u8) -> bool {
        let before = self.read() & 0x0F;
        self.state = new;
        let after = self.read() & 0x0F;
        before & !after != 0
    }
}

impl Joypad {
    pub(crate) fn save_state(&self, w: &mut crate::state::StateWriter) {
        w.u8(self.state);
        w.u8(self.select);
    }

    pub(crate) fn load_state(&mut self, r: &mut crate::state::StateReader) -> Result<(), String> {
        self.state = r.u8()?;
        self.select = r.u8()? & 0x30;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_pressed_reads_all_ones() {
        let mut pad = Joypad::new();
        pad.write(0x10); // boutons choisis
        assert_eq!(pad.read() & 0x0F, 0x0F);
        pad.write(0x20); // croix choisie
        assert_eq!(pad.read() & 0x0F, 0x0F);
    }

    #[test]
    fn dpad_and_buttons_are_separate_groups() {
        let mut pad = Joypad::new();
        pad.set_state(UP | A);
        pad.write(0x20); // croix : seul « haut » (bit 2) tombe à 0
        assert_eq!(pad.read() & 0x0F, 0b1011);
        pad.write(0x10); // boutons : seul A (bit 0) tombe à 0
        assert_eq!(pad.read() & 0x0F, 0b1110);
        pad.write(0x30); // aucun groupe : tout à 1
        assert_eq!(pad.read() & 0x0F, 0x0F);
    }

    #[test]
    fn both_groups_selected_combine() {
        let mut pad = Joypad::new();
        pad.set_state(RIGHT | START);
        pad.write(0x00);
        assert_eq!(pad.read() & 0x0F, 0b0110); // droite (bit 0) et start (bit 3) tombent à 0
    }

    #[test]
    fn unused_upper_bits_read_as_one() {
        let pad = Joypad::new();
        assert_eq!(pad.read() & 0xC0, 0xC0);
    }

    #[test]
    fn pressing_a_selected_button_raises_the_interrupt_once() {
        let mut pad = Joypad::new();
        pad.write(0x10); // boutons choisis
        assert!(pad.set_state(START));
        assert!(!pad.set_state(START)); // déjà enfoncé : pas de nouvelle demande
        assert!(!pad.set_state(0)); // relâché
        assert!(pad.set_state(B));
    }

    #[test]
    fn pressing_an_unselected_group_does_not_interrupt() {
        let mut pad = Joypad::new();
        pad.write(0x10); // seulement les boutons
        assert!(!pad.set_state(UP)); // la croix n'est pas choisie
    }
}
