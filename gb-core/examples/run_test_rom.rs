//! Lance une ROM de test (celles de Blargg) sans fenêtre et affiche son verdict.
//! Usage : cargo run --release -p gb-core --example run_test_rom -- <rom.gb> [secondes d'émulation]
//! Code de sortie : 0 si réussi, 1 si échec, 2 si aucun verdict à temps.
//!
//! Les ROMs de Blargg donnent leur verdict de deux façons :
//!  - sur la liaison série, un texte qui contient « Passed » ou « Failed » ;
//!  - dans la RAM de la cartouche : la signature DE B0 61 en 0xA001, un code en 0xA000 (0x80 = en cours,
//!    0 = réussi, autre = échec) et un texte à partir de 0xA004.

use gb_core::{GameBoy, CYCLES_PER_FRAME};

/// Le verdict écrit dans la RAM de la cartouche, s'il existe : (code, texte).
fn ram_report(gb: &GameBoy) -> Option<(u8, String)> {
    let ram = gb.mmu.cart.ram();
    if ram.len() < 0x10 || ram[1..4] != [0xDE, 0xB0, 0x61] {
        return None;
    }
    let text: Vec<u8> = ram[4..].iter().copied().take_while(|&b| b != 0).collect();
    Some((ram[0], String::from_utf8_lossy(&text).into_owned()))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = std::fs::read(&args[1]).expect("ROM illisible");
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let mut gb = GameBoy::new(rom).expect("ROM invalide");

    let mut verdict = None;
    for _ in 0..seconds * 60 {
        gb.run_frame();
        let _ = gb.take_audio(); // on ne joue pas le son : on évite juste qu'il s'accumule
        let text = gb.serial_text();
        if text.contains("Passed") {
            verdict = Some(0);
            break;
        }
        if text.contains("Failed") {
            verdict = Some(1);
            break;
        }
        if let Some((code, _)) = ram_report(&gb) {
            if code != 0x80 {
                verdict = Some(if code == 0 { 0 } else { 1 });
                break;
            }
        }
        if gb.cpu.locked {
            println!("(le processeur s'est figé sur une instruction illégale, PC = {:04X})", gb.cpu.pc);
            break;
        }
    }
    let serial = gb.serial_text();
    if !serial.trim().is_empty() {
        println!("{}", serial.trim_end());
    }
    if let Some((code, text)) = ram_report(&gb) {
        println!("[RAM cartouche : code {code:#04X}]\n{}", text.trim_end());
    }
    let emulated = gb.total_cycles as f64 / (CYCLES_PER_FRAME as f64 * 60.0);
    match verdict {
        Some(0) => println!("--> RÉUSSI ({emulated:.1} s émulées)"),
        Some(_) => println!("--> ÉCHEC ({emulated:.1} s émulées)"),
        None => println!("--> sans verdict après {emulated:.1} s émulées (PC = {:04X})", gb.cpu.pc),
    }
    std::process::exit(verdict.unwrap_or(2));
}
