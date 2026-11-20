//! Lance une ROM un certain nombre d'images, puis écrit la dernière image dans un fichier :
//! 160 × 144 octets (nuances 0 à 3) pour une Game Boy, ou 160 × 144 mots de 16 bits (couleurs 15 bits) pour une Game Boy Color.
//! Usage : cargo run --release -p gb-core --example dump_frame -- <rom> <images> <sortie.bin>

use gb_core::GameBoy;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = std::fs::read(&args[1]).expect("ROM illisible");
    let frames: usize = args[2].parse().expect("nombre d'images");
    let mut gb = GameBoy::new(rom).expect("ROM invalide");
    for _ in 0..frames {
        gb.run_frame();
        let _ = gb.take_audio();
    }
    let bytes: Vec<u8> = if gb.is_cgb() { gb.frame_color().iter().flat_map(|c| c.to_le_bytes()).collect() } else { gb.frame().to_vec() };
    std::fs::write(&args[3], bytes).expect("écriture impossible");
    println!("{} images, mode {}, PC = {:04X}", frames, if gb.is_cgb() { "Color" } else { "d'origine" }, gb.cpu.pc);
}
