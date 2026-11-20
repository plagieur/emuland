# Downloads the test ROMs (Blargg, dmg-acid2, cgb-acid2) into test-roms\ (Windows PowerShell).
# They are not part of this repository: see the README for links and licences.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dir = Join-Path $root "test-roms"
$blargg = "https://raw.githubusercontent.com/retrio/gb-test-roms/master"

function Fetch($path, $dest) {
    # $path is "<base>/<relative path>" with spaces and commas left as they are; only the path part is escaped
    $uri = [uri]$path
    $escaped = ($uri.AbsolutePath -split '/' | ForEach-Object { [uri]::EscapeDataString([uri]::UnescapeDataString($_)) }) -join '/'
    $target = Join-Path $dir $dest
    New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
    Invoke-WebRequest -Uri ($uri.GetLeftPart('Authority') + $escaped) -OutFile $target -UseBasicParsing
}

foreach ($n in "01-special","02-interrupts","03-op sp,hl","04-op r,imm","05-op rp","06-ld r,r","07-jr,jp,call,ret,rst","08-misc instrs","09-op r,r","10-bit ops","11-op a,(hl)") {
    Fetch "$blargg/cpu_instrs/individual/$n.gb" "cpu_instrs\$n.gb"
}
Fetch "$blargg/instr_timing/instr_timing.gb" "instr_timing\instr_timing.gb"
Fetch "$blargg/halt_bug.gb" "halt_bug.gb"
Fetch "$blargg/interrupt_time/interrupt_time.gb" "interrupt_time\interrupt_time.gb"
foreach ($n in "01-read_timing","02-write_timing","03-modify_timing") {
    Fetch "$blargg/mem_timing/individual/$n.gb" "mem_timing\individual\$n.gb"
    Fetch "$blargg/mem_timing-2/rom_singles/$n.gb" "mem_timing-2\rom_singles\$n.gb"
}
$sound = "01-registers","02-len ctr","03-trigger","04-sweep","05-sweep details","06-overflow on trigger","07-len sweep period sync","08-len ctr during power","09-wave read while on","10-wave trigger while on","11-regs after power"
foreach ($n in $sound) {
    Fetch "$blargg/dmg_sound/rom_singles/$n.gb" "dmg_sound\rom_singles\$n.gb"
    Fetch "$blargg/cgb_sound/rom_singles/$n.gb" "cgb_sound\rom_singles\$n.gb"
}
Fetch "$blargg/dmg_sound/rom_singles/12-wave write while on.gb" "dmg_sound\rom_singles\12-wave write while on.gb"
Fetch "$blargg/cgb_sound/rom_singles/12-wave.gb" "cgb_sound\rom_singles\12-wave.gb"
foreach ($n in "1-lcd_sync","2-causes","3-non_causes","4-scanline_timing","5-timing_bug","6-timing_no_bug","7-timing_effect","8-instr_effect") {
    Fetch "$blargg/oam_bug/rom_singles/$n.gb" "oam_bug\rom_singles\$n.gb"
}
Fetch "https://github.com/mattcurrie/dmg-acid2/releases/download/v1.0/dmg-acid2.gb" "dmg-acid2\dmg-acid2.gb"
Fetch "https://github.com/mattcurrie/cgb-acid2/releases/download/v1.1/cgb-acid2.gbc" "cgb-acid2\cgb-acid2.gbc"
Write-Host "Test ROMs downloaded into $dir"
