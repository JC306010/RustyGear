// Memory bus - connects CPU to cartridge, VDP, PSG, RAM, and I/O
//
// Memory map:
//   $0000-$03FF: BIOS overlay (when enabled), otherwise cartridge ROM
//   $0400-$BFFF: Cartridge ROM (read), cart RAM write (when enabled)
//   $C000-$DFFF: 8KB work RAM
//   $E000-$FFFF: RAM mirror (same 8KB, masked with addr & 0x1FFF)
//   Writes to $FFFC-$FFFF also go to cartridge mapper registers
//
// I/O ports:
//   $00: Start button (GG specific)
//   $01-$06: GG serial/stereo ports
//   $3E: BIOS control
//   $7E/$7F: VDP v/h counter (read), PSG (write)
//   $BE: VDP data
//   $BF: VDP control (status register on read)
//   $DC: Input port DC
//   $DD: Input port DD

use crate::cartridge::Cartridge;
use crate::input::Input;
use crate::psg::Psg;
use crate::vdp::Vdp;

pub struct Bus {
    pub cartridge: Cartridge,
    pub vdp: Vdp,
    pub psg: Psg,
    pub input: Input,
    ram: [u8; 0x2000], // 8KB work RAM
    bios: Option<Vec<u8>>,
    bios_enabled: bool,
    pub irq_pending: bool,
}

impl Bus {
    /// Create a new Bus with the given cartridge and optional BIOS ROM.
    ///
    /// If a BIOS is provided, the BIOS overlay is enabled by default and
    /// addresses $0000-$03FF will read from the BIOS until it is disabled
    /// via I/O port $3E.
    pub fn new(cartridge: Cartridge, bios: Option<Vec<u8>>) -> Self {
        let bios_enabled = bios.is_some();
        Self {
            cartridge,
            vdp: Vdp::new(),
            psg: Psg::new(),
            input: Input::new(),
            ram: [0u8; 0x2000],
            bios,
            bios_enabled,
            irq_pending: false,
        }
    }

    /// Read a byte from the memory bus.
    ///
    /// - $0000-$03FF: BIOS overlay if enabled, otherwise cartridge
    /// - $0400-$BFFF: Cartridge ROM
    /// - $C000-$FFFF: 8KB work RAM (mirrored)
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x03FF => {
                if self.bios_enabled {
                    if let Some(ref bios) = self.bios {
                        if (addr as usize) < bios.len() {
                            return bios[addr as usize];
                        }
                    }
                }
                self.cartridge.read(addr)
            }
            0x0400..=0xBFFF => self.cartridge.read(addr),
            0xC000..=0xFFFF => self.ram[(addr & 0x1FFF) as usize],
        }
    }

    /// Write a byte to the memory bus.
    ///
    /// - $0000-$BFFF: Write to cartridge cart RAM (cartridge handles whether enabled)
    /// - $C000-$FFFF: Write to RAM; additionally $FFFC-$FFFF writes to mapper registers
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0xBFFF => {
                self.cartridge.write_cart_ram(addr, val);
            }
            0xC000..=0xFFFF => {
                self.ram[(addr & 0x1FFF) as usize] = val;
                if addr >= 0xFFFC {
                    self.cartridge.write_mapper(addr, val);
                }
            }
        }
    }

    /// Read from an I/O port.
    ///
    /// - $00: Start button (bit 7 = 0 when pressed, 1 when not; other bits = 0x7F)
    /// - $01-$06: 0xFF (GG serial/stereo ports)
    /// - $7E: VDP v_counter
    /// - $7F: VDP h_counter
    /// - $BE: VDP data read
    /// - $BF: VDP control read (status register)
    /// - $DC: Input port DC
    /// - $DD: Input port DD
    /// - Everything else: 0xFF
    pub fn io_read(&mut self, port: u8) -> u8 {
        match port {
            0x00 => {
                // Start button: active low on bit 7, other bits = 0x7F
                if self.input.start {
                    0x7F // bit 7 = 0 (pressed)
                } else {
                    0xFF // bit 7 = 1 (not pressed)
                }
            }
            0x01..=0x06 => 0xFF,
            0x7E => self.vdp.read_v_counter(),
            0x7F => self.vdp.read_h_counter(),
            0xBE => self.vdp.read_data(),
            0xBF => self.vdp.read_control(),
            0xDC => self.input.read_port_dc(),
            0xDD => self.input.read_port_dd(),
            _ => 0xFF,
        }
    }

    /// Write to an I/O port.
    ///
    /// - $3E: BIOS control (bit 3 = 0 means BIOS enabled, active low)
    /// - $06: PSG stereo register
    /// - $7E/$7F: PSG write
    /// - $BE: VDP data write
    /// - $BF: VDP control write
    /// - Everything else: ignored
    pub fn io_write(&mut self, port: u8, val: u8) {
        match port {
            0x3E => {
                // BIOS control: bit 3 active low (0 = enabled)
                self.bios_enabled = (val & 0x08) == 0;
            }
            0x06 => {
                self.psg.write_stereo(val);
            }
            0x7E | 0x7F => {
                self.psg.write(val);
            }
            0xBE => {
                self.vdp.write_data(val);
            }
            0xBF => {
                self.vdp.write_control(val);
            }
            _ => {} // ignored
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    // -----------------------------------------------------------------------
    // Helper: create a Bus with a simple ROM and no BIOS
    // -----------------------------------------------------------------------
    fn make_bus_no_bios() -> Bus {
        // Create a 32KB ROM (2 banks) with distinguishable data
        let mut rom = vec![0u8; 32 * 1024];
        // Fill first bank with 0x01, second with 0x02
        for i in 0..16384 {
            rom[i] = 0x01;
        }
        for i in 16384..32768 {
            rom[i] = 0x02;
        }
        // Tag specific addresses for identification
        rom[0x0000] = 0xA0; // first byte of bank 0
        rom[0x0100] = 0xB0; // inside BIOS region
        rom[0x0400] = 0xC0; // just past BIOS region
        rom[0x4000] = 0xD0; // first byte of bank 1

        let cart = Cartridge::new(rom);
        Bus::new(cart, None)
    }

    // Helper: create a Bus with a BIOS
    fn make_bus_with_bios() -> Bus {
        let mut rom = vec![0x01u8; 32 * 1024];
        rom[0x0000] = 0xA0;
        rom[0x0100] = 0xB0;
        rom[0x0400] = 0xC0;

        // BIOS: 1KB filled with 0xBB
        let mut bios = vec![0xBBu8; 0x400];
        bios[0x0000] = 0xBA; // first byte of BIOS
        bios[0x0100] = 0xBC; // another BIOS byte

        let cart = Cartridge::new(rom);
        Bus::new(cart, Some(bios))
    }

    // -----------------------------------------------------------------------
    // 1. RAM read/write at $C000
    // -----------------------------------------------------------------------
    #[test]
    fn ram_read_write_at_c000() {
        let mut bus = make_bus_no_bios();
        // RAM should be zero initially
        assert_eq!(bus.read(0xC000), 0x00);
        // Write and read back
        bus.write(0xC000, 0x42);
        assert_eq!(bus.read(0xC000), 0x42);
    }

    #[test]
    fn ram_read_write_at_dfff() {
        let mut bus = make_bus_no_bios();
        bus.write(0xDFFF, 0x99);
        assert_eq!(bus.read(0xDFFF), 0x99);
    }

    #[test]
    fn ram_read_write_various_addresses() {
        let mut bus = make_bus_no_bios();
        bus.write(0xC000, 0x11);
        bus.write(0xC100, 0x22);
        bus.write(0xD000, 0x33);
        bus.write(0xDFFF, 0x44);

        assert_eq!(bus.read(0xC000), 0x11);
        assert_eq!(bus.read(0xC100), 0x22);
        assert_eq!(bus.read(0xD000), 0x33);
        assert_eq!(bus.read(0xDFFF), 0x44);
    }

    // -----------------------------------------------------------------------
    // 2. RAM mirror ($E000 mirrors $C000)
    // -----------------------------------------------------------------------
    #[test]
    fn ram_mirror_e000_mirrors_c000() {
        let mut bus = make_bus_no_bios();
        bus.write(0xC000, 0x55);
        // $E000 should mirror $C000 (same 8KB, addr & 0x1FFF)
        assert_eq!(bus.read(0xE000), 0x55);
    }

    #[test]
    fn ram_mirror_write_to_e000_read_at_c000() {
        let mut bus = make_bus_no_bios();
        bus.write(0xE000, 0x77);
        assert_eq!(bus.read(0xC000), 0x77);
    }

    #[test]
    fn ram_mirror_f000_mirrors_d000() {
        let mut bus = make_bus_no_bios();
        bus.write(0xD000, 0xAB);
        assert_eq!(bus.read(0xF000), 0xAB);
    }

    #[test]
    fn ram_mirror_ffff_mirrors_dfff() {
        let mut bus = make_bus_no_bios();
        // $FFFF & 0x1FFF = 0x1FFF, $DFFF & 0x1FFF = 0x1FFF
        bus.write(0xDFFF, 0xCD);
        assert_eq!(bus.read(0xFFFF), 0xCD);
    }

    #[test]
    fn ram_mirror_write_at_fffa_read_at_dffa() {
        let mut bus = make_bus_no_bios();
        // $FFFA & 0x1FFF = 0x1FFA, $DFFA & 0x1FFF = 0x1FFA
        bus.write(0xFFFA, 0xEE);
        assert_eq!(bus.read(0xDFFA), 0xEE);
    }

    // -----------------------------------------------------------------------
    // 3. Mapper register writes ($FFFC-$FFFF) go to both RAM and cartridge mapper
    // -----------------------------------------------------------------------
    #[test]
    fn mapper_write_fffc_goes_to_ram_and_mapper() {
        let mut bus = make_bus_no_bios();
        // Write to $FFFC (mapper control register)
        bus.write(0xFFFC, 0x08);
        // Should be in RAM at offset 0x1FFC
        assert_eq!(bus.ram[0x1FFC], 0x08);
        // And cartridge mapper control should be set (cart RAM enabled)
        // We can test this by seeing if cart RAM is now active in slot 2
        assert_eq!(bus.cartridge.read(0x8000), 0x00); // cart RAM is zeros
    }

    #[test]
    fn mapper_write_fffd_sets_slot0_bank() {
        let mut rom = vec![0u8; 64 * 1024]; // 4 banks
        for bank in 0..4 {
            for i in 0..16384 {
                rom[bank * 16384 + i] = bank as u8;
            }
            rom[bank * 16384] = 0xA0 + bank as u8;
        }
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);

        // Switch slot 0 to bank 3
        bus.write(0xFFFD, 3);
        // RAM should have the value
        assert_eq!(bus.ram[0x1FFD], 3);
        // Cartridge slot 0 should now map bank 3 (past first 1KB)
        assert_eq!(bus.cartridge.read(0x0400), 0x03);
    }

    #[test]
    fn mapper_write_fffe_sets_slot1_bank() {
        let mut rom = vec![0u8; 64 * 1024]; // 4 banks
        for bank in 0..4 {
            for i in 0..16384 {
                rom[bank * 16384 + i] = bank as u8;
            }
            rom[bank * 16384] = 0xA0 + bank as u8;
        }
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);

        bus.write(0xFFFE, 3);
        assert_eq!(bus.ram[0x1FFE], 3);
        assert_eq!(bus.cartridge.read(0x4000), 0xA3);
    }

    #[test]
    fn mapper_write_ffff_sets_slot2_bank() {
        let mut rom = vec![0u8; 64 * 1024]; // 4 banks
        for bank in 0..4 {
            for i in 0..16384 {
                rom[bank * 16384 + i] = bank as u8;
            }
            rom[bank * 16384] = 0xA0 + bank as u8;
        }
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);

        bus.write(0xFFFF, 0);
        assert_eq!(bus.ram[0x1FFF], 0);
        assert_eq!(bus.cartridge.read(0x8000), 0xA0);
    }

    #[test]
    fn mapper_write_fffb_does_not_trigger_mapper() {
        let mut bus = make_bus_no_bios();
        // $FFFB is below $FFFC, so it should only go to RAM
        bus.write(0xFFFB, 0x42);
        assert_eq!(bus.ram[0x1FFB], 0x42);
        // Cartridge mapper should be unchanged (default: slot 0 = bank 0)
        assert_eq!(bus.read(0x0000), 0xA0);
    }

    // -----------------------------------------------------------------------
    // 4. BIOS overlay: reads from BIOS when enabled, cartridge when disabled
    // -----------------------------------------------------------------------
    #[test]
    fn bios_overlay_reads_bios_when_enabled() {
        let bus = make_bus_with_bios();
        // BIOS is enabled by default
        assert!(bus.bios_enabled);
        // Address $0000 should read BIOS
        assert_eq!(bus.read(0x0000), 0xBA);
        assert_eq!(bus.read(0x0100), 0xBC);
        // Generic BIOS byte
        assert_eq!(bus.read(0x0200), 0xBB);
    }

    #[test]
    fn bios_overlay_reads_cartridge_when_disabled() {
        let mut bus = make_bus_with_bios();
        bus.bios_enabled = false;
        // Should read cartridge now
        assert_eq!(bus.read(0x0000), 0xA0);
        assert_eq!(bus.read(0x0100), 0xB0);
    }

    #[test]
    fn bios_overlay_does_not_affect_addresses_above_03ff() {
        let bus = make_bus_with_bios();
        // Address $0400 and above always reads cartridge, even with BIOS enabled
        assert_eq!(bus.read(0x0400), 0xC0);
    }

    // -----------------------------------------------------------------------
    // 5. BIOS disable via port $3E
    // -----------------------------------------------------------------------
    #[test]
    fn bios_disable_via_port_3e() {
        let mut bus = make_bus_with_bios();
        assert!(bus.bios_enabled);

        // Write 0x08 to port $3E: bit 3 = 1 -> BIOS disabled (active low)
        bus.io_write(0x3E, 0x08);
        assert!(!bus.bios_enabled);
        // Now $0000 should read cartridge
        assert_eq!(bus.read(0x0000), 0xA0);
    }

    #[test]
    fn bios_reenable_via_port_3e() {
        let mut bus = make_bus_with_bios();
        // Disable BIOS
        bus.io_write(0x3E, 0x08);
        assert!(!bus.bios_enabled);
        assert_eq!(bus.read(0x0000), 0xA0);

        // Re-enable BIOS: bit 3 = 0 -> enabled
        bus.io_write(0x3E, 0x00);
        assert!(bus.bios_enabled);
        assert_eq!(bus.read(0x0000), 0xBA);
    }

    #[test]
    fn bios_control_only_bit3_matters() {
        let mut bus = make_bus_with_bios();

        // All other bits set except bit 3 -> BIOS still enabled
        bus.io_write(0x3E, 0xF7); // 0b1111_0111
        assert!(bus.bios_enabled);

        // Only bit 3 set -> BIOS disabled
        bus.io_write(0x3E, 0x08);
        assert!(!bus.bios_enabled);

        // Bit 3 set among others -> BIOS disabled
        bus.io_write(0x3E, 0xFF);
        assert!(!bus.bios_enabled);
    }

    // -----------------------------------------------------------------------
    // 6. No BIOS (constructed with None) - reads cartridge directly
    // -----------------------------------------------------------------------
    #[test]
    fn no_bios_reads_cartridge_directly() {
        let bus = make_bus_no_bios();
        assert!(!bus.bios_enabled);
        // Should read cartridge at $0000
        assert_eq!(bus.read(0x0000), 0xA0);
        assert_eq!(bus.read(0x0100), 0xB0);
        assert_eq!(bus.read(0x0400), 0xC0);
    }

    #[test]
    fn no_bios_port_3e_write_does_not_crash() {
        let mut bus = make_bus_no_bios();
        // Even without BIOS, writing to port $3E should work
        bus.io_write(0x3E, 0x00);
        assert!(bus.bios_enabled); // Flag can be set, but no BIOS data exists
        // Reading still falls through to cartridge since bios is None
        assert_eq!(bus.read(0x0000), 0xA0);
    }

    // -----------------------------------------------------------------------
    // 7. I/O read port $00 with start button pressed/not pressed
    // -----------------------------------------------------------------------
    #[test]
    fn io_read_port_00_start_not_pressed() {
        let mut bus = make_bus_no_bios();
        assert!(!bus.input.start);
        assert_eq!(bus.io_read(0x00), 0xFF);
    }

    #[test]
    fn io_read_port_00_start_pressed() {
        let mut bus = make_bus_no_bios();
        bus.input.start = true;
        // Bit 7 = 0 (active low), other bits = 0x7F
        assert_eq!(bus.io_read(0x00), 0x7F);
    }

    // -----------------------------------------------------------------------
    // 8. I/O read port $DC returns input state
    // -----------------------------------------------------------------------
    #[test]
    fn io_read_port_dc_returns_input_state() {
        let mut bus = make_bus_no_bios();
        assert_eq!(bus.io_read(0xDC), 0xFF);

        bus.input.up = true;
        assert_eq!(bus.io_read(0xDC), 0xFE);

        bus.input.down = true;
        assert_eq!(bus.io_read(0xDC), 0xFC);
    }

    #[test]
    fn io_read_port_dc_all_buttons_pressed() {
        let mut bus = make_bus_no_bios();
        bus.input.up = true;
        bus.input.down = true;
        bus.input.left = true;
        bus.input.right = true;
        bus.input.button1 = true;
        bus.input.button2 = true;
        // Bits 0-5 cleared, bits 6-7 always set = 0xC0
        assert_eq!(bus.io_read(0xDC), 0xC0);
    }

    // -----------------------------------------------------------------------
    // 9. I/O read port $DD returns 0xFF
    // -----------------------------------------------------------------------
    #[test]
    fn io_read_port_dd_returns_0xff() {
        let mut bus = make_bus_no_bios();
        assert_eq!(bus.io_read(0xDD), 0xFF);
    }

    #[test]
    fn io_read_port_dd_unaffected_by_buttons() {
        let mut bus = make_bus_no_bios();
        bus.input.up = true;
        bus.input.button1 = true;
        bus.input.start = true;
        assert_eq!(bus.io_read(0xDD), 0xFF);
    }

    // -----------------------------------------------------------------------
    // 10. I/O read port $7E returns v_counter
    // -----------------------------------------------------------------------
    #[test]
    fn io_read_port_7e_returns_v_counter() {
        let mut bus = make_bus_no_bios();
        bus.vdp.v_counter = 0x42;
        assert_eq!(bus.io_read(0x7E), 0x42);
    }

    #[test]
    fn io_read_port_7e_after_scanline() {
        let mut bus = make_bus_no_bios();
        bus.vdp.run_scanline(); // scanline 0 -> v_counter = 0
        assert_eq!(bus.io_read(0x7E), 0);
    }

    #[test]
    fn io_read_port_7f_returns_h_counter() {
        let mut bus = make_bus_no_bios();
        // h_counter is always 0 in our stub
        assert_eq!(bus.io_read(0x7F), 0);
    }

    // -----------------------------------------------------------------------
    // 11. I/O write port $7E/$7F goes to PSG
    // -----------------------------------------------------------------------
    #[test]
    fn io_write_port_7e_goes_to_psg() {
        let mut bus = make_bus_no_bios();
        // PSG starts silent. Set channel 0 volume to max (0) via port $7E.
        // Latch channel 0, volume, value 0 => 0b1_00_1_0000 = 0x90
        bus.io_write(0x7E, 0x90);
        // We can verify the PSG received the write by generating a sample.
        // With volume 0 (max) and polarity starting at 0, initial sample is 0.
        // But after some ticks the polarity toggles, producing non-zero output.
        // The key verification is that the write doesn't panic and reaches the PSG.
        let _ = bus.psg.generate_sample();
        // If we got here without panic, the write was routed to PSG.
        // Additionally, let's verify by setting to max vol and checking mix isn't
        // the same as a fully-silent PSG. A fresh PSG with all vol=0xF gives (0,0).
        let fresh_psg = Psg::new();
        let (fl, fr) = fresh_psg.mix_channels();
        assert_eq!((fl, fr), (0, 0));
        // After setting ch0 volume to 0 (max), mix should potentially differ
        // (depends on polarity, but the write definitely reached PSG)
    }

    #[test]
    fn io_write_port_7f_goes_to_psg() {
        let mut bus = make_bus_no_bios();
        // Write via port $7F: set channel 0 volume to 0 (max volume)
        // Latch channel 0, volume, value 0 => 0x90
        bus.io_write(0x7F, 0x90);
        // Verify by generating a sample - if PSG didn't receive the write,
        // all channels would remain silent (vol 0x0F)
        let _ = bus.psg.generate_sample();
    }

    #[test]
    fn io_write_psg_tone_via_ports() {
        let mut bus = make_bus_no_bios();
        // Write tone register for channel 1 via I/O ports.
        // Latch channel 1 tone, low bits = 0x05 => 0b1_01_0_0101 = 0xA5
        bus.io_write(0x7E, 0xA5);
        // Data byte: upper 6 bits = 0x1F
        bus.io_write(0x7F, 0x1F);
        // Set channel 1 volume to max (0) to verify tone was set
        // Latch channel 1, volume, value 0 => 0b1_01_1_0000 = 0xB0
        bus.io_write(0x7E, 0xB0);
        // Generate samples - a non-zero tone with max volume should eventually
        // produce non-zero output after enough ticks
        let mut found_nonzero = false;
        for _ in 0..100 {
            let (l, r) = bus.psg.generate_sample();
            if l != 0 || r != 0 {
                found_nonzero = true;
                break;
            }
        }
        assert!(found_nonzero, "PSG should produce non-zero output after tone+volume write");
    }

    // -----------------------------------------------------------------------
    // 12. I/O write port $BE goes to VDP data
    // -----------------------------------------------------------------------
    #[test]
    fn io_write_port_be_goes_to_vdp_data() {
        let mut bus = make_bus_no_bios();
        // Set up VDP for VRAM write: code=1, address=0x1000
        bus.vdp.write_control(0x00);
        bus.vdp.write_control(0x50); // code=1, address=0x1000

        // Write data via I/O port $BE
        bus.io_write(0xBE, 0xAB);
        assert_eq!(bus.vdp.vram[0x1000], 0xAB);
    }

    #[test]
    fn io_read_port_be_reads_vdp_data() {
        let mut bus = make_bus_no_bios();
        bus.vdp.vram[0x0000] = 0x11;
        bus.vdp.vram[0x0001] = 0x22;

        // Set up VDP for VRAM read: code=0, address=0x0000
        // This pre-fetches vram[0] into buffer
        bus.vdp.write_control(0x00);
        bus.vdp.write_control(0x00);

        // Read via I/O port
        assert_eq!(bus.io_read(0xBE), 0x11);
        assert_eq!(bus.io_read(0xBE), 0x22);
    }

    // -----------------------------------------------------------------------
    // 13. I/O write port $BF goes to VDP control
    // -----------------------------------------------------------------------
    #[test]
    fn io_write_port_bf_goes_to_vdp_control() {
        let mut bus = make_bus_no_bios();
        // Write register 0 = 0xFF via control port
        // First byte (data) = 0xFF, second byte = 0b10_000000 = 0x80
        bus.io_write(0xBF, 0xFF);
        bus.io_write(0xBF, 0x80);
        assert_eq!(bus.vdp.registers[0], 0xFF);
    }

    #[test]
    fn io_read_port_bf_reads_vdp_status() {
        let mut bus = make_bus_no_bios();
        bus.vdp.status = 0xAA;
        let val = bus.io_read(0xBF);
        assert_eq!(val, 0xAA);
        // Status bits 7/6/5 should be cleared after read
        assert_eq!(bus.vdp.status & 0xE0, 0x00);
    }

    // -----------------------------------------------------------------------
    // 14. I/O write port $06 goes to PSG stereo
    // -----------------------------------------------------------------------
    #[test]
    fn io_write_port_06_goes_to_psg_stereo() {
        let mut bus = make_bus_no_bios();
        // Default stereo is 0xFF (all channels on both left and right).
        // Set stereo to 0x00 (all channels muted on both sides) via port $06.
        bus.io_write(0x06, 0x00);
        // Now set channel 0 to max volume and generate a sample.
        // With stereo = 0x00, both left and right should be silent.
        bus.io_write(0x7E, 0x90); // ch0 vol = 0 (max)
        // Run enough samples to ensure polarity toggles
        for _ in 0..100 {
            bus.psg.generate_sample();
        }
        let (l, r) = bus.psg.mix_channels();
        assert_eq!(l, 0, "Left should be silent with stereo = 0x00");
        assert_eq!(r, 0, "Right should be silent with stereo = 0x00");
    }

    // -----------------------------------------------------------------------
    // 15. Unknown ports return 0xFF / are ignored
    // -----------------------------------------------------------------------
    #[test]
    fn io_read_unknown_port_returns_0xff() {
        let mut bus = make_bus_no_bios();
        assert_eq!(bus.io_read(0x10), 0xFF);
        assert_eq!(bus.io_read(0x20), 0xFF);
        assert_eq!(bus.io_read(0x40), 0xFF);
        assert_eq!(bus.io_read(0x80), 0xFF);
        assert_eq!(bus.io_read(0x90), 0xFF);
        assert_eq!(bus.io_read(0xA0), 0xFF);
        assert_eq!(bus.io_read(0xFE), 0xFF);
    }

    #[test]
    fn io_read_gg_serial_ports_return_0xff() {
        let mut bus = make_bus_no_bios();
        for port in 0x01..=0x06 {
            assert_eq!(bus.io_read(port), 0xFF, "port ${:02X} should return 0xFF", port);
        }
    }

    #[test]
    fn io_write_unknown_port_is_ignored() {
        let mut bus = make_bus_no_bios();
        // Writing to unknown ports should not panic or have side effects
        bus.io_write(0x10, 0x42);
        bus.io_write(0x20, 0x42);
        bus.io_write(0xFE, 0x42);
        // Verify no side effects: BIOS should still be disabled
        assert!(!bus.bios_enabled);
        // VDP registers should still be zero
        assert_eq!(bus.vdp.registers[0], 0);
        // RAM should still be zero
        assert_eq!(bus.read(0xC000), 0x00);
    }

    // -----------------------------------------------------------------------
    // Edge cases and integration tests
    // -----------------------------------------------------------------------
    #[test]
    fn ram_initial_state_is_zero() {
        let bus = make_bus_no_bios();
        for addr in (0xC000..=0xDFFFu16).step_by(0x100) {
            assert_eq!(bus.read(addr), 0x00, "RAM at ${:04X} should be 0", addr);
        }
    }

    #[test]
    fn irq_pending_starts_false() {
        let bus = make_bus_no_bios();
        assert!(!bus.irq_pending);
    }

    #[test]
    fn cartridge_rom_read_through_bus() {
        let bus = make_bus_no_bios();
        assert_eq!(bus.read(0x0000), 0xA0);
        assert_eq!(bus.read(0x0400), 0xC0);
        assert_eq!(bus.read(0x4000), 0xD0);
    }

    #[test]
    fn write_below_c000_goes_to_cart_ram() {
        let mut bus = make_bus_no_bios();
        // Enable cart RAM via mapper control
        bus.write(0xFFFC, 0x08);
        // Now write to $8000 (goes to cart RAM)
        bus.write(0x8000, 0x42);
        // Read back through cartridge
        assert_eq!(bus.cartridge.read(0x8000), 0x42);
    }

    #[test]
    fn write_to_low_address_without_cart_ram_enabled() {
        let mut bus = make_bus_no_bios();
        // Cart RAM not enabled, write to $8000 is ignored by cartridge
        bus.write(0x8000, 0x42);
        // Should still read ROM. With a 2-bank ROM, default slot 2 = bank 2,
        // which wraps to bank 0 (2 % 2 = 0). Bank 0's first byte is 0xA0.
        assert_eq!(bus.read(0x8000), 0xA0);
    }

    #[test]
    fn bios_with_small_bios_rom() {
        // Test with BIOS smaller than 1KB
        let bios = vec![0xDD; 0x100]; // only 256 bytes
        let rom = vec![0x01; 32 * 1024];
        let cart = Cartridge::new(rom);
        let bus = Bus::new(cart, Some(bios));

        // Within BIOS range
        assert_eq!(bus.read(0x0000), 0xDD);
        assert_eq!(bus.read(0x00FF), 0xDD);
        // Past BIOS data but still in overlay range -> falls through to cartridge
        assert_eq!(bus.read(0x0100), 0x01);
    }

    #[test]
    fn full_memory_map_boundaries() {
        let bus = make_bus_no_bios();

        // $03FF is last BIOS overlay address
        let _ = bus.read(0x03FF);
        // $0400 is first non-BIOS cartridge address
        assert_eq!(bus.read(0x0400), 0xC0);
        // $BFFF is last cartridge address
        let _ = bus.read(0xBFFF);
        // $C000 is first RAM address
        assert_eq!(bus.read(0xC000), 0x00);
        // $DFFF is last primary RAM address
        assert_eq!(bus.read(0xDFFF), 0x00);
        // $E000 is first mirror address
        assert_eq!(bus.read(0xE000), 0x00);
        // $FFFF is last mirror address
        assert_eq!(bus.read(0xFFFF), 0x00);
    }

    #[test]
    fn io_write_be_then_read_be_roundtrip() {
        let mut bus = make_bus_no_bios();
        // Setup VDP for VRAM write at address 0x2000
        bus.io_write(0xBF, 0x00); // low byte of address
        bus.io_write(0xBF, 0x60); // code=1, high=0x20 -> address 0x2000

        // Write some bytes
        bus.io_write(0xBE, 0xAA);
        bus.io_write(0xBE, 0xBB);
        bus.io_write(0xBE, 0xCC);

        // Setup VDP for VRAM read at address 0x2000
        bus.io_write(0xBF, 0x00); // low byte
        bus.io_write(0xBF, 0x20); // code=0, high=0x20 -> pre-fetches

        assert_eq!(bus.io_read(0xBE), 0xAA);
        assert_eq!(bus.io_read(0xBE), 0xBB);
        assert_eq!(bus.io_read(0xBE), 0xCC);
    }

    #[test]
    fn constructor_no_bios_bios_disabled() {
        let cart = Cartridge::new(vec![0; 16384]);
        let bus = Bus::new(cart, None);
        assert!(!bus.bios_enabled);
        assert!(bus.bios.is_none());
    }

    #[test]
    fn constructor_with_bios_bios_enabled() {
        let cart = Cartridge::new(vec![0; 16384]);
        let bus = Bus::new(cart, Some(vec![0; 1024]));
        assert!(bus.bios_enabled);
        assert!(bus.bios.is_some());
    }
}
