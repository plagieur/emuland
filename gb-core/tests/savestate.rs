//! Les états sauvegardés doivent restituer la console à l'identique : après un rechargement,
//! la suite du jeu se déroule exactement comme la première fois.

use gb_core::demo::demo_rom;
use gb_core::{Bus, GameBoy};

fn run(gb: &mut GameBoy, frames: usize) {
    for _ in 0..frames {
        gb.run_frame();
        let _ = gb.take_audio();
    }
}

/// Un résumé de ce qui est visible et audible : l'image, le texte série et l'état du processeur.
fn fingerprint(gb: &GameBoy) -> (Vec<u8>, String, u16, u16, u64) {
    (gb.frame().to_vec(), gb.serial_text(), gb.cpu.pc, gb.cpu.sp, gb.total_cycles)
}

#[test]
fn reloading_a_state_replays_the_same_future() {
    let mut gb = GameBoy::new(demo_rom()).unwrap();
    run(&mut gb, 3);
    let state = gb.save_state();

    run(&mut gb, 10);
    let first_future = fingerprint(&gb);

    gb.load_state(&state).unwrap();
    run(&mut gb, 10);
    assert_eq!(fingerprint(&gb), first_future, "la suite doit être identique image par image");
}

#[test]
fn a_state_can_be_loaded_into_a_fresh_console() {
    let mut gb = GameBoy::new(demo_rom()).unwrap();
    run(&mut gb, 4);
    let state = gb.save_state();
    run(&mut gb, 6);
    let expected = fingerprint(&gb);

    let mut other = GameBoy::new(demo_rom()).unwrap();
    other.load_state(&state).unwrap();
    run(&mut other, 6);
    assert_eq!(fingerprint(&other), expected);
}

#[test]
fn a_state_from_another_rom_is_refused_and_leaves_the_console_untouched() {
    let mut a = GameBoy::new(demo_rom()).unwrap();
    run(&mut a, 2);
    let state = a.save_state();

    let mut other_rom = demo_rom();
    other_rom[0x134] = b'X'; // un autre jeu
    let mut b = GameBoy::new(other_rom).unwrap();
    run(&mut b, 2);
    let before = fingerprint(&b);
    let err = b.load_state(&state).unwrap_err();
    assert!(err.contains("another ROM"), "{err}");
    assert_eq!(fingerprint(&b), before);
}

#[test]
fn damaged_states_are_refused() {
    let mut gb = GameBoy::new(demo_rom()).unwrap();
    run(&mut gb, 2);
    let state = gb.save_state();
    let before = fingerprint(&gb);

    assert!(gb.load_state(&[]).is_err());
    assert!(gb.load_state(b"n'importe quoi").is_err());
    assert!(gb.load_state(&state[..state.len() / 2]).is_err(), "état tronqué");
    let mut too_long = state.clone();
    too_long.push(0);
    assert!(gb.load_state(&too_long).is_err(), "données en trop");
    let mut wrong_version = state.clone();
    wrong_version[8] = 99; // l'octet de version suit les 4 octets de longueur et les 4 de la signature
    assert!(gb.load_state(&wrong_version).unwrap_err().contains("another emuland version"));
    assert_eq!(fingerprint(&gb), before, "après un échec, la console n'a pas bougé");
}

#[test]
fn cartridge_ram_and_banks_survive_a_state() {
    // une cartouche MBC3 + horloge + pile, assez grande pour avoir plusieurs banques
    let mut rom = vec![0u8; 8 * 0x4000];
    rom[0x147] = 0x10;
    rom[0x148] = 2;
    rom[0x149] = 3;
    for b in 0..8 {
        rom[b * 0x4000 + 0x3FFF] = b as u8;
    }
    let mut gb = GameBoy::new(rom).unwrap();
    gb.mmu.write(0x0000, 0x0A); // active la RAM
    gb.mmu.write(0x2000, 5); // banque de ROM 5
    gb.mmu.write(0x4000, 2); // banque de RAM 2
    gb.mmu.write(0xA123, 0x5A);
    let state = gb.save_state();

    gb.mmu.write(0x2000, 1);
    gb.mmu.write(0x4000, 0);
    gb.mmu.write(0xA123, 0x00);
    assert_ne!(gb.mmu.read(0x7FFF), 5);

    gb.load_state(&state).unwrap();
    assert_eq!(gb.mmu.read(0x7FFF), 5, "la banque de ROM revient");
    assert_eq!(gb.mmu.read(0xA123), 0x5A, "la RAM de la banque 2 revient");
}

#[test]
fn a_blargg_test_continues_correctly_after_a_reload() {
    // si la ROM de test est présente : on la coupe en deux avec un état, et elle doit quand même réussir
    let path: std::path::PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "test-roms", "cpu_instrs", "09-op r,r.gb"].iter().collect();
    let Ok(rom) = std::fs::read(&path) else {
        eprintln!("ROM absente, test ignoré");
        return;
    };
    let mut gb = GameBoy::new(rom).unwrap();
    run(&mut gb, 60 * 3); // 3 secondes : le test est en cours
    assert!(!gb.serial_text().contains("Passed"));
    let state = gb.save_state();
    run(&mut gb, 60 * 40);
    assert!(gb.serial_text().contains("Passed"));

    let mut resumed = GameBoy::new(std::fs::read(&path).unwrap()).unwrap();
    resumed.load_state(&state).unwrap();
    run(&mut resumed, 60 * 40);
    assert!(resumed.serial_text().contains("Passed"), "après rechargement : {}", resumed.serial_text());
}
