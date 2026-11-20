//! Les ROMs de test de Blargg (et dmg-acid2) : des programmes écrits pour vérifier un émulateur de Game Boy.
//! Elles ne sont pas dans le dépôt : on les télécharge dans `test-roms/` (voir le LISEZMOI de ce dossier).
//! Elles sont longues à lancer en mode debug, donc ignorées par défaut. Pour les lancer :
//!     cargo test --release -p gb-core --test blargg -- --ignored

use gb_core::GameBoy;
use std::path::PathBuf;

fn rom_path(rel: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "test-roms", rel].iter().collect()
}

/// Les images de référence (petites, fournies avec le dépôt) : celles des projets dmg-acid2 et cgb-acid2.
fn data_path(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "data", name].iter().collect()
}

/// Le verdict que les ROMs récentes écrivent dans la RAM de la cartouche : (code, texte). 0x80 = toujours en cours.
fn ram_report(gb: &GameBoy) -> Option<(u8, String)> {
    let ram = gb.mmu.cart.ram();
    if ram.len() < 0x10 || ram[1..4] != [0xDE, 0xB0, 0x61] {
        return None;
    }
    let text: Vec<u8> = ram[4..].iter().copied().take_while(|&b| b != 0).collect();
    Some((ram[0], String::from_utf8_lossy(&text).into_owned()))
}

/// Lance une ROM jusqu'à ce qu'elle donne son verdict (texte série « Passed »/« Failed » ou code en RAM).
fn run(rel: &str, max_seconds: u64) {
    let path = rom_path(rel);
    let Ok(rom) = std::fs::read(&path) else {
        eprintln!("ROM absente, test ignoré : {}", path.display());
        return;
    };
    let mut gb = GameBoy::new(rom).unwrap();
    for _ in 0..max_seconds * 60 {
        gb.run_frame();
        let _ = gb.take_audio();
        let text = gb.serial_text();
        if text.contains("Passed") {
            return;
        }
        assert!(!text.contains("Failed"), "{rel} a échoué :\n{text}");
        if let Some((code, text)) = ram_report(&gb) {
            if code != 0x80 {
                assert_eq!(code, 0, "{rel} a échoué (code {code:#04X}) :\n{text}");
                return;
            }
        }
        assert!(!gb.cpu.locked, "{rel} : le processeur s'est figé (PC = {:04X})", gb.cpu.pc);
    }
    panic!("{rel} : pas de verdict après {max_seconds} s émulées :\n{}", gb.serial_text());
}

macro_rules! blargg {
    ($($name:ident: $file:expr, $secs:expr;)*) => {
        $(
            #[test]
            #[ignore]
            fn $name() {
                run($file, $secs);
            }
        )*
    };
}

blargg! {
    cpu_01_special: "cpu_instrs/01-special.gb", 30;
    cpu_02_interrupts: "cpu_instrs/02-interrupts.gb", 30;
    cpu_03_op_sp_hl: "cpu_instrs/03-op sp,hl.gb", 30;
    cpu_04_op_r_imm: "cpu_instrs/04-op r,imm.gb", 30;
    cpu_05_op_rp: "cpu_instrs/05-op rp.gb", 30;
    cpu_06_ld_r_r: "cpu_instrs/06-ld r,r.gb", 30;
    cpu_07_jumps: "cpu_instrs/07-jr,jp,call,ret,rst.gb", 30;
    cpu_08_misc: "cpu_instrs/08-misc instrs.gb", 30;
    cpu_09_op_r_r: "cpu_instrs/09-op r,r.gb", 60;
    cpu_10_bit_ops: "cpu_instrs/10-bit ops.gb", 60;
    cpu_11_op_a_hl: "cpu_instrs/11-op a,(hl).gb", 60;
    instr_timing: "instr_timing/instr_timing.gb", 30;
    halt_bug: "halt_bug.gb", 30;
    mem_timing_read: "mem_timing/individual/01-read_timing.gb", 30;
    mem_timing_write: "mem_timing/individual/02-write_timing.gb", 30;
    mem_timing_modify: "mem_timing/individual/03-modify_timing.gb", 30;
    mem_timing2_read: "mem_timing-2/rom_singles/01-read_timing.gb", 30;
    mem_timing2_write: "mem_timing-2/rom_singles/02-write_timing.gb", 30;
    mem_timing2_modify: "mem_timing-2/rom_singles/03-modify_timing.gb", 30;
    sound_01_registers: "dmg_sound/rom_singles/01-registers.gb", 30;
    sound_02_len_ctr: "dmg_sound/rom_singles/02-len ctr.gb", 30;
    sound_03_trigger: "dmg_sound/rom_singles/03-trigger.gb", 30;
    sound_04_sweep: "dmg_sound/rom_singles/04-sweep.gb", 30;
    sound_05_sweep_details: "dmg_sound/rom_singles/05-sweep details.gb", 30;
    sound_06_overflow_on_trigger: "dmg_sound/rom_singles/06-overflow on trigger.gb", 30;
    sound_07_len_sweep_period_sync: "dmg_sound/rom_singles/07-len sweep period sync.gb", 30;
    sound_08_len_ctr_during_power: "dmg_sound/rom_singles/08-len ctr during power.gb", 30;
    sound_11_regs_after_power: "dmg_sound/rom_singles/11-regs after power.gb", 30;
    oam_bug_3_non_causes: "oam_bug/rom_singles/3-non_causes.gb", 30;
    oam_bug_6_timing_no_bug: "oam_bug/rom_singles/6-timing_no_bug.gb", 30;
    // Game Boy Color : vitesse double
    cgb_interrupt_time: "interrupt_time/interrupt_time.gb", 30;
    // Game Boy Color : le son
    cgb_sound_01_registers: "cgb_sound/rom_singles/01-registers.gb", 30;
    cgb_sound_02_len_ctr: "cgb_sound/rom_singles/02-len ctr.gb", 30;
    cgb_sound_03_trigger: "cgb_sound/rom_singles/03-trigger.gb", 40;
    cgb_sound_04_sweep: "cgb_sound/rom_singles/04-sweep.gb", 30;
    cgb_sound_05_sweep_details: "cgb_sound/rom_singles/05-sweep details.gb", 30;
    cgb_sound_06_overflow_on_trigger: "cgb_sound/rom_singles/06-overflow on trigger.gb", 30;
    cgb_sound_07_len_sweep_period_sync: "cgb_sound/rom_singles/07-len sweep period sync.gb", 30;
    cgb_sound_08_len_ctr_during_power: "cgb_sound/rom_singles/08-len ctr during power.gb", 30;
    cgb_sound_10_wave_trigger_while_on: "cgb_sound/rom_singles/10-wave trigger while on.gb", 30;
    cgb_sound_11_regs_after_power: "cgb_sound/rom_singles/11-regs after power.gb", 30;
    cgb_sound_12_wave: "cgb_sound/rom_singles/12-wave.gb", 30;
}

/// cgb-acid2 : l'image en couleurs doit être identique, pixel pour pixel, à l'image de référence.
#[test]
#[ignore]
fn cgb_acid2_matches_the_reference_image() {
    let (rom, reference) = match (std::fs::read(rom_path("cgb-acid2/cgb-acid2.gbc")), std::fs::read(data_path("cgb-acid2-reference.bin"))) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            eprintln!("cgb-acid2 absent, test ignoré");
            return;
        }
    };
    let mut gb = GameBoy::new(rom).unwrap();
    assert!(gb.is_cgb(), "cgb-acid2 doit tourner en mode Game Boy Color");
    for _ in 0..120 {
        gb.run_frame();
        let _ = gb.take_audio();
    }
    let expected: Vec<u16> = reference.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
    let different = gb.frame_color().iter().zip(&expected).filter(|(a, b)| a != b).count();
    assert_eq!(different, 0, "{different} pixels diffèrent de l'image de référence");
}

/// dmg-acid2 : l'image dessinée doit être identique, pixel pour pixel, à l'image de référence.
#[test]
#[ignore]
fn dmg_acid2_matches_the_reference_image() {
    let (rom, reference) = match (std::fs::read(rom_path("dmg-acid2/dmg-acid2.gb")), std::fs::read(data_path("dmg-acid2-reference.bin"))) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            eprintln!("dmg-acid2 absent, test ignoré");
            return;
        }
    };
    let mut gb = GameBoy::new(rom).unwrap();
    for _ in 0..120 {
        gb.run_frame();
        let _ = gb.take_audio();
    }
    let different = gb.frame().iter().zip(&reference).filter(|(a, b)| a != b).count();
    assert_eq!(different, 0, "{different} pixels diffèrent de l'image de référence");
}
