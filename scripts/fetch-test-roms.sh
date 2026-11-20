#!/usr/bin/env bash
# Télécharge les ROMs de test (Blargg, dmg-acid2, cgb-acid2) dans test-roms/.
# Elles ne sont pas dans le dépôt : ce sont des programmes écrits par d'autres auteurs, qui servent seulement
# à vérifier l'émulateur. Voir le README pour les liens et les licences.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/test-roms"
blargg="https://raw.githubusercontent.com/retrio/gb-test-roms/master"

fetch() { # fetch <url> <destination relative à test-roms/>
  local url="${1// /%20}" dest="$dir/$2"
  mkdir -p "$(dirname "$dest")"
  curl -fsSL "$url" -o "$dest"
}

# cpu_instrs : on aplatit le dossier « individual »
for n in "01-special" "02-interrupts" "03-op sp,hl" "04-op r,imm" "05-op rp" "06-ld r,r" "07-jr,jp,call,ret,rst" \
         "08-misc instrs" "09-op r,r" "10-bit ops" "11-op a,(hl)"; do
  fetch "$blargg/cpu_instrs/individual/$n.gb" "cpu_instrs/$n.gb"
done

fetch "$blargg/instr_timing/instr_timing.gb" "instr_timing/instr_timing.gb"
fetch "$blargg/halt_bug.gb" "halt_bug.gb"
fetch "$blargg/interrupt_time/interrupt_time.gb" "interrupt_time/interrupt_time.gb"

for n in "01-read_timing" "02-write_timing" "03-modify_timing"; do
  fetch "$blargg/mem_timing/individual/$n.gb" "mem_timing/individual/$n.gb"
  fetch "$blargg/mem_timing-2/rom_singles/$n.gb" "mem_timing-2/rom_singles/$n.gb"
done

for n in "01-registers" "02-len ctr" "03-trigger" "04-sweep" "05-sweep details" "06-overflow on trigger" \
         "07-len sweep period sync" "08-len ctr during power" "09-wave read while on" "10-wave trigger while on" \
         "11-regs after power" "12-wave write while on"; do
  fetch "$blargg/dmg_sound/rom_singles/$n.gb" "dmg_sound/rom_singles/$n.gb"
done
for n in "01-registers" "02-len ctr" "03-trigger" "04-sweep" "05-sweep details" "06-overflow on trigger" \
         "07-len sweep period sync" "08-len ctr during power" "09-wave read while on" "10-wave trigger while on" \
         "11-regs after power" "12-wave"; do
  fetch "$blargg/cgb_sound/rom_singles/$n.gb" "cgb_sound/rom_singles/$n.gb"
done

for n in "1-lcd_sync" "2-causes" "3-non_causes" "4-scanline_timing" "5-timing_bug" "6-timing_no_bug" "7-timing_effect" "8-instr_effect"; do
  fetch "$blargg/oam_bug/rom_singles/$n.gb" "oam_bug/rom_singles/$n.gb"
done

fetch "https://github.com/mattcurrie/dmg-acid2/releases/download/v1.0/dmg-acid2.gb" "dmg-acid2/dmg-acid2.gb"
fetch "https://github.com/mattcurrie/cgb-acid2/releases/download/v1.1/cgb-acid2.gbc" "cgb-acid2/cgb-acid2.gbc"

echo "ROMs de test téléchargées dans $dir"
