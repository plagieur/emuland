//! Une toute petite ROM de démonstration, écrite à la main pour emuland (aucun jeu commercial).
//! Elle dessine « EMULAND » à l'écran, envoie un message sur la liaison série, puis tourne en boucle.

use crate::cartridge::LOGO;

pub const DEMO_TITLE: &str = "EMULAND DEMO";
pub const DEMO_MESSAGE: &str = "emuland ready\n";

const TILES_AT: u16 = 0x0200;
const MAP_AT: u16 = 0x0300;
const MSG_AT: u16 = 0x0540;
const COPY_AT: u16 = 0x0190;

/// Les lettres, en 8 × 8 : `#` noir, `+` gris, `.` blanc.
const GLYPHS: [[&str; 8]; 9] = [
    ["........"; 8], // 0 : vide
    ["########", "##......", "##......", "######..", "##......", "##......", "##......", "########"], // E
    ["##....##", "###..###", "########", "##.##.##", "##....##", "##....##", "##....##", "##....##"], // M
    ["##....##", "##....##", "##....##", "##....##", "##....##", "##....##", "##....##", ".######."], // U
    ["##......", "##......", "##......", "##......", "##......", "##......", "##......", "########"], // L
    ["..####..", ".##..##.", "##....##", "##....##", "########", "##....##", "##....##", "##....##"], // A
    ["##....##", "###...##", "####..##", "##.##.##", "##..####", "##...###", "##....##", "##....##"], // N
    ["######..", "##...##.", "##....##", "##....##", "##....##", "##....##", "##...##.", "######.."], // D
    ["+.+.+.+.", ".+.+.+.+", "+.+.+.+.", ".+.+.+.+", "+.+.+.+.", ".+.+.+.+", "+.+.+.+.", ".+.+.+.+"], // 8 : damier
];

/// Convertit un glyphe en 16 octets au format 2 bits par pixel de la Game Boy.
fn encode_tile(glyph: &[&str; 8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (row, line) in glyph.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            let id = match ch {
                '#' => 3,
                '+' => 2,
                '-' => 1,
                _ => 0,
            };
            out[row * 2] |= (id & 1) << (7 - col);
            out[row * 2 + 1] |= (id >> 1) << (7 - col);
        }
    }
    out
}

fn tiles() -> Vec<u8> {
    GLYPHS.iter().flat_map(encode_tile).collect()
}

/// La carte des tuiles de l'écran : 32 colonnes × 18 lignes.
fn tile_map() -> Vec<u8> {
    let mut map = vec![0u8; 32 * 18];
    for col in 4..=15 {
        map[7 * 32 + col] = 8;
        map[11 * 32 + col] = 8;
    }
    for (i, tile) in (1u8..=7).enumerate() {
        map[9 * 32 + 6 + i] = tile;
    }
    map
}

pub fn demo_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];

    // point d'entrée : NOP ; JP 0x0150
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]);
    rom[0x104..0x134].copy_from_slice(&LOGO);
    rom[0x134..0x134 + DEMO_TITLE.len()].copy_from_slice(DEMO_TITLE.as_bytes());
    // 0x147 type de cartouche = 0 (ROM seule), 0x148 taille = 0 (32 Kio), 0x149 RAM = 0
    let mut sum = 0u8;
    for &b in &rom[0x134..=0x14C] {
        sum = sum.wrapping_sub(b).wrapping_sub(1);
    }
    rom[0x14D] = sum;

    let tiles = tiles();
    let map = tile_map();
    let ld16 = |op: u8, v: u16| [op, v as u8, (v >> 8) as u8];

    let mut code: Vec<u8> = Vec::new();
    code.extend([0x3E, 0x00, 0xE0, 0x40]); //                     LD A,0 ; LDH (LCDC),A : éteint l'écran pour copier tranquillement
    code.extend(ld16(0x21, TILES_AT)); //                         LD HL, tuiles
    code.extend(ld16(0x11, 0x8000)); //                           LD DE, VRAM
    code.extend(ld16(0x01, tiles.len() as u16)); //               LD BC, taille
    code.extend(ld16(0xCD, COPY_AT)); //                          CALL copie
    code.extend(ld16(0x21, MAP_AT)); //                           LD HL, carte
    code.extend(ld16(0x11, 0x9800)); //                           LD DE, carte de fond
    code.extend(ld16(0x01, map.len() as u16)); //                 LD BC, taille
    code.extend(ld16(0xCD, COPY_AT)); //                          CALL copie
    code.extend([0x3E, 0xE4, 0xE0, 0x47]); //                     LD A,0xE4 ; LDH (BGP),A : palette normale
    code.extend([0x3E, 0x91, 0xE0, 0x40]); //                     LD A,0x91 ; LDH (LCDC),A : allume l'écran
    code.extend(ld16(0x21, MSG_AT)); //                           LD HL, message
    code.extend([
        0x2A, //       boucle : LD A,(HL+)
        0xB7, //       OR A (fin du texte ?)
        0x28, 0x08, // JR Z, fin
        0xE0, 0x01, // LDH (SB),A : octet à envoyer
        0x3E, 0x81, // LD A,0x81
        0xE0, 0x02, // LDH (SC),A : démarre le transfert série
        0x18, 0xF4, // JR boucle
        0x18, 0xFE, // fin : JR fin (boucle infinie)
    ]);
    assert!(0x150 + code.len() <= COPY_AT as usize, "le programme déborde sur la routine de copie");
    rom[0x150..0x150 + code.len()].copy_from_slice(&code);

    // routine de copie : copie BC octets de (HL) vers (DE)
    let copy = [
        0x2A, // LD A,(HL+)
        0x12, // LD (DE),A
        0x13, // INC DE
        0x0B, // DEC BC
        0x78, // LD A,B
        0xB1, // OR C
        0x20, 0xF8, // JR NZ, début
        0xC9, // RET
    ];
    rom[COPY_AT as usize..COPY_AT as usize + copy.len()].copy_from_slice(&copy);

    rom[TILES_AT as usize..TILES_AT as usize + tiles.len()].copy_from_slice(&tiles);
    rom[MAP_AT as usize..MAP_AT as usize + map.len()].copy_from_slice(&map);
    let msg = DEMO_MESSAGE.as_bytes();
    rom[MSG_AT as usize..MSG_AT as usize + msg.len()].copy_from_slice(msg);
    rom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::CartridgeInfo;
    use crate::gameboy::GameBoy;

    #[test]
    fn demo_rom_has_a_valid_header() {
        let info = CartridgeInfo::parse(&demo_rom()).unwrap();
        assert_eq!(info.title, DEMO_TITLE);
        assert!(info.logo_ok && info.checksum_ok && info.supported);
        assert_eq!((info.rom_kib, info.ram_kib, info.mapper_name), (32, 0, "ROM seule"));
    }

    #[test]
    fn demo_rom_prints_its_message_over_serial() {
        let mut gb = GameBoy::new(demo_rom()).unwrap();
        gb.run_frame();
        assert_eq!(gb.serial_text(), DEMO_MESSAGE);
        // et elle ne fait rien d'autre ensuite
        gb.run_frame();
        assert_eq!(gb.serial_text(), DEMO_MESSAGE);
        assert!(!gb.cpu.locked);
    }

    #[test]
    fn demo_rom_draws_its_title() {
        let mut gb = GameBoy::new(demo_rom()).unwrap();
        for _ in 0..3 {
            gb.run_frame();
        }
        let frame = gb.frame();
        let count = |y0: usize, y1: usize, shade: u8| (y0..y1).flat_map(|y| (0..160).map(move |x| (x, y))).filter(|&(x, y)| frame[y * 160 + x] == shade).count();
        assert!(count(72, 80, 3) > 80, "les lettres noires doivent apparaître sur la ligne du texte");
        assert!(count(56, 64, 2) > 40, "le damier gris doit apparaître au-dessus");
        assert_eq!(count(0, 48, 3) + count(0, 48, 2), 0, "le haut de l'écran reste blanc");
    }

    #[test]
    fn too_small_files_are_rejected() {
        assert!(CartridgeInfo::parse(&[0u8; 100]).is_err());
    }

    #[test]
    fn mbc1_switches_rom_banks() {
        // 4 banques de 16 Kio ; le dernier octet de chaque banque vaut son numéro
        let mut rom = vec![0u8; 0x10000];
        rom[0x147] = 0x01; // MBC1
        rom[0x148] = 0x01; // 64 Kio
        for bank in 0..4 {
            rom[bank * 0x4000 + 0x3FFF] = 0xA0 + bank as u8;
        }
        let mut cart = crate::cartridge::Cartridge::new(rom).unwrap();
        assert_eq!(cart.read_rom(0x7FFF), 0xA1); // banque 1 par défaut
        cart.write_rom(0x2000, 2);
        assert_eq!(cart.read_rom(0x7FFF), 0xA2);
        cart.write_rom(0x2000, 0); // 0 devient 1
        assert_eq!(cart.read_rom(0x7FFF), 0xA1);
        assert_eq!(cart.read_rom(0x3FFF), 0xA0); // la banque 0 reste fixe en bas
    }

    #[test]
    fn vblank_interrupt_is_requested_every_frame() {
        let mut gb = GameBoy::new(demo_rom()).unwrap();
        gb.run_frame();
        assert!(crate::bus::Bus::read(&mut gb.mmu, 0xFF0F) & 1 == 1);
    }
}
