// Integration tests for Game Gear emulation
//
// These tests load actual BIOS/ROM files and run the emulation loop for
// several frames headlessly. They verify that no panics or infinite loops
// occur during execution.
//
// Run with: cargo test -- --ignored

use std::path::Path;

use rustygear::bus::Bus;
use rustygear::cartridge::Cartridge;
use rustygear::cpu::Cpu;

const CYCLES_PER_SCANLINE: u32 = 228;
const SCANLINES_PER_FRAME: u16 = 262;

/// Run the emulation loop for a given number of frames, returning the total
/// number of T-cycles executed. If the CPU appears stuck (zero progress after
/// a full scanline), we break early rather than hang forever.
fn run_frames(bus: &mut Bus, cpu: &mut Cpu, frames: u32) -> u64 {
    let mut total_cycles: u64 = 0;
    for _ in 0..frames {
        for _ in 0..SCANLINES_PER_FRAME {
            let mut cycles_this_line: u32 = 0;
            while cycles_this_line < CYCLES_PER_SCANLINE {
                let c = cpu.step(bus);
                if c == 0 {
                    // Safety valve: if step() returns 0 cycles we'd spin forever
                    panic!("cpu.step() returned 0 cycles -- possible infinite loop");
                }
                cycles_this_line += c;
            }
            bus.vdp.run_scanline();
            bus.irq_pending = bus.vdp.irq_pending();
            cpu.handle_interrupts(bus);
            total_cycles += cycles_this_line as u64;
        }
    }
    total_cycles
}

/// Helper: load a ROM file, returning its bytes. Panics with a helpful message
/// if the file is missing.
fn load_file(path: &str) -> Vec<u8> {
    let p = Path::new(path);
    if !p.exists() {
        panic!(
            "Required file not found: '{}'. \
             Integration tests need actual BIOS/ROM files to run.",
            path
        );
    }
    std::fs::read(p).unwrap_or_else(|e| panic!("Failed to read '{}': {}", path, e))
}

// ---------------------------------------------------------------------------
// BIOS-only boot test (no game ROM -- use a dummy 16KB ROM of zeroes)
// ---------------------------------------------------------------------------
#[test]
#[ignore]
fn test_bios_boot() {
    let bios = load_file("bios/bios.gg");
    // Create a minimal dummy ROM (16KB of 0x00 -- NOP-land if the BIOS
    // ever tries to jump into cartridge space).
    let dummy_rom = vec![0u8; 16384];
    let cartridge = Cartridge::new(dummy_rom);
    let mut bus = Bus::new(cartridge, Some(bios));
    let mut cpu = Cpu::new();

    let total_cycles = run_frames(&mut bus, &mut cpu, 60);

    // Sanity check: the CPU should have executed a non-trivial number of
    // cycles over 60 frames. Each frame has 262 scanlines * 228 cycles =
    // ~59736 cycles, so 60 frames is roughly 3.6 million minimum.
    assert!(
        total_cycles > 1_000_000,
        "Expected at least 1M cycles over 60 frames, got {}",
        total_cycles
    );
    println!(
        "test_bios_boot: completed 60 frames, {} total cycles, PC={:04X}",
        total_cycles, cpu.pc
    );
}
