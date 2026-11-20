//! Le PPU (Pixel Processing Unit) : la puce qui dessine l'écran de la Game Boy.
//!
//! Écran de 160 × 144 pixels, 4 nuances. Chaque ligne dure 456 « dots » (T-cycles) :
//! mode 2 (lecture de l'OAM, 80 dots), mode 3 (dessin, ~172 dots), mode 0 (HBlank, le reste).
//! Après les 144 lignes visibles viennent 10 lignes de VBlank (mode 1), soit 154 lignes par image.
//!
//! Simplification : la ligne est dessinée d'un coup à la fin du mode 3, avec les registres
//! tels qu'ils sont à cet instant (pas de changement en cours de ligne). Les restrictions d'accès
//! à la VRAM et à l'OAM selon le mode ne sont pas émulées.

pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 144;

const DOTS_PER_LINE: u32 = 456;
const MODE2_END: u32 = 80;
const MODE3_END: u32 = 252;

pub const IRQ_VBLANK: u8 = 0x01;
pub const IRQ_STAT: u8 = 0x02;

/// Taille d'une banque de VRAM (la Game Boy Color en a deux).
pub const VRAM_BANK: usize = 0x2000;

pub struct Ppu {
    /// Mémoire vidéo : deux banques de 8 Kio (la deuxième ne sert qu'en mode Game Boy Color).
    pub vram: Vec<u8>,
    pub oam: Vec<u8>,
    /// Mode Game Boy Color : couleurs, deux banques de VRAM, attributs de tuiles.
    cgb: bool,
    /// Banque de VRAM visible par le processeur (registre VBK, 0xFF4F).
    vram_bank: usize,
    /// Palettes de couleurs : 8 palettes de 4 couleurs de 2 octets (15 bits RVB) pour le fond et pour les sprites.
    bg_pal: [u8; 64],
    obj_pal: [u8; 64],
    bcps: u8,
    ocps: u8,
    /// Priorité entre sprites : 0 = par numéro dans l'OAM (mode Color), 1 = par position (mode d'origine).
    opri: u8,
    bg_attr_prio: [bool; WIDTH],
    /// Image en couleurs (15 bits par pixel) en cours de dessin et dernière image terminée.
    front_color: Vec<u16>,
    pub frame_color: Vec<u16>,
    /// L'écran vient d'entrer en HBlank sur une ligne visible (pour la copie HDMA).
    hblank_entered: bool,
    pub lcdc: u8,
    pub stat: u8,
    pub scy: u8,
    pub scx: u8,
    pub ly: u8,
    pub lyc: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,
    dot: u32,
    window_line: u8,
    stat_line: bool,
    bg_ids: [u8; WIDTH],
    /// Image en cours de dessin (nuances 0 à 3).
    front: Vec<u8>,
    /// Dernière image terminée : c'est elle qu'on affiche.
    pub frame: Vec<u8>,
    frame_ready: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            vram: vec![0; VRAM_BANK * 2],
            oam: vec![0; 0xA0],
            cgb: false,
            vram_bank: 0,
            bg_pal: [0xFF; 64],
            obj_pal: [0xFF; 64],
            bcps: 0,
            ocps: 0,
            opri: 1,
            bg_attr_prio: [false; WIDTH],
            front_color: vec![0x7FFF; WIDTH * HEIGHT],
            frame_color: vec![0x7FFF; WIDTH * HEIGHT],
            hblank_entered: false,
            lcdc: 0x91,
            stat: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            dot: 0,
            window_line: 0,
            stat_line: false,
            bg_ids: [0; WIDTH],
            front: vec![0; WIDTH * HEIGHT],
            frame: vec![0; WIDTH * HEIGHT],
            frame_ready: false,
        }
    }

    pub fn lcd_on(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    /// Passe l'écran en mode Game Boy Color (à faire une seule fois, avant de lancer le jeu).
    pub fn set_cgb(&mut self, on: bool) {
        self.cgb = on;
        self.opri = if on { 0 } else { 1 };
    }

    pub fn is_cgb(&self) -> bool {
        self.cgb
    }

    /// La banque de VRAM visible par le processeur : décalage dans `vram`.
    pub fn vram_offset(&self) -> usize {
        self.vram_bank * VRAM_BANK
    }

    /// Vrai (une seule fois) quand l'écran vient d'entrer en HBlank sur une ligne visible.
    pub fn take_hblank(&mut self) -> bool {
        std::mem::take(&mut self.hblank_entered)
    }

    /// Les palettes de couleurs ne sont accessibles que hors du dessin (mode 3), comme la VRAM.
    fn palette_accessible(&self) -> bool {
        self.vram_accessible()
    }

    /// La couleur (15 bits) numéro `color` de la palette `pal` du fond (`obj = false`) ou des sprites.
    fn palette_color(&self, obj: bool, pal: usize, color: usize) -> u16 {
        let table = if obj { &self.obj_pal } else { &self.bg_pal };
        let i = (pal * 4 + color) * 2;
        u16::from_le_bytes([table[i], table[i + 1]]) & 0x7FFF
    }

    pub fn mode(&self) -> u8 {
        if !self.lcd_on() {
            0
        } else if self.ly >= 144 {
            1
        } else if self.dot < MODE2_END {
            2
        } else if self.dot < MODE3_END {
            3
        } else {
            0
        }
    }

    /// La VRAM est lisible et modifiable par le processeur, sauf pendant que l'écran la lit pour dessiner (mode 3).
    pub fn vram_accessible(&self) -> bool {
        !self.lcd_on() || self.mode() != 3
    }

    /// L'OAM est accessible seulement pendant le HBlank et le VBlank (modes 0 et 1) quand l'écran est allumé.
    pub fn oam_accessible(&self) -> bool {
        !self.lcd_on() || self.mode() < 2
    }

    /// Vrai (une seule fois) quand une nouvelle image vient d'être terminée.
    pub fn take_frame_ready(&mut self) -> bool {
        std::mem::take(&mut self.frame_ready)
    }

    pub fn read_reg(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.lcdc,
            0xFF41 => 0x80 | (self.stat & 0x78) | ((self.ly == self.lyc) as u8) << 2 | self.mode(),
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            // registres propres à la Game Boy Color
            0xFF4F if self.cgb => 0xFE | self.vram_bank as u8,
            0xFF68 if self.cgb => 0x40 | self.bcps,
            0xFF69 if self.cgb => if self.palette_accessible() { self.bg_pal[(self.bcps & 0x3F) as usize] } else { 0xFF },
            0xFF6A if self.cgb => 0x40 | self.ocps,
            0xFF6B if self.cgb => if self.palette_accessible() { self.obj_pal[(self.ocps & 0x3F) as usize] } else { 0xFF },
            0xFF6C if self.cgb => 0xFE | self.opri,
            _ => 0xFF,
        }
    }

    pub fn write_reg(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF40 => {
                let was_on = self.lcd_on();
                self.lcdc = value;
                if was_on && !self.lcd_on() {
                    // écran éteint : retour à la ligne 0 et image blanche
                    self.ly = 0;
                    self.dot = 0;
                    self.window_line = 0;
                    self.stat_line = false;
                    self.front.fill(0);
                    self.frame.fill(0);
                    self.front_color.fill(0x7FFF);
                    self.frame_color.fill(0x7FFF);
                } else if !was_on && self.lcd_on() {
                    self.ly = 0;
                    self.dot = 0;
                }
            }
            0xFF41 => self.stat = value & 0x78,
            0xFF42 => self.scy = value,
            0xFF43 => self.scx = value,
            0xFF45 => self.lyc = value,
            0xFF47 => self.bgp = value,
            0xFF48 => self.obp0 = value,
            0xFF49 => self.obp1 = value,
            0xFF4A => self.wy = value,
            0xFF4B => self.wx = value,
            0xFF4F if self.cgb => self.vram_bank = (value & 1) as usize,
            0xFF68 if self.cgb => self.bcps = value & 0xBF,
            0xFF69 if self.cgb => {
                if self.palette_accessible() {
                    self.bg_pal[(self.bcps & 0x3F) as usize] = value;
                }
                if self.bcps & 0x80 != 0 {
                    self.bcps = 0x80 | (self.bcps.wrapping_add(1) & 0x3F); // l'index avance même quand l'écriture est perdue
                }
            }
            0xFF6A if self.cgb => self.ocps = value & 0xBF,
            0xFF6B if self.cgb => {
                if self.palette_accessible() {
                    self.obj_pal[(self.ocps & 0x3F) as usize] = value;
                }
                if self.ocps & 0x80 != 0 {
                    self.ocps = 0x80 | (self.ocps.wrapping_add(1) & 0x3F);
                }
            }
            0xFF6C if self.cgb => self.opri = value & 1,
            _ => {}
        }
    }

    /// Avance de `cycles` dots. Renvoie les bits d'interruption à ajouter à IF.
    pub fn tick(&mut self, cycles: u32) -> u8 {
        if !self.lcd_on() {
            return 0;
        }
        let mut irq = 0;
        for _ in 0..cycles {
            self.dot += 1;
            if self.dot == DOTS_PER_LINE {
                self.dot = 0;
                self.ly += 1;
                if self.ly == 154 {
                    self.ly = 0;
                    self.window_line = 0;
                }
                if self.ly == 144 {
                    irq |= IRQ_VBLANK;
                    self.frame.copy_from_slice(&self.front);
                    self.frame_color.copy_from_slice(&self.front_color);
                    self.frame_ready = true;
                }
            } else if self.dot == MODE3_END && self.ly < 144 {
                if self.cgb {
                    self.render_line_cgb();
                } else {
                    self.render_line();
                }
                self.hblank_entered = true;
            }
            // la ligne STAT : une interruption n'est demandée que quand elle passe de faux à vrai
            let mode = self.mode();
            let line = (mode == 0 && self.stat & 0x08 != 0)
                || (mode == 1 && self.stat & 0x10 != 0)
                || (mode == 2 && self.stat & 0x20 != 0)
                || (self.ly == self.lyc && self.stat & 0x40 != 0);
            if line && !self.stat_line {
                irq |= IRQ_STAT;
            }
            self.stat_line = line;
        }
        irq
    }

    /// Les 2 bits de couleur du pixel (`col`, `row`) d'une tuile de 8 × 8 dont les données commencent à `addr`.
    fn tile_pixel(&self, addr: usize, row: usize, col: usize) -> u8 {
        let lo = self.vram[(addr + row * 2) & 0x1FFF];
        let hi = self.vram[(addr + row * 2 + 1) & 0x1FFF];
        let bit = 7 - col;
        ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1)
    }

    fn render_line(&mut self) {
        let ly = self.ly as usize;
        let base = ly * WIDTH;
        let mut window_used = false;

        for x in 0..WIDTH {
            let (mut id, mut shade) = (0u8, 0u8);
            if self.lcdc & 0x01 != 0 {
                let in_window = self.lcdc & 0x20 != 0 && self.ly >= self.wy && x as i32 + 7 >= self.wx as i32;
                let (map, px, py) = if in_window {
                    window_used = true;
                    let map = if self.lcdc & 0x40 != 0 { 0x1C00 } else { 0x1800 };
                    (map, (x as i32 + 7 - self.wx as i32) as usize, self.window_line as usize)
                } else {
                    let map = if self.lcdc & 0x08 != 0 { 0x1C00 } else { 0x1800 };
                    (map, (x + self.scx as usize) & 0xFF, (ly + self.scy as usize) & 0xFF)
                };
                let tile = self.vram[map + (py / 8) * 32 + px / 8] as usize;
                let addr = if self.lcdc & 0x10 != 0 { tile * 16 } else { (0x1000 + (tile as u8 as i8 as i32) * 16) as usize };
                id = self.tile_pixel(addr, py % 8, px % 8);
                shade = (self.bgp >> (id * 2)) & 3;
            }
            self.bg_ids[x] = id;
            self.front[base + x] = shade;
        }
        if window_used {
            self.window_line += 1;
        }

        if self.lcdc & 0x02 != 0 {
            self.render_sprites(ly, base);
        }
    }

    fn render_sprites(&mut self, ly: usize, base: usize) {
        let height: i32 = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        // le matériel ne montre que les 10 premiers sprites (dans l'ordre de l'OAM) qui touchent la ligne
        let mut sprites: Vec<(i32, usize)> = Vec::with_capacity(10);
        for i in 0..40 {
            let y = self.oam[i * 4] as i32 - 16;
            if (ly as i32) >= y && (ly as i32) < y + height {
                sprites.push((self.oam[i * 4 + 1] as i32 - 8, i));
                if sprites.len() == 10 {
                    break;
                }
            }
        }
        // priorité : le plus à gauche gagne, puis le plus petit numéro dans l'OAM
        sprites.sort();

        for px in 0..WIDTH as i32 {
            for &(sx, i) in &sprites {
                if px < sx || px >= sx + 8 {
                    continue;
                }
                let y = self.oam[i * 4] as i32 - 16;
                let mut tile = self.oam[i * 4 + 2] as usize;
                let flags = self.oam[i * 4 + 3];
                let mut row = ly as i32 - y;
                if flags & 0x40 != 0 {
                    row = height - 1 - row;
                }
                if height == 16 {
                    tile &= 0xFE;
                    if row >= 8 {
                        tile += 1;
                        row -= 8;
                    }
                }
                let mut col = (px - sx) as usize;
                if flags & 0x20 != 0 {
                    col = 7 - col;
                }
                let id = self.tile_pixel(tile * 16, row as usize, col);
                if id == 0 {
                    continue; // transparent : un sprite moins prioritaire peut apparaître
                }
                if flags & 0x80 == 0 || self.bg_ids[px as usize] == 0 {
                    let palette = if flags & 0x10 != 0 { self.obp1 } else { self.obp0 };
                    self.front[base + px as usize] = (palette >> (id * 2)) & 3;
                }
                break;
            }
        }
    }
}

/// Le dessin en mode Game Boy Color : tuiles avec attributs (palette, banque, retournement, priorité),
/// 8 palettes de fond et 8 de sprites, deux banques de VRAM.
impl Ppu {
    fn tile_pixel_bank(&self, bank: usize, addr: usize, row: usize, col: usize) -> u8 {
        let base = bank * VRAM_BANK;
        let lo = self.vram[base + ((addr + row * 2) & 0x1FFF)];
        let hi = self.vram[base + ((addr + row * 2 + 1) & 0x1FFF)];
        let bit = 7 - col;
        ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1)
    }

    /// Une nuance de 0 (clair) à 3 (foncé) approchée d'une couleur 15 bits : pour les outils qui ne regardent que le gris.
    fn shade_of(color: u16) -> u8 {
        let (r, g, b) = ((color & 31) as u32, ((color >> 5) & 31) as u32, ((color >> 10) & 31) as u32);
        let lum = (r * 30 + g * 59 + b * 11) / 100; // 0..31
        3 - (lum * 4 / 32).min(3) as u8
    }

    fn render_line_cgb(&mut self) {
        let ly = self.ly as usize;
        let base = ly * WIDTH;
        let mut window_used = false;
        let sprites_always_on_top = self.lcdc & 0x01 == 0; // en mode Color, le bit 0 ne coupe pas le fond : il coupe les priorités

        for x in 0..WIDTH {
            let in_window = self.lcdc & 0x20 != 0 && self.ly >= self.wy && x as i32 + 7 >= self.wx as i32;
            let (map, px, py) = if in_window {
                window_used = true;
                let map = if self.lcdc & 0x40 != 0 { 0x1C00 } else { 0x1800 };
                (map, (x as i32 + 7 - self.wx as i32) as usize, self.window_line as usize)
            } else {
                let map = if self.lcdc & 0x08 != 0 { 0x1C00 } else { 0x1800 };
                (map, (x + self.scx as usize) & 0xFF, (ly + self.scy as usize) & 0xFF)
            };
            let map_index = map + (py / 8) * 32 + px / 8;
            let tile = self.vram[map_index] as usize;
            let attr = self.vram[VRAM_BANK + map_index]; // les attributs sont dans la banque 1, à la même adresse
            let bank = ((attr >> 3) & 1) as usize;
            let addr = if self.lcdc & 0x10 != 0 { tile * 16 } else { (0x1000 + (tile as u8 as i8 as i32) * 16) as usize };
            let (mut row, mut col) = (py % 8, px % 8);
            if attr & 0x40 != 0 {
                row = 7 - row;
            }
            if attr & 0x20 != 0 {
                col = 7 - col;
            }
            let id = self.tile_pixel_bank(bank, addr, row, col);
            let color = self.palette_color(false, (attr & 7) as usize, id as usize);
            self.bg_ids[x] = id;
            self.bg_attr_prio[x] = attr & 0x80 != 0 && !sprites_always_on_top;
            self.front_color[base + x] = color;
            self.front[base + x] = Self::shade_of(color);
        }
        if window_used {
            self.window_line += 1;
        }
        if self.lcdc & 0x02 != 0 {
            self.render_sprites_cgb(ly, base);
        }
    }

    fn render_sprites_cgb(&mut self, ly: usize, base: usize) {
        let height: i32 = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        let master = self.lcdc & 0x01 != 0;
        let mut sprites: Vec<(i32, usize)> = Vec::with_capacity(10);
        for i in 0..40 {
            let y = self.oam[i * 4] as i32 - 16;
            if (ly as i32) >= y && (ly as i32) < y + height {
                sprites.push((self.oam[i * 4 + 1] as i32 - 8, i));
                if sprites.len() == 10 {
                    break;
                }
            }
        }
        // mode Color : le plus petit numéro dans l'OAM gagne ; mode d'origine (OPRI = 1) : le plus à gauche
        if self.opri == 0 {
            sprites.sort_by_key(|&(_, i)| i);
        } else {
            sprites.sort();
        }

        for px in 0..WIDTH as i32 {
            for &(sx, i) in &sprites {
                if px < sx || px >= sx + 8 {
                    continue;
                }
                let y = self.oam[i * 4] as i32 - 16;
                let mut tile = self.oam[i * 4 + 2] as usize;
                let flags = self.oam[i * 4 + 3];
                let mut row = ly as i32 - y;
                if flags & 0x40 != 0 {
                    row = height - 1 - row;
                }
                if height == 16 {
                    tile &= 0xFE;
                    if row >= 8 {
                        tile += 1;
                        row -= 8;
                    }
                }
                let mut col = (px - sx) as usize;
                if flags & 0x20 != 0 {
                    col = 7 - col;
                }
                let bank = ((flags >> 3) & 1) as usize;
                let id = self.tile_pixel_bank(bank, tile * 16, row as usize, col);
                if id == 0 {
                    continue;
                }
                // un sprite passe derrière le fond si le fond n'est pas transparent et que l'un des deux le demande
                let behind = master && self.bg_ids[px as usize] != 0 && (flags & 0x80 != 0 || self.bg_attr_prio[px as usize]);
                if !behind {
                    let color = self.palette_color(true, (flags & 7) as usize, id as usize);
                    self.front_color[base + px as usize] = color;
                    self.front[base + px as usize] = Self::shade_of(color);
                }
                break;
            }
        }
    }
}

impl Ppu {
    pub(crate) fn save_state(&self, w: &mut crate::state::StateWriter) {
        w.bool(self.cgb);
        w.u8(self.vram_bank as u8);
        w.bytes(&self.bg_pal);
        w.bytes(&self.obj_pal);
        w.u8(self.bcps);
        w.u8(self.ocps);
        w.u8(self.opri);
        for plane in [&self.front_color, &self.frame_color] {
            let bytes: Vec<u8> = plane.iter().flat_map(|c| c.to_le_bytes()).collect();
            w.bytes(&bytes);
        }
        w.bytes(&self.vram);
        w.bytes(&self.oam);
        for v in [self.lcdc, self.stat, self.scy, self.scx, self.ly, self.lyc, self.bgp, self.obp0, self.obp1, self.wy, self.wx] {
            w.u8(v);
        }
        w.u32(self.dot);
        w.u8(self.window_line);
        w.bool(self.stat_line);
        w.bytes(&self.front);
        w.bytes(&self.frame);
        w.bool(self.frame_ready);
    }

    pub(crate) fn load_state(&mut self, r: &mut crate::state::StateReader) -> Result<(), String> {
        let cgb = r.bool()?;
        if cgb != self.cgb {
            return Err("this state comes from another mode (Game Boy or Game Boy Color)".into());
        }
        self.vram_bank = (r.u8()? & 1) as usize;
        r.bytes_into(&mut self.bg_pal)?;
        r.bytes_into(&mut self.obj_pal)?;
        self.bcps = r.u8()?;
        self.ocps = r.u8()?;
        self.opri = r.u8()? & 1;
        for plane in [&mut self.front_color, &mut self.frame_color] {
            let mut bytes = vec![0u8; WIDTH * HEIGHT * 2];
            r.bytes_into(&mut bytes)?;
            for (c, pair) in plane.iter_mut().zip(bytes.chunks_exact(2)) {
                *c = u16::from_le_bytes([pair[0], pair[1]]) & 0x7FFF;
            }
        }
        r.bytes_into(&mut self.vram)?;
        r.bytes_into(&mut self.oam)?;
        self.lcdc = r.u8()?;
        self.stat = r.u8()?;
        self.scy = r.u8()?;
        self.scx = r.u8()?;
        self.ly = r.u8()?;
        self.lyc = r.u8()?;
        self.bgp = r.u8()?;
        self.obp0 = r.u8()?;
        self.obp1 = r.u8()?;
        self.wy = r.u8()?;
        self.wx = r.u8()?;
        self.dot = r.u32()?;
        self.window_line = r.u8()?;
        self.stat_line = r.bool()?;
        r.bytes_into(&mut self.front)?;
        r.bytes_into(&mut self.frame)?;
        self.frame_ready = r.bool()?;
        if self.dot >= DOTS_PER_LINE || self.ly >= 154 {
            return Err("invalid screen state".into());
        }
        Ok(())
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: u32 = 154 * DOTS_PER_LINE;

    /// Écrit une ligne de tuile (2 octets) pour la tuile `tile`, ligne `row`.
    fn set_row(ppu: &mut Ppu, tile: usize, row: usize, lo: u8, hi: u8) {
        ppu.vram[tile * 16 + row * 2] = lo;
        ppu.vram[tile * 16 + row * 2 + 1] = hi;
    }

    fn run_frame(ppu: &mut Ppu) {
        ppu.tick(FRAME);
    }

    #[test]
    fn background_draws_tile_rows() {
        let mut ppu = Ppu::new();
        ppu.bgp = 0xE4; // palette identité
        set_row(&mut ppu, 0, 0, 0xFF, 0x00); // ligne 0 de la tuile 0 : couleur 1
        run_frame(&mut ppu);
        assert_eq!(ppu.frame[5], 1);
        assert_eq!(ppu.frame[160 + 5], 0);
        assert_eq!(ppu.frame[8 * 160 + 3], 1);
    }

    #[test]
    fn palette_maps_color_ids_to_shades() {
        let mut ppu = Ppu::new();
        ppu.bgp = 0b00_01_10_11; // id 0 -> 3, id 1 -> 2, id 2 -> 1, id 3 -> 0
        set_row(&mut ppu, 0, 0, 0xFF, 0xFF); // couleur 3
        run_frame(&mut ppu);
        assert_eq!(ppu.frame[0], 0);
        assert_eq!(ppu.frame[160], 3); // couleur 0 -> nuance 3
    }

    #[test]
    fn scroll_x_shifts_the_background() {
        let mut ppu = Ppu::new();
        ppu.bgp = 0xE4;
        set_row(&mut ppu, 0, 0, 0b1000_0000, 0); // seul le pixel le plus à gauche
        run_frame(&mut ppu);
        assert_eq!((ppu.frame[0], ppu.frame[1]), (1, 0));
        ppu.scx = 1;
        run_frame(&mut ppu);
        assert_eq!(ppu.frame[0], 0);
        assert_eq!(ppu.frame[7], 1); // x = 7 voit maintenant la colonne 8 (tuile suivante)
    }

    #[test]
    fn signed_tile_addressing_uses_0x9000() {
        let mut ppu = Ppu::new();
        ppu.bgp = 0xE4;
        ppu.lcdc = 0x81; // LCD + fond, données de tuiles en mode signé
        ppu.vram[0x1800] = 0xFF; // tuile -1 => 0x9000 - 16 = 0x8FF0
        ppu.vram[0x0FF0] = 0xFF; // ligne 0, bit-plane bas
        run_frame(&mut ppu);
        assert_eq!(ppu.frame[0], 1);
    }

    #[test]
    fn sprite_is_drawn_with_transparency_and_flip() {
        let mut ppu = Ppu::new();
        ppu.bgp = 0xE4;
        ppu.obp0 = 0xE4;
        ppu.lcdc = 0x93; // LCD + fond + sprites
        set_row(&mut ppu, 1, 0, 0b1000_0000, 0); // un seul pixel, à gauche
        ppu.oam[..4].copy_from_slice(&[16, 8, 1, 0]); // en (0, 0)
        run_frame(&mut ppu);
        assert_eq!((ppu.frame[0], ppu.frame[1]), (1, 0)); // transparent ailleurs
        ppu.oam[3] = 0x20; // retourné horizontalement
        run_frame(&mut ppu);
        assert_eq!((ppu.frame[0], ppu.frame[7]), (0, 1));
    }

    #[test]
    fn sprite_behind_background_when_flagged() {
        let mut ppu = Ppu::new();
        ppu.bgp = 0xE4;
        ppu.obp0 = 0xE4;
        ppu.lcdc = 0x93;
        set_row(&mut ppu, 0, 0, 0xFF, 0x00); // fond : couleur 1 sur la ligne 0
        set_row(&mut ppu, 1, 0, 0xFF, 0xFF); // sprite : couleur 3
        ppu.oam[..4].copy_from_slice(&[16, 8, 1, 0x80]); // derrière le fond
        run_frame(&mut ppu);
        assert_eq!(ppu.frame[0], 1); // le fond (couleur 1, non nulle) cache le sprite
        ppu.oam[3] = 0;
        run_frame(&mut ppu);
        assert_eq!(ppu.frame[0], 3);
    }

    #[test]
    fn leftmost_sprite_wins_overlap() {
        let mut ppu = Ppu::new();
        ppu.obp0 = 0xE4;
        ppu.lcdc = 0x93;
        set_row(&mut ppu, 1, 0, 0xFF, 0x00); // couleur 1
        set_row(&mut ppu, 2, 0, 0xFF, 0xFF); // couleur 3
        ppu.oam[..4].copy_from_slice(&[16, 12, 1, 0]); // sprite 0 : x = 4
        ppu.oam[4..8].copy_from_slice(&[16, 10, 2, 0]); // sprite 1 : x = 2, plus à gauche => prioritaire
        run_frame(&mut ppu);
        assert_eq!(ppu.frame[5], 3);
    }

    #[test]
    fn window_replaces_background_from_wx() {
        let mut ppu = Ppu::new();
        ppu.bgp = 0xE4;
        ppu.lcdc = 0xF1; // LCD, fond, fenêtre, carte de la fenêtre en 0x9C00
        ppu.wx = 7 + 80;
        ppu.wy = 0;
        ppu.vram[0x1C00..0x1C20].fill(1); // la fenêtre utilise la tuile 1
        set_row(&mut ppu, 1, 0, 0xFF, 0xFF);
        run_frame(&mut ppu);
        assert_eq!((ppu.frame[79], ppu.frame[80]), (0, 3));
    }

    #[test]
    fn modes_follow_the_line_timing() {
        let mut ppu = Ppu::new();
        assert_eq!(ppu.mode(), 2);
        ppu.tick(79);
        assert_eq!(ppu.mode(), 2);
        ppu.tick(1);
        assert_eq!(ppu.mode(), 3);
        ppu.tick(MODE3_END - 80);
        assert_eq!(ppu.mode(), 0);
        ppu.tick(DOTS_PER_LINE - MODE3_END);
        assert_eq!((ppu.ly, ppu.mode()), (1, 2));
    }

    #[test]
    fn vblank_interrupt_and_frame_ready() {
        let mut ppu = Ppu::new();
        let irq = ppu.tick(144 * DOTS_PER_LINE);
        assert_eq!(irq & IRQ_VBLANK, IRQ_VBLANK);
        assert_eq!((ppu.ly, ppu.mode()), (144, 1));
        assert!(ppu.take_frame_ready());
        assert!(!ppu.take_frame_ready()); // une seule fois
        ppu.tick(10 * DOTS_PER_LINE);
        assert_eq!(ppu.ly, 0); // retour en haut de l'écran
    }

    #[test]
    fn lyc_match_raises_stat_interrupt() {
        let mut ppu = Ppu::new();
        ppu.write_reg(0xFF41, 0x40);
        ppu.write_reg(0xFF45, 5);
        assert_eq!(ppu.tick(4 * DOTS_PER_LINE) & IRQ_STAT, 0);
        assert_eq!(ppu.tick(DOTS_PER_LINE) & IRQ_STAT, IRQ_STAT);
        assert_eq!(ppu.read_reg(0xFF41) & 0x04, 0x04);
    }

    #[test]
    fn turning_the_lcd_off_blanks_the_screen() {
        let mut ppu = Ppu::new();
        ppu.bgp = 0xFF;
        set_row(&mut ppu, 0, 0, 0xFF, 0xFF);
        run_frame(&mut ppu);
        assert_eq!(ppu.frame[0], 3);
        ppu.write_reg(0xFF40, 0x00);
        assert_eq!((ppu.ly, ppu.frame[0]), (0, 0));
        assert_eq!(ppu.tick(FRAME), 0);
        assert_eq!(ppu.ly, 0);
    }
}
