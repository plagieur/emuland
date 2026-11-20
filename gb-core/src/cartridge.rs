//! La cartouche : en-tête de la ROM et circuit de changement de banque (« mapper »).
//!
//! La Game Boy ne voit que 32 Kio de ROM à la fois. Les gros jeux contiennent plus : un circuit dans
//! la cartouche (le mapper) change la « banque » visible quand le jeu écrit à certaines adresses.

const CPU_HZ: u32 = 4_194_304;

/// Logo Nintendo attendu à l'adresse 0x104 de toute ROM officielle.
pub const LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88,
    0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
    0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

/// Taille du bloc d'horloge ajouté à la fin d'un fichier de sauvegarde (format commun à BGB et VBA) :
/// 5 registres en cours, 5 registres figés (4 octets chacun) et l'heure réelle de l'enregistrement (8 octets).
const RTC_FOOTER: usize = 48;
/// Ancien format : l'heure sur 4 octets seulement.
const RTC_FOOTER_SHORT: usize = 44;

/// Ce que l'en-tête de la ROM (0x100–0x14F) nous apprend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartridgeInfo {
    pub title: String,
    pub cart_type: u8,
    pub mapper_name: &'static str,
    pub rom_kib: usize,
    pub ram_kib: usize,
    pub cgb_only: bool,
    /// Vrai si le jeu a des graphismes en couleurs (Game Boy Color) : seul ou en plus de la version d'origine.
    pub cgb: bool,
    pub logo_ok: bool,
    pub checksum_ok: bool,
    /// Vrai si la RAM de la cartouche est alimentée par une pile : c'est elle qui garde les sauvegardes.
    pub has_battery: bool,
    /// Vrai si la cartouche contient une horloge temps réel (certaines MBC3).
    pub has_rtc: bool,
    /// Vrai si emuland sait gérer ce type de cartouche.
    pub supported: bool,
}

impl CartridgeInfo {
    pub fn parse(rom: &[u8]) -> Result<Self, String> {
        if rom.len() < 0x150 {
            return Err("this file is too small to be a Game Boy ROM".into());
        }
        let title: String = rom[0x134..0x144]
            .iter()
            .take_while(|&&b| b != 0)
            .filter(|&&b| (0x20..0x7F).contains(&b))
            .map(|&b| b as char)
            .collect();
        let cart_type = rom[0x147];
        let multicart = Self::is_mbc1_multicart(rom, cart_type);
        let mapper_name = match cart_type {
            0x00 | 0x08 | 0x09 => "ROM seule",
            0x01..=0x03 if multicart => "MBC1 multi-jeux",
            0x01..=0x03 => "MBC1",
            0x05 | 0x06 => "MBC2",
            0x0B..=0x0D => "MMM01",
            0x0F..=0x13 => "MBC3",
            0x19..=0x1E => "MBC5",
            0x20 => "MBC6",
            0x22 => "MBC7",
            0xFC => "Pocket Camera",
            0xFD => "TAMA5",
            0xFE => "HuC3",
            0xFF => "HuC1",
            _ => "inconnu",
        };
        let rom_kib = match rom[0x148] {
            n @ 0..=8 => 32usize << n,
            _ => rom.len() / 1024,
        };
        let ram_kib = match rom[0x149] {
            2 => 8,
            3 => 32,
            4 => 128,
            5 => 64,
            _ => 0,
        };
        let mut sum = 0u8;
        for &b in &rom[0x134..=0x14C] {
            sum = sum.wrapping_sub(b).wrapping_sub(1);
        }
        Ok(Self {
            title: if title.trim().is_empty() { "Untitled".into() } else { title.trim().to_string() },
            cart_type,
            mapper_name,
            rom_kib,
            ram_kib,
            cgb_only: rom[0x143] == 0xC0,
            cgb: rom[0x143] & 0x80 != 0,
            logo_ok: rom[0x104..0x134] == LOGO,
            checksum_ok: sum == rom[0x14D],
            has_battery: matches!(cart_type, 0x03 | 0x06 | 0x09 | 0x0D | 0x0F | 0x10 | 0x13 | 0x1B | 0x1E | 0x22 | 0xFC | 0xFE | 0xFF),
            has_rtc: matches!(cart_type, 0x0F | 0x10),
            // MBC6 (mémoire flash), MMM01 et TAMA5 (horloge de Tamagotchi) ne sont pas gérées
            supported: matches!(cart_type, 0x00 | 0x01..=0x03 | 0x05 | 0x06 | 0x0F..=0x13 | 0x19..=0x1E | 0x22 | 0xFC | 0xFE | 0xFF),
        })
    }

    /// Les compilations « multi-jeux » en MBC1 (1 Mio) câblent le numéro de banque autrement : on les reconnaît
    /// au logo Nintendo présent aussi au début de la deuxième moitié de la ROM (le menu est dans la première, les jeux après).
    fn is_mbc1_multicart(rom: &[u8], cart_type: u8) -> bool {
        matches!(cart_type, 0x01..=0x03) && rom.len() == 0x10_0000 && rom[0x40104..0x40134] == LOGO
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mapper {
    RomOnly,
    Mbc1,
    Mbc2,
    Mbc3,
    Mbc5,
    Mbc7,
    Camera,
    Huc1,
    Huc3,
}

/// L'EEPROM de la MBC7 (une 93LC56 de 256 octets, lue et écrite bit par bit sur 3 fils : CS, CLK et DI/DO).
#[derive(Default, Clone, Copy)]
struct Eeprom {
    cs: bool,
    clk: bool,
    di: bool,
    /// Le bit que la puce présente sur la sortie DO.
    dout: bool,
    write_enabled: bool,
    /// 0 = attend le bit de départ, 1 = reçoit la commande, 2 = envoie des données, 3 = reçoit des données à écrire.
    phase: u8,
    shift: u32,
    count: u32,
    /// Commande reçue (2 bits d'opération + 8 bits d'adresse) et mot à écrire ou envoyer.
    command: u16,
    word: u16,
}

/// L'état propre à la MBC7 : accéléromètre et EEPROM.
#[derive(Default, Clone, Copy)]
struct Mbc7State {
    enable1: bool,
    enable2: bool,
    erase_armed: bool,
    /// Inclinaison de la console (fournie par l'application) et valeurs figées lues par le jeu.
    tilt_x: u16,
    tilt_y: u16,
    latched_x: u16,
    latched_y: u16,
    eeprom: Eeprom,
}

/// Valeur de l'accéléromètre à plat, et variation pour une inclinaison maximale.
const TILT_CENTER: i32 = 0x81D0;
const TILT_RANGE: i32 = 0x70 * 4;

/// L'horloge temps réel de la MBC3 : secondes, minutes, heures, jours (9 bits), plus un bit d'arrêt et un bit de débordement.
#[derive(Default, Clone, Copy)]
struct Rtc {
    /// secondes, minutes, heures, jours (bas), jours (bit 0 = jour 8, bit 6 = arrêt, bit 7 = débordement)
    regs: [u8; 5],
    latched: [u8; 5],
    /// T-cycles accumulés vers la prochaine seconde.
    cycles: u32,
}

impl Rtc {
    fn halted(&self) -> bool {
        self.regs[4] & 0x40 != 0
    }

    /// Fait avancer l'heure de `n` secondes (sans effet si l'horloge est arrêtée).
    fn advance_seconds(&mut self, n: u64) {
        if self.halted() || n == 0 {
            return;
        }
        let day = ((self.regs[4] as u64 & 1) << 8) | self.regs[3] as u64;
        let total = self.regs[0] as u64 + 60 * (self.regs[1] as u64 + 60 * (self.regs[2] as u64 + 24 * day)) + n;
        let days = total / 86_400;
        let rest = total % 86_400;
        self.regs[0] = (rest % 60) as u8;
        self.regs[1] = (rest / 60 % 60) as u8;
        self.regs[2] = (rest / 3600) as u8;
        if days > 511 {
            self.regs[4] |= 0x80; // le compteur de jours a débordé (il reste affiché à ce moment-là)
        }
        let days = days % 512;
        self.regs[3] = days as u8;
        self.regs[4] = (self.regs[4] & 0xC0) | ((days >> 8) as u8 & 1);
    }

    fn tick(&mut self, cycles: u32) {
        if self.halted() {
            return;
        }
        self.cycles += cycles;
        if self.cycles >= CPU_HZ {
            let secs = self.cycles / CPU_HZ;
            self.cycles %= CPU_HZ;
            self.advance_seconds(secs as u64);
        }
    }

    fn write(&mut self, index: usize, value: u8) {
        self.regs[index] = match index {
            0 | 1 => value & 0x3F,
            2 => value & 0x1F,
            3 => value,
            _ => value & 0xC1,
        };
        if index == 0 {
            self.cycles = 0; // écrire les secondes remet à zéro la fraction de seconde
        }
    }
}

pub struct Cartridge {
    rom: Vec<u8>,
    ram: Vec<u8>,
    mapper: Mapper,
    ram_enabled: bool,
    // MBC1
    bank_low: usize,  // 5 bits, jamais 0
    bank_high: usize, // 2 bits
    mode: bool,
    // MBC2 / MBC3 / MBC5
    rom_bank: usize,
    ram_bank: usize,
    // horloge temps réel de la MBC3
    rtc: Rtc,
    rtc_select: Option<u8>,
    latch_prev: u8,
    /// MBC1 « multi-jeux » : le numéro de banque haut n'est décalé que de 4 bits.
    multicart: bool,
    /// HuC1 / HuC3 : registre de mode (0x0A = RAM, 0x0E = infrarouge, 0x0B à 0x0D = commandes de l'horloge de la HuC3).
    huc_mode: u8,
    mbc7: Mbc7State,
    // Pocket Camera
    camera_regs: [u8; 0x36],
    camera_busy: u32,
    camera_image: Vec<u8>,
    pub info: CartridgeInfo,
}

impl Eeprom {
    /// Les 3 fils de l'EEPROM sont pilotés par un seul octet : bit 7 = CS (sélection), bit 6 = CLK (horloge), bit 1 = DI (entrée).
    /// Les commandes arrivent bit par bit, à chaque front montant de l'horloge. `ram` contient les 128 mots de 16 bits.
    fn write_lines(&mut self, v: u8, ram: &mut [u8]) {
        let (cs, clk, di) = (v & 0x80 != 0, v & 0x40 != 0, v & 0x02 != 0);
        let (old_cs, old_clk) = (self.cs, self.clk);
        self.cs = cs;
        self.clk = clk;
        self.di = di;
        if !cs {
            self.phase = 0;
            self.count = 0;
            self.shift = 0;
            self.dout = true; // au repos, la sortie indique « prêt »
            return;
        }
        if !old_cs {
            self.phase = 0;
            self.count = 0;
            self.shift = 0;
        }
        if old_clk || !clk {
            return; // seul un front montant de l'horloge compte
        }
        match self.phase {
            0 => {
                if di {
                    self.phase = 1; // le bit de départ
                    self.count = 0;
                    self.shift = 0;
                }
            }
            1 => {
                self.shift = (self.shift << 1) | di as u32;
                self.count += 1;
                if self.count == 10 {
                    self.run_command((self.shift & 0x3FF) as u16, ram);
                }
            }
            2 => {
                // lecture : un bit de donnée par impulsion, le bit de poids fort d'abord
                self.dout = self.word & 0x8000 != 0;
                self.word <<= 1;
                self.count += 1;
                if self.count == 16 {
                    // lecture continue : le mot suivant
                    let next = (((self.command & 0x7F) + 1) & 0x7F) as usize;
                    self.command = (self.command & !0x7F) | next as u16;
                    self.word = Self::read_word(ram, next);
                    self.count = 0;
                }
            }
            _ => {
                // écriture : on reçoit les 16 bits du mot
                self.shift = (self.shift << 1) | di as u32;
                self.count += 1;
                if self.count == 16 {
                    let word = self.shift as u16;
                    if self.write_enabled {
                        if (self.command >> 8) & 3 == 0 {
                            for i in 0..128 {
                                Self::write_word(ram, i, word); // écriture de tout le contenu
                            }
                        } else {
                            Self::write_word(ram, (self.command & 0x7F) as usize, word);
                        }
                    }
                    self.phase = 0;
                    self.dout = true; // l'écriture est terminée
                }
            }
        }
    }

    fn read_word(ram: &[u8], index: usize) -> u16 {
        u16::from_le_bytes([ram[index * 2], ram[index * 2 + 1]])
    }

    fn write_word(ram: &mut [u8], index: usize, word: u16) {
        ram[index * 2..index * 2 + 2].copy_from_slice(&word.to_le_bytes());
    }

    /// Une commande complète : 2 bits d'opération puis 8 bits d'adresse.
    fn run_command(&mut self, command: u16, ram: &mut [u8]) {
        let index = (command & 0x7F) as usize;
        self.command = command;
        match (command >> 8) & 3 {
            2 => {
                // READ : un bit à 0, puis le mot
                self.word = Self::read_word(ram, index);
                self.phase = 2;
                self.count = 0;
                self.dout = false;
            }
            1 => {
                // WRITE : suivi de 16 bits à écrire
                self.phase = 3;
                self.count = 0;
                self.shift = 0;
            }
            3 => {
                // ERASE : le mot repasse à 0xFFFF
                if self.write_enabled {
                    Self::write_word(ram, index, 0xFFFF);
                }
                self.phase = 0;
                self.dout = true;
            }
            _ => match (command >> 6) & 3 {
                3 => {
                    self.write_enabled = true; // EWEN
                    self.phase = 0;
                    self.dout = true;
                }
                0 => {
                    self.write_enabled = false; // EWDS
                    self.phase = 0;
                    self.dout = true;
                }
                2 => {
                    // ERAL : tout effacer
                    if self.write_enabled {
                        ram.fill(0xFF);
                    }
                    self.phase = 0;
                    self.dout = true;
                }
                _ => {
                    // WRAL : un mot écrit partout
                    self.phase = 3;
                    self.count = 0;
                    self.shift = 0;
                }
            },
        }
    }
}

/// Une capture de la Pocket Camera dure environ 32 ms.
const CAMERA_CAPTURE_CYCLES: u32 = 136_000;
const CAMERA_W: usize = 128;
const CAMERA_H: usize = 112;

impl Cartridge {
    pub fn new(rom: Vec<u8>) -> Result<Self, String> {
        let info = CartridgeInfo::parse(&rom)?;
        let mapper = match info.cart_type {
            0x01..=0x03 => Mapper::Mbc1,
            0x05 | 0x06 => Mapper::Mbc2,
            0x0F..=0x13 => Mapper::Mbc3,
            0x19..=0x1E => Mapper::Mbc5,
            0x22 => Mapper::Mbc7,
            0xFC => Mapper::Camera,
            0xFE => Mapper::Huc3,
            0xFF => Mapper::Huc1,
            _ => Mapper::RomOnly,
        };
        let multicart = CartridgeInfo::is_mbc1_multicart(&rom, info.cart_type);
        // la MBC2 a 512 demi-octets de RAM dans la puce elle-même ; l'en-tête n'indique donc aucune RAM
        // Certaines ROMs (dont des ROMs de test) déclarent un type « avec RAM » sans indiquer sa taille : on lui donne 8 Kio.
        let declares_ram = matches!(info.cart_type, 0x02 | 0x03 | 0x08 | 0x09 | 0x10 | 0x12 | 0x13 | 0x1A | 0x1B | 0x1D | 0x1E);
        let ram_kib = if info.ram_kib == 0 && declares_ram { 8 } else { info.ram_kib };
        let ram = match mapper {
            Mapper::Mbc2 => vec![0; 512],
            Mapper::Mbc7 => vec![0xFF; 256], // l'EEPROM de la MBC7 : 128 mots de 16 bits, vides = 0xFFFF
            Mapper::Camera if ram_kib == 0 => vec![0; 128 * 1024],
            _ => vec![0; ram_kib * 1024],
        };
        // une image de départ pour la Pocket Camera (un dégradé) : l'application peut en fournir une vraie
        let camera_image = (0..CAMERA_W * CAMERA_H).map(|i| ((i % CAMERA_W) * 2) as u8).collect();
        Ok(Self {
            rom,
            ram,
            mapper,
            ram_enabled: matches!(mapper, Mapper::Huc1),
            multicart,
            huc_mode: 0,
            mbc7: Mbc7State::default(),
            camera_regs: [0; 0x36],
            camera_busy: 0,
            camera_image,
            bank_low: 1,
            bank_high: 0,
            mode: false,
            rom_bank: 1,
            ram_bank: 0,
            rtc: Rtc::default(),
            rtc_select: None,
            latch_prev: 0xFF,
            info,
        })
    }

    fn banks(&self) -> usize {
        (self.rom.len() / 0x4000).max(1)
    }

    pub fn read_rom(&self, addr: u16) -> u8 {
        let bank = match self.mapper {
            Mapper::RomOnly => 0,
            Mapper::Mbc1 => {
                let shift = if self.multicart { 4 } else { 5 };
                let low = if self.multicart { self.bank_low & 0x0F } else { self.bank_low };
                if addr < 0x4000 {
                    if self.mode { self.bank_high << shift } else { 0 }
                } else {
                    (self.bank_high << shift) | low
                }
            }
            Mapper::Mbc2 | Mapper::Mbc3 | Mapper::Mbc5 | Mapper::Mbc7 | Mapper::Camera | Mapper::Huc1 | Mapper::Huc3 => {
                if addr < 0x4000 { 0 } else { self.rom_bank }
            }
        };
        let index = if self.mapper == Mapper::RomOnly { addr as usize } else { (bank % self.banks()) * 0x4000 + (addr as usize & 0x3FFF) };
        self.rom.get(index).copied().unwrap_or(0xFF)
    }

    /// Écrire dans la zone ROM ne modifie pas la ROM : ce sont des ordres au mapper.
    pub fn write_rom(&mut self, addr: u16, value: u8) {
        match self.mapper {
            Mapper::RomOnly => {}
            Mapper::Mbc1 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = value & 0x0F == 0x0A,
                0x2000..=0x3FFF => self.bank_low = ((value & 0x1F) as usize).max(1),
                0x4000..=0x5FFF => self.bank_high = (value & 0x03) as usize,
                _ => self.mode = value & 1 == 1,
            },
            Mapper::Mbc2 => {
                // un seul registre pour deux rôles : le bit 8 de l'adresse choisit lequel
                if addr < 0x4000 {
                    if addr & 0x0100 == 0 {
                        self.ram_enabled = value & 0x0F == 0x0A;
                    } else {
                        self.rom_bank = ((value & 0x0F) as usize).max(1);
                    }
                }
            }
            Mapper::Mbc3 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = value & 0x0F == 0x0A,
                0x2000..=0x3FFF => self.rom_bank = ((value & 0x7F) as usize).max(1),
                0x4000..=0x5FFF => match value {
                    0..=3 => {
                        self.ram_bank = value as usize;
                        self.rtc_select = None;
                    }
                    0x08..=0x0C => self.rtc_select = Some(value),
                    _ => {}
                },
                _ => {
                    // écrire 0 puis 1 fige l'heure courante dans les registres lisibles
                    if self.latch_prev == 0 && value == 1 {
                        self.rtc.latched = self.rtc.regs;
                    }
                    self.latch_prev = value;
                }
            },
            Mapper::Mbc5 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = value & 0x0F == 0x0A,
                0x2000..=0x2FFF => self.rom_bank = (self.rom_bank & 0x100) | value as usize,
                0x3000..=0x3FFF => self.rom_bank = (self.rom_bank & 0xFF) | ((value as usize & 1) << 8),
                0x4000..=0x5FFF => self.ram_bank = (value & 0x0F) as usize,
                _ => {}
            },
            Mapper::Mbc7 => match addr {
                // la RAM (accéléromètre et EEPROM) demande deux autorisations
                0x0000..=0x1FFF => self.mbc7.enable1 = value == 0x0A,
                0x2000..=0x3FFF => self.rom_bank = ((value & 0x7F) as usize).max(1),
                0x4000..=0x5FFF => self.mbc7.enable2 = value == 0x40,
                _ => {}
            },
            Mapper::Camera => match addr {
                0x0000..=0x1FFF => self.ram_enabled = value & 0x0F == 0x0A,
                0x2000..=0x3FFF => self.rom_bank = ((value & 0x3F) as usize).max(1),
                0x4000..=0x5FFF => self.ram_bank = (value & 0x1F) as usize, // 0x10 et plus : registres de l'appareil photo
                _ => {}
            },
            Mapper::Huc1 => match addr {
                0x0000..=0x1FFF => {
                    self.huc_mode = value & 0x0F;
                    self.ram_enabled = self.huc_mode != 0x0E; // 0x0E : récepteur infrarouge à la place de la RAM
                }
                0x2000..=0x3FFF => self.rom_bank = ((value & 0x3F) as usize).max(1),
                0x4000..=0x5FFF => self.ram_bank = (value & 0x03) as usize,
                _ => {}
            },
            Mapper::Huc3 => match addr {
                0x0000..=0x1FFF => {
                    self.huc_mode = value & 0x0F;
                    self.ram_enabled = matches!(self.huc_mode, 0x00 | 0x0A);
                }
                0x2000..=0x3FFF => self.rom_bank = ((value & 0x7F) as usize).max(1),
                0x4000..=0x5FFF => self.ram_bank = (value & 0x03) as usize,
                _ => {}
            },
        }
    }

    fn ram_index(&self, addr: u16) -> Option<usize> {
        if self.ram.is_empty() || self.mapper == Mapper::Mbc7 || (self.mapper != Mapper::RomOnly && !self.ram_enabled) {
            return None;
        }
        let offset = addr as usize - 0xA000;
        let bank = match self.mapper {
            Mapper::RomOnly | Mapper::Mbc2 | Mapper::Mbc7 => 0,
            Mapper::Mbc1 => if self.mode { self.bank_high } else { 0 },
            Mapper::Mbc3 | Mapper::Mbc5 | Mapper::Camera | Mapper::Huc1 | Mapper::Huc3 => self.ram_bank,
        };
        if self.mapper == Mapper::Mbc2 {
            return Some(offset & 0x1FF); // les 512 demi-octets se répètent sur toute la zone
        }
        Some((bank * 0x2000 + offset) % self.ram.len())
    }

    pub fn read_ram(&self, addr: u16) -> u8 {
        match self.mapper {
            Mapper::Mbc7 => return self.mbc7_read(addr),
            Mapper::Camera if self.ram_enabled && self.ram_bank >= 0x10 => return self.camera_read(addr),
            Mapper::Huc1 if self.huc_mode == 0x0E => return 0xC0, // infrarouge : aucun signal reçu
            Mapper::Huc3 => match self.huc_mode {
                0x0C | 0x0D => return 0x01, // réponse de l'horloge et sémaphore « prêt »
                0x0E => return 0xC0,
                0x0B => return 0xFF,
                _ => {}
            },
            _ => {}
        }
        if self.mapper == Mapper::Mbc3 && self.ram_enabled {
            if let Some(reg) = self.rtc_select {
                return self.rtc.latched[(reg - 0x08) as usize];
            }
        }
        match self.ram_index(addr) {
            Some(i) if self.mapper == Mapper::Mbc2 => 0xF0 | (self.ram[i] & 0x0F), // seuls les 4 bits du bas existent
            Some(i) => self.ram[i],
            None => 0xFF,
        }
    }

    pub fn write_ram(&mut self, addr: u16, value: u8) {
        match self.mapper {
            Mapper::Mbc7 => return self.mbc7_write(addr, value),
            Mapper::Camera if self.ram_enabled && self.ram_bank >= 0x10 => return self.camera_write(addr, value),
            Mapper::Huc1 if self.huc_mode == 0x0E => return,
            Mapper::Huc3 if matches!(self.huc_mode, 0x00 | 0x0B | 0x0C | 0x0D | 0x0E) => return, // mode 0 : lecture seule
            _ => {}
        }
        if self.mapper == Mapper::Mbc3 && self.ram_enabled {
            if let Some(reg) = self.rtc_select {
                self.rtc.write((reg - 0x08) as usize, value);
                return;
            }
        }
        if let Some(i) = self.ram_index(addr) {
            self.ram[i] = if self.mapper == Mapper::Mbc2 { value & 0x0F } else { value };
        }
    }

    /// Fait avancer l'horloge temps réel de la cartouche (si elle en a une) et la capture de la Pocket Camera.
    pub fn tick(&mut self, cycles: u32) {
        if self.info.has_rtc {
            self.rtc.tick(cycles);
        }
        if self.camera_busy > 0 {
            self.camera_busy = self.camera_busy.saturating_sub(cycles);
            if self.camera_busy == 0 {
                self.camera_finish_capture();
            }
        }
    }

    // ---------- MBC7 : accéléromètre et EEPROM ----------

    /// Inclinaison de la console, de -1 (gauche / haut) à +1 (droite / bas), pour les jeux qui s'en servent (MBC7).
    pub fn set_tilt(&mut self, x: f32, y: f32) {
        let conv = |v: f32| (TILT_CENTER + (v.clamp(-1.0, 1.0) * TILT_RANGE as f32) as i32) as u16;
        self.mbc7.tilt_x = conv(-x); // l'accéléromètre lit l'inverse : incliner à droite baisse la valeur
        self.mbc7.tilt_y = conv(-y);
    }

    /// Vrai si la cartouche a un accéléromètre.
    pub fn has_tilt(&self) -> bool {
        self.mapper == Mapper::Mbc7
    }

    fn mbc7_read(&self, addr: u16) -> u8 {
        if !(self.mbc7.enable1 && self.mbc7.enable2) {
            return 0xFF;
        }
        let m = &self.mbc7;
        match addr & 0xF0 {
            0x20 => m.latched_x as u8,
            0x30 => (m.latched_x >> 8) as u8,
            0x40 => m.latched_y as u8,
            0x50 => (m.latched_y >> 8) as u8,
            0x60 => 0,
            0x80 => (m.eeprom.cs as u8) << 7 | (m.eeprom.clk as u8) << 6 | (m.eeprom.di as u8) << 1 | m.eeprom.dout as u8,
            _ => 0xFF,
        }
    }

    fn mbc7_write(&mut self, addr: u16, value: u8) {
        if !(self.mbc7.enable1 && self.mbc7.enable2) {
            return;
        }
        match addr & 0xF0 {
            0x00 => {
                // 0x55 efface les valeurs figées, 0xAA (ensuite) fige l'inclinaison actuelle
                if value == 0x55 {
                    self.mbc7.erase_armed = true;
                    self.mbc7.latched_x = 0x8000;
                    self.mbc7.latched_y = 0x8000;
                }
            }
            0x10 => {
                if value == 0xAA && self.mbc7.erase_armed {
                    self.mbc7.erase_armed = false;
                    self.mbc7.latched_x = self.mbc7.tilt_x;
                    self.mbc7.latched_y = self.mbc7.tilt_y;
                }
            }
            0x80 => self.mbc7.eeprom.write_lines(value, &mut self.ram),
            _ => {}
        }
    }

    // ---------- Pocket Camera ----------

    /// Fournit l'image que l'appareil verra à la prochaine capture : 128 × 112 pixels en gris (0 = noir, 255 = blanc).
    pub fn set_camera_image(&mut self, gray: &[u8]) {
        if gray.len() == CAMERA_W * CAMERA_H {
            self.camera_image.copy_from_slice(gray);
        }
    }

    /// Vrai si la cartouche est une Pocket Camera.
    pub fn is_camera(&self) -> bool {
        self.mapper == Mapper::Camera
    }

    fn camera_read(&self, addr: u16) -> u8 {
        let reg = (addr as usize - 0xA000) & 0x7F;
        if reg == 0 {
            (self.camera_regs[0] & 0xFE) | (self.camera_busy > 0) as u8 // bit 0 : capture en cours
        } else {
            0 // les autres registres s'écrivent seulement
        }
    }

    fn camera_write(&mut self, addr: u16, value: u8) {
        let reg = (addr as usize - 0xA000) & 0x7F;
        if reg == 0 {
            self.camera_regs[0] = value & 0x07;
            if value & 1 != 0 && self.camera_busy == 0 {
                self.camera_busy = CAMERA_CAPTURE_CYCLES;
            }
        } else if reg < self.camera_regs.len() {
            self.camera_regs[reg] = value;
        }
    }

    /// Écrit l'image capturée dans la RAM (banque 0, à partir de 0x100) au format des tuiles de la Game Boy : 4 nuances.
    fn camera_finish_capture(&mut self) {
        self.camera_regs[0] &= !1;
        if self.ram.len() < 0x1000 {
            return;
        }
        for ty in 0..CAMERA_H / 8 {
            for tx in 0..CAMERA_W / 8 {
                let tile = 0x100 + (ty * (CAMERA_W / 8) + tx) * 16;
                for row in 0..8 {
                    let (mut lo, mut hi) = (0u8, 0u8);
                    for col in 0..8 {
                        let gray = self.camera_image[(ty * 8 + row) * CAMERA_W + tx * 8 + col];
                        let shade = 3 - (gray >> 6); // plus clair = nuance plus petite
                        lo |= (shade & 1) << (7 - col);
                        hi |= (shade >> 1) << (7 - col);
                    }
                    self.ram[tile + row * 2] = lo;
                    self.ram[tile + row * 2 + 1] = hi;
                }
            }
        }
    }

    /// Le contenu de la RAM de la cartouche (sans l'horloge).
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// Recharge seulement la RAM. Un fichier de taille différente est ignoré (sauvegarde d'un autre jeu).
    pub fn load_ram(&mut self, data: &[u8]) -> bool {
        if data.len() == self.ram.len() && !data.is_empty() {
            self.ram.copy_from_slice(data);
            true
        } else {
            false
        }
    }

    /// Le fichier de sauvegarde complet : la RAM, suivie (pour les cartouches à horloge) de l'état de
    /// l'horloge et de l'heure réelle `now_unix` (en secondes) à laquelle on enregistre.
    pub fn save_bytes(&self, now_unix: u64) -> Vec<u8> {
        let mut out = self.ram.clone();
        if self.info.has_rtc {
            for regs in [&self.rtc.regs, &self.rtc.latched] {
                for &r in regs.iter() {
                    out.extend_from_slice(&(r as u32).to_le_bytes());
                }
            }
            out.extend_from_slice(&now_unix.to_le_bytes());
        }
        out
    }

    /// Recharge une sauvegarde. Pour une cartouche à horloge, l'heure avance du temps réel écoulé
    /// entre l'enregistrement et `now_unix` : l'horloge continue de tourner console éteinte, comme grâce à la pile.
    pub fn load_save(&mut self, data: &[u8], now_unix: u64) -> bool {
        let ram_len = self.ram.len();
        if self.info.has_rtc && (data.len() == ram_len + RTC_FOOTER || data.len() == ram_len + RTC_FOOTER_SHORT) {
            let (ram, footer) = data.split_at(ram_len);
            self.ram.copy_from_slice(ram);
            let word = |i: usize| u32::from_le_bytes(footer[i * 4..i * 4 + 4].try_into().unwrap());
            for i in 0..5 {
                self.rtc.regs[i] = word(i) as u8;
                self.rtc.latched[i] = word(5 + i) as u8;
            }
            let saved_at = if footer.len() == RTC_FOOTER {
                u64::from_le_bytes(footer[40..48].try_into().unwrap())
            } else {
                word(10) as u64
            };
            self.rtc.cycles = 0;
            self.rtc.advance_seconds(now_unix.saturating_sub(saved_at));
            return true;
        }
        self.load_ram(data)
    }
}

impl Cartridge {
    /// La ROM elle-même n'est pas dans l'état : seulement ce qui change pendant le jeu.
    pub(crate) fn save_state(&self, w: &mut crate::state::StateWriter) {
        w.bytes(&self.ram);
        w.bool(self.ram_enabled);
        w.u32(self.bank_low as u32);
        w.u32(self.bank_high as u32);
        w.bool(self.mode);
        w.u32(self.rom_bank as u32);
        w.u32(self.ram_bank as u32);
        for regs in [&self.rtc.regs, &self.rtc.latched] {
            for &r in regs.iter() {
                w.u8(r);
            }
        }
        w.u32(self.rtc.cycles);
        w.bool(self.rtc_select.is_some());
        w.u8(self.rtc_select.unwrap_or(0));
        w.u8(self.latch_prev);
        w.u8(self.huc_mode);
        let m = &self.mbc7;
        for b in [m.enable1, m.enable2, m.erase_armed, m.eeprom.cs, m.eeprom.clk, m.eeprom.di, m.eeprom.dout, m.eeprom.write_enabled] {
            w.bool(b);
        }
        for v in [m.tilt_x, m.tilt_y, m.latched_x, m.latched_y, m.eeprom.command, m.eeprom.word] {
            w.u16(v);
        }
        w.u8(m.eeprom.phase);
        w.u32(m.eeprom.shift);
        w.u32(m.eeprom.count);
        w.bytes(&self.camera_regs);
        w.u32(self.camera_busy);
    }

    pub(crate) fn load_state(&mut self, r: &mut crate::state::StateReader) -> Result<(), String> {
        r.bytes_into(&mut self.ram)?;
        self.ram_enabled = r.bool()?;
        self.bank_low = r.u32()? as usize;
        self.bank_high = r.u32()? as usize;
        self.mode = r.bool()?;
        self.rom_bank = r.u32()? as usize;
        self.ram_bank = r.u32()? as usize;
        for i in 0..5 {
            self.rtc.regs[i] = r.u8()?;
        }
        for i in 0..5 {
            self.rtc.latched[i] = r.u8()?;
        }
        self.rtc.cycles = r.u32()?;
        let has_select = r.bool()?;
        let select = r.u8()?;
        self.rtc_select = if has_select && (0x08..=0x0C).contains(&select) { Some(select) } else { None };
        self.latch_prev = r.u8()?;
        self.huc_mode = r.u8()? & 0x0F;
        let m = &mut self.mbc7;
        m.enable1 = r.bool()?;
        m.enable2 = r.bool()?;
        m.erase_armed = r.bool()?;
        m.eeprom.cs = r.bool()?;
        m.eeprom.clk = r.bool()?;
        m.eeprom.di = r.bool()?;
        m.eeprom.dout = r.bool()?;
        m.eeprom.write_enabled = r.bool()?;
        m.tilt_x = r.u16()?;
        m.tilt_y = r.u16()?;
        m.latched_x = r.u16()?;
        m.latched_y = r.u16()?;
        m.eeprom.command = r.u16()?;
        m.eeprom.word = r.u16()?;
        m.eeprom.phase = r.u8()?.min(3);
        m.eeprom.shift = r.u32()?;
        m.eeprom.count = r.u32()?;
        r.bytes_into(&mut self.camera_regs)?;
        self.camera_busy = r.u32()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une ROM de `banks` banques de 16 Kio dont le dernier octet de chaque banque vaut son numéro
    /// (modulo 256 pour rester sur un octet).
    fn rom_with(cart_type: u8, banks: usize, ram_code: u8) -> Vec<u8> {
        let mut rom = vec![0u8; banks * 0x4000];
        rom[0x147] = cart_type;
        rom[0x148] = (banks.trailing_zeros() as u8).saturating_sub(1);
        rom[0x149] = ram_code;
        for b in 0..banks {
            rom[b * 0x4000 + 0x3FFF] = (b & 0xFF) as u8;
        }
        rom
    }

    #[test]
    fn mbc3_info_is_recognised_and_supported() {
        let info = CartridgeInfo::parse(&rom_with(0x13, 64, 3)).unwrap();
        assert_eq!((info.mapper_name, info.supported, info.has_battery, info.ram_kib), ("MBC3", true, true, 32));
        assert!(!info.has_rtc);
        assert!(CartridgeInfo::parse(&rom_with(0x10, 4, 3)).unwrap().has_rtc);
    }

    #[test]
    fn mbc3_switches_up_to_128_banks() {
        let mut cart = Cartridge::new(rom_with(0x13, 64, 3)).unwrap();
        assert_eq!(cart.read_rom(0x7FFF), 1); // banque 1 au démarrage
        cart.write_rom(0x2000, 0x25);
        assert_eq!(cart.read_rom(0x7FFF), 0x25);
        cart.write_rom(0x2000, 0); // 0 devient 1
        assert_eq!(cart.read_rom(0x7FFF), 1);
        assert_eq!(cart.read_rom(0x3FFF), 0); // banque 0 fixe en bas
        cart.write_rom(0x2000, 0x7F); // 127 % 64 = 63
        assert_eq!(cart.read_rom(0x7FFF), 63);
    }

    #[test]
    fn mbc3_ram_needs_enabling_and_has_four_banks() {
        let mut cart = Cartridge::new(rom_with(0x13, 4, 3)).unwrap();
        cart.write_ram(0xA000, 0x42);
        assert_eq!(cart.read_ram(0xA000), 0xFF); // RAM désactivée : rien ne s'écrit
        cart.write_rom(0x0000, 0x0A);
        cart.write_ram(0xA000, 0x42);
        cart.write_rom(0x4000, 2);
        cart.write_ram(0xA000, 0x99);
        assert_eq!(cart.read_ram(0xA000), 0x99);
        cart.write_rom(0x4000, 0);
        assert_eq!(cart.read_ram(0xA000), 0x42);
        assert_eq!(cart.ram()[0x4000], 0x99); // banque 2 = octets 0x4000..0x6000 du fichier de sauvegarde
    }

    #[test]
    fn mbc5_has_nine_bit_rom_banks() {
        let mut cart = Cartridge::new(rom_with(0x1B, 512, 3)).unwrap();
        cart.write_rom(0x2000, 0x01);
        cart.write_rom(0x3000, 0x01); // 0x101 = 257
        assert_eq!(cart.read_rom(0x7FFF), (257 & 0xFF) as u8);
        cart.write_rom(0x3000, 0x00);
        cart.write_rom(0x2000, 0x00); // la banque 0 est permise en haut
        assert_eq!(cart.read_rom(0x7FFF), 0);
    }

    #[test]
    fn save_ram_round_trips_and_rejects_wrong_size() {
        let mut cart = Cartridge::new(rom_with(0x13, 4, 3)).unwrap();
        let mut save = vec![0u8; 32 * 1024];
        save[5] = 7;
        assert!(cart.load_ram(&save));
        assert_eq!(cart.ram()[5], 7);
        assert!(!cart.load_ram(&[1, 2, 3])); // mauvaise taille : ignorée
        assert_eq!(cart.ram()[5], 7);
    }

    // ---------- MBC2 ----------

    #[test]
    fn mbc2_is_supported_and_has_built_in_ram() {
        let cart = Cartridge::new(rom_with(0x06, 8, 0)).unwrap();
        assert_eq!((cart.info.mapper_name, cart.info.supported, cart.info.has_battery), ("MBC2", true, true));
        assert_eq!(cart.ram().len(), 512);
    }

    #[test]
    fn mbc2_address_bit_8_chooses_between_ram_enable_and_rom_bank() {
        let mut cart = Cartridge::new(rom_with(0x05, 16, 0)).unwrap();
        cart.write_rom(0x2100, 0x05); // bit 8 à 1 : banque de ROM
        assert_eq!(cart.read_rom(0x7FFF), 5);
        cart.write_rom(0x2100, 0x00); // 0 devient 1
        assert_eq!(cart.read_rom(0x7FFF), 1);
        cart.write_rom(0x2100, 0x1F); // seuls 4 bits comptent : 15
        assert_eq!(cart.read_rom(0x7FFF), 15);
        assert_eq!(cart.read_rom(0x3FFF), 0);
        // bit 8 à 0 : activation de la RAM, même à une adresse de la zone « banque » (0x2000)
        cart.write_ram(0xA000, 0x05);
        assert_eq!(cart.read_ram(0xA000), 0xFF); // désactivée
        cart.write_rom(0x2000, 0x0A);
        cart.write_ram(0xA000, 0x05);
        assert_eq!(cart.read_ram(0xA000) & 0x0F, 5);
        assert_eq!(cart.read_rom(0x7FFF), 15, "le bit 8 à 0 n'a pas changé la banque");
    }

    #[test]
    fn mbc2_ram_is_four_bits_and_repeats_every_512() {
        let mut cart = Cartridge::new(rom_with(0x05, 4, 0)).unwrap();
        cart.write_rom(0x0000, 0x0A);
        cart.write_ram(0xA000, 0xAB);
        assert_eq!(cart.read_ram(0xA000), 0xFB, "les 4 bits du haut se lisent à 1");
        assert_eq!(cart.read_ram(0xA200), 0xFB, "0xA200 est un écho de 0xA000");
        assert_eq!(cart.read_ram(0xBE00), 0xFB);
        cart.write_ram(0xA1FF, 0x03);
        assert_eq!(cart.ram()[0x1FF], 0x03);
    }

    // ---------- horloge temps réel ----------

    /// Une cartouche à horloge dont la RAM et l'horloge sont activées.
    fn rtc_cart() -> Cartridge {
        let mut cart = Cartridge::new(rom_with(0x10, 4, 3)).unwrap();
        cart.write_rom(0x0000, 0x0A);
        cart
    }

    fn read_clock(cart: &mut Cartridge, reg: u8) -> u8 {
        cart.write_rom(0x6000, 0);
        cart.write_rom(0x6000, 1); // fige l'heure
        cart.write_rom(0x4000, reg);
        cart.read_ram(0xA000)
    }

    fn set_clock(cart: &mut Cartridge, reg: u8, value: u8) {
        cart.write_rom(0x4000, reg);
        cart.write_ram(0xA000, value);
    }

    #[test]
    fn clock_counts_real_seconds_from_emulated_cycles() {
        let mut cart = rtc_cart();
        cart.tick(CPU_HZ - 1);
        assert_eq!(read_clock(&mut cart, 0x08), 0);
        cart.tick(1);
        assert_eq!(read_clock(&mut cart, 0x08), 1);
        cart.tick(CPU_HZ * 61);
        assert_eq!((read_clock(&mut cart, 0x09), read_clock(&mut cart, 0x08)), (1, 2)); // 1 min 2 s
    }

    #[test]
    fn clock_rolls_over_minutes_hours_and_days() {
        let mut cart = rtc_cart();
        for (reg, v) in [(0x08, 59), (0x09, 59), (0x0A, 23), (0x0B, 255), (0x0C, 0)] {
            set_clock(&mut cart, reg, v);
        }
        cart.tick(CPU_HZ);
        assert_eq!((read_clock(&mut cart, 0x08), read_clock(&mut cart, 0x09), read_clock(&mut cart, 0x0A)), (0, 0, 0));
        assert_eq!(read_clock(&mut cart, 0x0B), 0); // jour 255 -> 256 : le bit 8 passe à 1
        assert_eq!(read_clock(&mut cart, 0x0C), 0x01);
    }

    #[test]
    fn day_counter_overflow_sets_the_carry_flag() {
        let mut cart = rtc_cart();
        for (reg, v) in [(0x08, 59), (0x09, 59), (0x0A, 23), (0x0B, 255), (0x0C, 1)] {
            set_clock(&mut cart, reg, v); // jour 511, 23:59:59
        }
        cart.tick(CPU_HZ);
        assert_eq!(read_clock(&mut cart, 0x0C) & 0x80, 0x80, "débordement signalé");
        assert_eq!(read_clock(&mut cart, 0x0B), 0);
    }

    #[test]
    fn halted_clock_does_not_advance_and_latch_freezes_the_view() {
        let mut cart = rtc_cart();
        set_clock(&mut cart, 0x0C, 0x40); // arrêt
        cart.tick(CPU_HZ * 5);
        assert_eq!(read_clock(&mut cart, 0x08), 0);
        set_clock(&mut cart, 0x0C, 0x00); // redémarre
        cart.tick(CPU_HZ * 2);
        cart.write_rom(0x6000, 0);
        cart.write_rom(0x6000, 1);
        cart.write_rom(0x4000, 0x08);
        cart.tick(CPU_HZ * 10); // le temps avance, mais la valeur lue reste figée
        assert_eq!(cart.read_ram(0xA000), 2);
    }

    #[test]
    fn save_file_has_a_clock_footer_and_restores_elapsed_real_time() {
        let mut cart = rtc_cart();
        cart.write_rom(0x4000, 0);
        cart.write_ram(0xA000, 0x77); // un octet de sauvegarde
        for (reg, v) in [(0x08, 50), (0x09, 1)] {
            set_clock(&mut cart, reg, v);
        }
        let save = cart.save_bytes(1_000_000);
        assert_eq!(save.len(), 32 * 1024 + 48);

        // on rouvre la sauvegarde 30 secondes (réelles) plus tard : 50 s + 30 s = 1 min 20 s en plus
        let mut reopened = Cartridge::new(rom_with(0x10, 4, 3)).unwrap();
        assert!(reopened.load_save(&save, 1_000_030));
        reopened.write_rom(0x0000, 0x0A);
        assert_eq!(reopened.ram()[0], 0x77);
        assert_eq!((read_clock(&mut reopened, 0x08), read_clock(&mut reopened, 0x09)), (20, 2)); // 2 min 20 s
    }

    #[test]
    fn a_halted_clock_stays_stopped_across_a_save() {
        let mut cart = rtc_cart();
        set_clock(&mut cart, 0x08, 10);
        set_clock(&mut cart, 0x0C, 0x40);
        let save = cart.save_bytes(500);
        let mut reopened = Cartridge::new(rom_with(0x10, 4, 3)).unwrap();
        reopened.load_save(&save, 500 + 3600);
        reopened.write_rom(0x0000, 0x0A);
        assert_eq!(read_clock(&mut reopened, 0x08), 10);
    }

    #[test]
    fn plain_ram_saves_have_no_footer_and_old_footers_load() {
        let mut cart = Cartridge::new(rom_with(0x13, 4, 3)).unwrap(); // MBC3 sans horloge
        assert_eq!(cart.save_bytes(123).len(), 32 * 1024);
        // ancien format : 44 octets de pied de page
        let mut old = Cartridge::new(rom_with(0x10, 4, 3)).unwrap();
        let mut data = vec![0u8; 32 * 1024];
        data.extend(std::iter::repeat(0u8).take(44));
        assert!(old.load_save(&data, 10));
        assert!(cart.load_save(&vec![0u8; 32 * 1024], 10));
    }

    // ---------- cartouches rares ----------

    #[test]
    fn mbc1_multicart_uses_four_bit_banks() {
        let mut rom = rom_with(0x01, 64, 0); // 1 Mio
        rom[0x40104..0x40134].copy_from_slice(&LOGO); // le logo à la moitié : c'est une compilation
        let mut cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.info.mapper_name, "MBC1 multi-jeux");
        cart.write_rom(0x4000, 1); // « jeu » numéro 1 : banques 16 à 31
        cart.write_rom(0x2000, 0x03);
        assert_eq!(cart.read_rom(0x7FFF), 0x13, "banque haute 1 (16) + banque basse 3");
        cart.write_rom(0x2000, 0x1F); // seuls 4 bits comptent dans une compilation
        assert_eq!(cart.read_rom(0x7FFF), 0x1F);
        cart.write_rom(0x6000, 1); // mode 1 : la banque 0 du bas suit le jeu choisi
        assert_eq!(cart.read_rom(0x3FFF), 0x10);
        let plain = Cartridge::new(rom_with(0x01, 64, 0)).unwrap();
        assert_eq!(plain.info.mapper_name, "MBC1");
    }

    #[test]
    fn huc1_switches_banks_and_exposes_the_infrared_port() {
        let mut cart = Cartridge::new(rom_with(0xFF, 8, 3)).unwrap();
        assert_eq!((cart.info.mapper_name, cart.info.supported, cart.info.has_battery), ("HuC1", true, true));
        cart.write_rom(0x2000, 5);
        assert_eq!(cart.read_rom(0x7FFF), 5);
        cart.write_ram(0xA000, 0x42);
        assert_eq!(cart.read_ram(0xA000), 0x42, "la RAM répond par défaut");
        cart.write_rom(0x0000, 0x0E); // mode infrarouge
        assert_eq!(cart.read_ram(0xA000), 0xC0, "aucun signal infrarouge reçu");
        cart.write_rom(0x0000, 0x0A);
        assert_eq!(cart.read_ram(0xA000), 0x42);
    }

    #[test]
    fn huc3_switches_rom_banks_and_ram_modes() {
        let mut cart = Cartridge::new(rom_with(0xFE, 16, 3)).unwrap();
        assert_eq!(cart.info.mapper_name, "HuC3");
        cart.write_rom(0x2000, 9);
        assert_eq!(cart.read_rom(0x7FFF), 9);
        cart.write_rom(0x0000, 0x0A);
        cart.write_ram(0xA010, 0x77);
        assert_eq!(cart.read_ram(0xA010), 0x77);
        cart.write_rom(0x0000, 0x00); // mode 0 : lecture seule
        cart.write_ram(0xA010, 0x11);
        assert_eq!(cart.read_ram(0xA010), 0x77);
        cart.write_rom(0x0000, 0x0D);
        assert_eq!(cart.read_ram(0xA000), 0x01, "le sémaphore de l'horloge dit « prêt »");
    }

    /// Envoie une commande à l'EEPROM de la MBC7 en pilotant ses trois fils comme le fait un jeu.
    fn eeprom_clock(cart: &mut Cartridge, cs: bool, di: bool) {
        let base = (cs as u8) << 7 | (di as u8) << 1;
        cart.write_ram(0xA080, base); // horloge basse
        cart.write_ram(0xA080, base | 0x40); // front montant
    }

    fn eeprom_send(cart: &mut Cartridge, bits: &[bool]) {
        for &b in bits {
            eeprom_clock(cart, true, b);
        }
    }

    fn bits(value: u32, count: usize) -> Vec<bool> {
        (0..count).rev().map(|i| (value >> i) & 1 != 0).collect()
    }

    fn mbc7_cart() -> Cartridge {
        let mut cart = Cartridge::new(rom_with(0x22, 8, 0)).unwrap();
        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x4000, 0x40);
        cart
    }

    #[test]
    fn mbc7_needs_both_enables_and_latches_the_tilt() {
        let mut cart = Cartridge::new(rom_with(0x22, 8, 0)).unwrap();
        assert!(cart.has_tilt());
        assert_eq!(cart.read_ram(0xA020), 0xFF, "RAM désactivée");
        cart.write_rom(0x0000, 0x0A);
        assert_eq!(cart.read_ram(0xA020), 0xFF, "une seule autorisation ne suffit pas");
        cart.write_rom(0x4000, 0x40);
        cart.set_tilt(0.0, 0.0);
        cart.write_ram(0xA000, 0x55);
        assert_eq!((cart.read_ram(0xA030), cart.read_ram(0xA020)), (0x80, 0x00), "valeurs effacées : 0x8000");
        cart.write_ram(0xA010, 0xAA);
        let x = u16::from_le_bytes([cart.read_ram(0xA020), cart.read_ram(0xA030)]);
        assert_eq!(x, 0x81D0, "console à plat : valeur centrale");
        // incliner à droite change la valeur, mais seulement après une nouvelle capture
        cart.set_tilt(1.0, 0.0);
        let still = u16::from_le_bytes([cart.read_ram(0xA020), cart.read_ram(0xA030)]);
        assert_eq!(still, 0x81D0);
        cart.write_ram(0xA000, 0x55);
        cart.write_ram(0xA010, 0xAA);
        let tilted = u16::from_le_bytes([cart.read_ram(0xA020), cart.read_ram(0xA030)]);
        assert!(tilted < 0x81D0 - 0x40, "{tilted:#06X}");
    }

    #[test]
    fn mbc7_eeprom_write_then_read_round_trips() {
        let mut cart = mbc7_cart();
        // EWEN : démarrage 1, opération 00, adresse 11xxxxxx
        eeprom_clock(&mut cart, false, false);
        eeprom_send(&mut cart, &[true]);
        eeprom_send(&mut cart, &bits(0b00_1100_0000, 10));
        eeprom_clock(&mut cart, false, false);
        // WRITE mot 5 = 0xBEEF
        eeprom_send(&mut cart, &[true]);
        eeprom_send(&mut cart, &bits(0b01_0000_0101, 10));
        eeprom_send(&mut cart, &bits(0xBEEF, 16));
        assert_eq!(cart.read_ram(0xA080) & 1, 1, "l'écriture est terminée : la sortie dit « prêt »");
        eeprom_clock(&mut cart, false, false);
        assert_eq!(&cart.ram()[10..12], &0xBEEFu16.to_le_bytes(), "le mot est dans la RAM, donc dans la sauvegarde");
        // READ mot 5
        eeprom_send(&mut cart, &[true]);
        eeprom_send(&mut cart, &bits(0b10_0000_0101, 10));
        assert_eq!(cart.read_ram(0xA080) & 1, 0, "un bit à 0 précède les données");
        let mut word = 0u16;
        for _ in 0..16 {
            eeprom_clock(&mut cart, true, false);
            word = (word << 1) | (cart.read_ram(0xA080) & 1) as u16;
        }
        assert_eq!(word, 0xBEEF);
    }

    #[test]
    fn mbc7_eeprom_ignores_writes_until_enabled() {
        let mut cart = mbc7_cart();
        eeprom_clock(&mut cart, false, false);
        eeprom_send(&mut cart, &[true]);
        eeprom_send(&mut cart, &bits(0b01_0000_0001, 10)); // WRITE mot 1, sans EWEN
        eeprom_send(&mut cart, &bits(0x1234, 16));
        assert_eq!(&cart.ram()[2..4], &[0xFF, 0xFF]);
    }

    #[test]
    fn pocket_camera_captures_an_image_into_ram_after_a_delay() {
        let mut cart = Cartridge::new(rom_with(0xFC, 8, 5)).unwrap();
        assert!(cart.is_camera());
        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x4000, 0x10); // registres de l'appareil
        cart.write_ram(0xA000, 0x01); // lance la capture
        assert_eq!(cart.read_ram(0xA000) & 1, 1, "occupé");
        cart.set_camera_image(&vec![0u8; CAMERA_W * CAMERA_H]); // tout noir
        cart.tick(CAMERA_CAPTURE_CYCLES);
        assert_eq!(cart.read_ram(0xA000) & 1, 0, "terminé");
        cart.write_rom(0x4000, 0x00); // la RAM normale
        assert_eq!(&cart.ram()[0x100..0x102], &[0xFF, 0xFF], "des pixels noirs : nuance 3 sur les deux plans");
    }

    #[test]
    fn unsupported_exotic_mappers_are_flagged() {
        for cart_type in [0x20u8, 0x0B, 0xFD] {
            let info = CartridgeInfo::parse(&rom_with(cart_type, 4, 0)).unwrap();
            assert!(!info.supported, "{}", info.mapper_name);
        }
    }
}
