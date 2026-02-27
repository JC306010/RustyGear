// Cartridge ROM loading and Sega mapper bank switching
//
// The Sega mapper divides the 48KB cartridge address space ($0000-$BFFF) into
// three 16KB slots:
//   Slot 0: $0000-$3FFF (mapped via $FFFD)
//   Slot 1: $4000-$7FFF (mapped via $FFFE)
//   Slot 2: $8000-$BFFF (mapped via $FFFF)
//
// Special rules:
//   - The first 1KB ($0000-$03FF) always reads from the first 1KB of ROM bank 0,
//     regardless of the slot 0 bank register.
//   - The control register at $FFFC can enable cart RAM at $8000-$BFFF.
//
// Control register ($FFFC) bits:
//   Bit 3: Cart RAM enable (1 = RAM at $8000-$BFFF instead of ROM)
//   Bit 2: Cart RAM bank select (0 = first 16KB, 1 = second 16KB)
//   Bit 7: ROM write enable (unused for reads, but stored)
//
// Default state: slot 0 = bank 0, slot 1 = bank 1, slot 2 = bank 2, control = 0

/// Sega mapper cartridge with ROM and optional cart RAM.
pub struct Cartridge {
    rom: Vec<u8>,
    bank_count: usize,
    /// 32KB of cart RAM (two 16KB banks)
    cart_ram: Vec<u8>,
    /// Mapper slot registers: which ROM bank each slot maps to
    /// slot_bank[0] -> $FFFD (slot 0, $0000-$3FFF)
    /// slot_bank[1] -> $FFFE (slot 1, $4000-$7FFF)
    /// slot_bank[2] -> $FFFF (slot 2, $8000-$BFFF)
    slot_bank: [u8; 3],
    /// Control register at $FFFC
    control: u8,
}

const BANK_SIZE: usize = 16 * 1024; // 16KB per bank
const CART_RAM_SIZE: usize = 32 * 1024; // 32KB cart RAM (two 16KB banks)
const CART_RAM_BANK_SIZE: usize = 16 * 1024; // 16KB per cart RAM bank

impl Cartridge {
    /// Create a new cartridge from raw ROM data.
    ///
    /// The ROM is padded to at least one full 16KB bank if necessary.
    /// Bank count is determined by dividing ROM size by 16KB (rounded up).
    pub fn new(rom: Vec<u8>) -> Self {
        let len = rom.len().max(BANK_SIZE);
        let bank_count = (len + BANK_SIZE - 1) / BANK_SIZE;

        let mut padded_rom = rom;
        padded_rom.resize(bank_count * BANK_SIZE, 0);

        Self {
            rom: padded_rom,
            bank_count,
            cart_ram: vec![0u8; CART_RAM_SIZE],
            slot_bank: [0, 1, 2],
            control: 0,
        }
    }

    /// Return the number of 16KB ROM banks.
    pub fn rom_bank_count(&self) -> usize {
        self.bank_count
    }

    /// Read a byte from the cartridge address space ($0000-$BFFF).
    ///
    /// Handles the Sega mapper bank switching, the first-1KB fixed mapping,
    /// and cart RAM at $8000-$BFFF when enabled.
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            // Slot 0: $0000-$3FFF
            0x0000..=0x3FFF => {
                if addr <= 0x03FF {
                    // First 1KB always maps to first 1KB of ROM bank 0
                    self.rom[addr as usize]
                } else {
                    let bank = self.effective_bank(self.slot_bank[0]);
                    let offset = bank * BANK_SIZE + (addr as usize);
                    self.rom[offset]
                }
            }
            // Slot 1: $4000-$7FFF
            0x4000..=0x7FFF => {
                let bank = self.effective_bank(self.slot_bank[1]);
                let offset = bank * BANK_SIZE + (addr as usize - 0x4000);
                self.rom[offset]
            }
            // Slot 2: $8000-$BFFF
            0x8000..=0xBFFF => {
                if self.cart_ram_enabled() {
                    let ram_bank_offset = self.cart_ram_bank_offset();
                    let offset = ram_bank_offset + (addr as usize - 0x8000);
                    self.cart_ram[offset]
                } else {
                    let bank = self.effective_bank(self.slot_bank[2]);
                    let offset = bank * BANK_SIZE + (addr as usize - 0x8000);
                    self.rom[offset]
                }
            }
            _ => 0xFF,
        }
    }

    /// Write to a mapper register ($FFFC-$FFFF).
    pub fn write_mapper(&mut self, addr: u16, val: u8) {
        match addr {
            0xFFFC => self.control = val,
            0xFFFD => self.slot_bank[0] = val,
            0xFFFE => self.slot_bank[1] = val,
            0xFFFF => self.slot_bank[2] = val,
            _ => {}
        }
    }

    /// Write to cart RAM at $8000-$BFFF (only effective when cart RAM is enabled).
    pub fn write_cart_ram(&mut self, addr: u16, val: u8) {
        if self.cart_ram_enabled() && (0x8000..=0xBFFF).contains(&addr) {
            let ram_bank_offset = self.cart_ram_bank_offset();
            let offset = ram_bank_offset + (addr as usize - 0x8000);
            self.cart_ram[offset] = val;
        }
    }

    /// Check if cart RAM is enabled (bit 3 of control register).
    fn cart_ram_enabled(&self) -> bool {
        self.control & 0x08 != 0
    }

    /// Get the byte offset into cart_ram for the currently selected RAM bank.
    /// Bit 2 of the control register selects which 16KB bank.
    fn cart_ram_bank_offset(&self) -> usize {
        if self.control & 0x04 != 0 {
            CART_RAM_BANK_SIZE // second 16KB bank
        } else {
            0 // first 16KB bank
        }
    }

    /// Wrap a bank number to the actual number of ROM banks.
    fn effective_bank(&self, bank: u8) -> usize {
        (bank as usize) % self.bank_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helper: build a ROM with `n` banks, where each bank is filled with a
    // distinct byte value (bank 0 -> 0x00, bank 1 -> 0x01, ...).
    // The very first byte of each bank is set to `0xA0 + bank_index` so we
    // can distinguish "first byte of bank" from "fill byte of bank".
    // -----------------------------------------------------------------------
    fn make_rom(banks: usize) -> Vec<u8> {
        let mut rom = vec![0u8; banks * BANK_SIZE];
        for b in 0..banks {
            let fill = b as u8;
            for i in 0..BANK_SIZE {
                rom[b * BANK_SIZE + i] = fill;
            }
            // tag the first byte specially
            rom[b * BANK_SIZE] = 0xA0 + b as u8;
        }
        rom
    }

    // -----------------------------------------------------------------------
    // ROM loading
    // -----------------------------------------------------------------------
    #[test]
    fn loading_empty_rom_gives_one_bank() {
        let cart = Cartridge::new(vec![]);
        assert_eq!(cart.rom_bank_count(), 1);
        assert_eq!(cart.rom.len(), BANK_SIZE);
    }

    #[test]
    fn loading_small_rom_pads_to_one_bank() {
        let cart = Cartridge::new(vec![0x42; 1024]);
        assert_eq!(cart.rom_bank_count(), 1);
        assert_eq!(cart.rom.len(), BANK_SIZE);
        // first 1KB should be 0x42, rest should be 0x00
        assert_eq!(cart.rom[0], 0x42);
        assert_eq!(cart.rom[1023], 0x42);
        assert_eq!(cart.rom[1024], 0x00);
    }

    #[test]
    fn loading_exact_bank_boundary() {
        let cart = Cartridge::new(vec![0xAB; BANK_SIZE * 4]);
        assert_eq!(cart.rom_bank_count(), 4);
        assert_eq!(cart.rom.len(), BANK_SIZE * 4);
    }

    #[test]
    fn loading_not_on_boundary_rounds_up() {
        let cart = Cartridge::new(vec![0xCD; BANK_SIZE * 2 + 100]);
        assert_eq!(cart.rom_bank_count(), 3);
        assert_eq!(cart.rom.len(), BANK_SIZE * 3);
    }

    // -----------------------------------------------------------------------
    // Default bank mapping
    // -----------------------------------------------------------------------
    #[test]
    fn default_slots_are_0_1_2() {
        let rom = make_rom(4);
        let cart = Cartridge::new(rom);

        // Slot 0 ($0000): should be bank 0 data
        // First byte of bank 0 is 0xA0
        assert_eq!(cart.read(0x0000), 0xA0);

        // Slot 1 ($4000): should be bank 1 data
        // First byte of bank 1 is 0xA1
        assert_eq!(cart.read(0x4000), 0xA1);

        // Slot 2 ($8000): should be bank 2 data
        // First byte of bank 2 is 0xA2
        assert_eq!(cart.read(0x8000), 0xA2);
    }

    #[test]
    fn default_slot_reads_interior_data() {
        let rom = make_rom(4);
        let cart = Cartridge::new(rom);

        // Address $0500 in slot 0 -> bank 0, offset $0500 -> fill byte 0x00
        assert_eq!(cart.read(0x0500), 0x00);
        // Address $4001 in slot 1 -> bank 1, offset $0001 -> fill byte 0x01
        assert_eq!(cart.read(0x4001), 0x01);
        // Address $8001 in slot 2 -> bank 2, offset $0001 -> fill byte 0x02
        assert_eq!(cart.read(0x8001), 0x02);
    }

    // -----------------------------------------------------------------------
    // Bank switching
    // -----------------------------------------------------------------------
    #[test]
    fn switch_slot1_to_bank3() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Switch slot 1 to bank 3
        cart.write_mapper(0xFFFE, 3);
        assert_eq!(cart.read(0x4000), 0xA3); // first byte of bank 3
        assert_eq!(cart.read(0x4001), 0x03); // fill byte of bank 3
    }

    #[test]
    fn switch_slot2_to_bank0() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Switch slot 2 to bank 0
        cart.write_mapper(0xFFFF, 0);
        assert_eq!(cart.read(0x8000), 0xA0);
        assert_eq!(cart.read(0x8001), 0x00);
    }

    #[test]
    fn switch_slot0_to_bank3() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        cart.write_mapper(0xFFFD, 3);
        // Address $0400 and above should now read bank 3
        assert_eq!(cart.read(0x0400), 0x03); // fill byte of bank 3
        // But first 1KB still reads bank 0 (tested separately below)
    }

    #[test]
    fn switch_all_slots() {
        let rom = make_rom(8);
        let mut cart = Cartridge::new(rom);

        cart.write_mapper(0xFFFD, 5);
        cart.write_mapper(0xFFFE, 6);
        cart.write_mapper(0xFFFF, 7);

        // Slot 0 past first 1KB -> bank 5
        assert_eq!(cart.read(0x0400), 0x05);
        // Slot 1 -> bank 6
        assert_eq!(cart.read(0x4000), 0xA6);
        // Slot 2 -> bank 7
        assert_eq!(cart.read(0x8000), 0xA7);
    }

    // -----------------------------------------------------------------------
    // Bank wrapping (modulo bank count)
    // -----------------------------------------------------------------------
    #[test]
    fn bank_wrapping_slot1() {
        let rom = make_rom(4); // banks 0-3
        let mut cart = Cartridge::new(rom);

        // Set slot 1 to bank 4 -> wraps to bank 0
        cart.write_mapper(0xFFFE, 4);
        assert_eq!(cart.read(0x4000), 0xA0);
    }

    #[test]
    fn bank_wrapping_slot2() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Set slot 2 to bank 7 -> 7 % 4 = 3
        cart.write_mapper(0xFFFF, 7);
        assert_eq!(cart.read(0x8000), 0xA3);
    }

    #[test]
    fn bank_wrapping_slot0() {
        let rom = make_rom(2); // banks 0-1
        let mut cart = Cartridge::new(rom);

        // Set slot 0 to bank 5 -> 5 % 2 = 1
        cart.write_mapper(0xFFFD, 5);
        assert_eq!(cart.read(0x0400), 0x01); // fill byte of bank 1
    }

    #[test]
    fn bank_wrapping_large_value() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // bank 255 -> 255 % 4 = 3
        cart.write_mapper(0xFFFE, 255);
        assert_eq!(cart.read(0x4000), 0xA3);
    }

    // -----------------------------------------------------------------------
    // Slot 0 first 1KB always maps to ROM bank 0
    // -----------------------------------------------------------------------
    #[test]
    fn first_1kb_always_bank0_regardless_of_slot0_register() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Default: slot 0 = bank 0, reads from bank 0
        assert_eq!(cart.read(0x0000), 0xA0);
        assert_eq!(cart.read(0x0001), 0x00); // fill byte of bank 0
        assert_eq!(cart.read(0x03FF), 0x00); // last byte of first 1KB, fill of bank 0

        // Switch slot 0 to bank 3
        cart.write_mapper(0xFFFD, 3);

        // First 1KB ($0000-$03FF) should STILL read bank 0
        assert_eq!(cart.read(0x0000), 0xA0);
        assert_eq!(cart.read(0x0001), 0x00);
        assert_eq!(cart.read(0x03FF), 0x00);

        // But $0400 onwards reads bank 3
        assert_eq!(cart.read(0x0400), 0x03);
    }

    #[test]
    fn first_1kb_boundary_exact() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        cart.write_mapper(0xFFFD, 2);

        // $03FF is the last byte of the fixed region -> bank 0
        assert_eq!(cart.read(0x03FF), 0x00); // fill byte of bank 0
        // $0400 is the first byte of the switchable region -> bank 2
        assert_eq!(cart.read(0x0400), 0x02); // fill byte of bank 2
    }

    // -----------------------------------------------------------------------
    // Cart RAM enable/disable
    // -----------------------------------------------------------------------
    #[test]
    fn cart_ram_disabled_by_default() {
        let rom = make_rom(4);
        let cart = Cartridge::new(rom);

        // Slot 2 should read ROM (bank 2)
        assert_eq!(cart.read(0x8000), 0xA2);
    }

    #[test]
    fn cart_ram_enable_via_control_register() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Enable cart RAM: bit 3 = 1 -> control = 0x08
        cart.write_mapper(0xFFFC, 0x08);

        // Now $8000-$BFFF should read cart RAM (initially zero)
        assert_eq!(cart.read(0x8000), 0x00);
        assert_eq!(cart.read(0xBFFF), 0x00);
    }

    #[test]
    fn cart_ram_disable_restores_rom_reads() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Enable cart RAM
        cart.write_mapper(0xFFFC, 0x08);
        assert_eq!(cart.read(0x8000), 0x00); // RAM

        // Disable cart RAM
        cart.write_mapper(0xFFFC, 0x00);
        assert_eq!(cart.read(0x8000), 0xA2); // ROM bank 2 again
    }

    // -----------------------------------------------------------------------
    // Cart RAM read/write
    // -----------------------------------------------------------------------
    #[test]
    fn cart_ram_write_and_read_back() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Enable cart RAM
        cart.write_mapper(0xFFFC, 0x08);

        // Write to cart RAM
        cart.write_cart_ram(0x8000, 0x42);
        cart.write_cart_ram(0x8001, 0xAB);
        cart.write_cart_ram(0xBFFF, 0xFF);

        // Read back
        assert_eq!(cart.read(0x8000), 0x42);
        assert_eq!(cart.read(0x8001), 0xAB);
        assert_eq!(cart.read(0xBFFF), 0xFF);
    }

    #[test]
    fn cart_ram_write_ignored_when_disabled() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Cart RAM is disabled by default
        cart.write_cart_ram(0x8000, 0x42);

        // Enable cart RAM and verify the write was not stored
        cart.write_mapper(0xFFFC, 0x08);
        assert_eq!(cart.read(0x8000), 0x00); // should be zero, write was ignored
    }

    #[test]
    fn cart_ram_persists_across_enable_disable() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Enable cart RAM and write
        cart.write_mapper(0xFFFC, 0x08);
        cart.write_cart_ram(0x8000, 0x42);
        assert_eq!(cart.read(0x8000), 0x42);

        // Disable cart RAM (reads ROM now)
        cart.write_mapper(0xFFFC, 0x00);
        assert_eq!(cart.read(0x8000), 0xA2); // ROM

        // Re-enable -> data persists
        cart.write_mapper(0xFFFC, 0x08);
        assert_eq!(cart.read(0x8000), 0x42);
    }

    // -----------------------------------------------------------------------
    // Cart RAM bank select (bit 2 of control register)
    // -----------------------------------------------------------------------
    #[test]
    fn cart_ram_bank_select() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Enable cart RAM, bank 0 (bits: 0000_1000)
        cart.write_mapper(0xFFFC, 0x08);
        cart.write_cart_ram(0x8000, 0xAA);
        assert_eq!(cart.read(0x8000), 0xAA);

        // Switch to cart RAM bank 1 (bits: 0000_1100)
        cart.write_mapper(0xFFFC, 0x0C);
        // Bank 1 should be untouched (zero)
        assert_eq!(cart.read(0x8000), 0x00);

        // Write to bank 1
        cart.write_cart_ram(0x8000, 0xBB);
        assert_eq!(cart.read(0x8000), 0xBB);

        // Switch back to bank 0
        cart.write_mapper(0xFFFC, 0x08);
        assert_eq!(cart.read(0x8000), 0xAA); // bank 0 data still there
    }

    #[test]
    fn cart_ram_bank1_full_range() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Enable cart RAM bank 1
        cart.write_mapper(0xFFFC, 0x0C);

        // Write at start and end of the 16KB range
        cart.write_cart_ram(0x8000, 0x11);
        cart.write_cart_ram(0xBFFF, 0x22);

        assert_eq!(cart.read(0x8000), 0x11);
        assert_eq!(cart.read(0xBFFF), 0x22);

        // Switch to bank 0 - these addresses should be zero
        cart.write_mapper(0xFFFC, 0x08);
        assert_eq!(cart.read(0x8000), 0x00);
        assert_eq!(cart.read(0xBFFF), 0x00);
    }

    // -----------------------------------------------------------------------
    // Control register bits
    // -----------------------------------------------------------------------
    #[test]
    fn control_register_stored() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        cart.write_mapper(0xFFFC, 0x88); // bit 7 (ROM write enable) + bit 3 (cart RAM)
        assert_eq!(cart.control, 0x88);
    }

    #[test]
    fn control_register_default_is_zero() {
        let cart = Cartridge::new(make_rom(2));
        assert_eq!(cart.control, 0);
    }

    #[test]
    fn rom_write_enable_bit_does_not_affect_reads() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        // Set ROM write enable (bit 7) without cart RAM enable
        cart.write_mapper(0xFFFC, 0x80);
        // Reads should still work normally from ROM
        assert_eq!(cart.read(0x8000), 0xA2);
    }

    // -----------------------------------------------------------------------
    // Out-of-range address reads
    // -----------------------------------------------------------------------
    #[test]
    fn read_above_bfff_returns_0xff() {
        let cart = Cartridge::new(make_rom(4));
        assert_eq!(cart.read(0xC000), 0xFF);
        assert_eq!(cart.read(0xFFFF), 0xFF);
    }

    // -----------------------------------------------------------------------
    // Mapper writes to non-register addresses are ignored
    // -----------------------------------------------------------------------
    #[test]
    fn mapper_write_to_non_register_ignored() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        cart.write_mapper(0xFFFB, 5); // not a mapper register
        // Slots should be unchanged
        assert_eq!(cart.read(0x0000), 0xA0);
        assert_eq!(cart.read(0x4000), 0xA1);
        assert_eq!(cart.read(0x8000), 0xA2);
    }

    // -----------------------------------------------------------------------
    // Cart RAM write out of range ($8000-$BFFF) is ignored
    // -----------------------------------------------------------------------
    #[test]
    fn cart_ram_write_outside_range_ignored() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        cart.write_mapper(0xFFFC, 0x08); // enable cart RAM
        cart.write_cart_ram(0x7FFF, 0x42); // below range
        cart.write_cart_ram(0xC000, 0x42); // above range

        // These should not have corrupted anything
        assert_eq!(cart.read(0x8000), 0x00);
    }

    // -----------------------------------------------------------------------
    // Single-bank ROM with wrapping
    // -----------------------------------------------------------------------
    #[test]
    fn single_bank_rom_all_slots_see_bank0() {
        let rom = make_rom(1);
        let cart = Cartridge::new(rom);

        // Default slots 0,1,2 but only 1 bank exists
        // slot 1 = bank 1 % 1 = bank 0
        assert_eq!(cart.read(0x4000), 0xA0);
        // slot 2 = bank 2 % 1 = bank 0
        assert_eq!(cart.read(0x8000), 0xA0);
    }

    // -----------------------------------------------------------------------
    // Slot boundaries
    // -----------------------------------------------------------------------
    #[test]
    fn slot_boundary_exact() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        cart.write_mapper(0xFFFD, 1);
        cart.write_mapper(0xFFFE, 2);
        cart.write_mapper(0xFFFF, 3);

        // Last byte of slot 0 ($3FFF) -> bank 1
        assert_eq!(cart.read(0x3FFF), 0x01);
        // First byte of slot 1 ($4000) -> bank 2
        assert_eq!(cart.read(0x4000), 0xA2);
        // Last byte of slot 1 ($7FFF) -> bank 2
        assert_eq!(cart.read(0x7FFF), 0x02);
        // First byte of slot 2 ($8000) -> bank 3
        assert_eq!(cart.read(0x8000), 0xA3);
        // Last byte of slot 2 ($BFFF) -> bank 3
        assert_eq!(cart.read(0xBFFF), 0x03);
    }

    // -----------------------------------------------------------------------
    // Cart RAM with slot 2 bank switching interaction
    // -----------------------------------------------------------------------
    #[test]
    fn cart_ram_overrides_slot2_bank_selection() {
        let rom = make_rom(8);
        let mut cart = Cartridge::new(rom);

        // Set slot 2 to bank 7
        cart.write_mapper(0xFFFF, 7);
        assert_eq!(cart.read(0x8000), 0xA7);

        // Enable cart RAM - should override the ROM bank selection
        cart.write_mapper(0xFFFC, 0x08);
        assert_eq!(cart.read(0x8000), 0x00); // cart RAM, not ROM

        // Write to cart RAM
        cart.write_cart_ram(0x8000, 0xDD);
        assert_eq!(cart.read(0x8000), 0xDD);

        // Disable cart RAM - slot 2 should still point to bank 7
        cart.write_mapper(0xFFFC, 0x00);
        assert_eq!(cart.read(0x8000), 0xA7);
    }

    // -----------------------------------------------------------------------
    // Ensure slot 0/1 are unaffected by cart RAM enable
    // -----------------------------------------------------------------------
    #[test]
    fn cart_ram_does_not_affect_slots_0_and_1() {
        let rom = make_rom(4);
        let mut cart = Cartridge::new(rom);

        cart.write_mapper(0xFFFC, 0x08); // enable cart RAM

        // Slot 0 and 1 should still read ROM normally
        assert_eq!(cart.read(0x0000), 0xA0);
        assert_eq!(cart.read(0x4000), 0xA1);
    }

    // -----------------------------------------------------------------------
    // Rapid bank switching
    // -----------------------------------------------------------------------
    #[test]
    fn rapid_bank_switching() {
        let rom = make_rom(8);
        let mut cart = Cartridge::new(rom);

        for bank in 0..8u8 {
            cart.write_mapper(0xFFFE, bank);
            assert_eq!(cart.read(0x4000), 0xA0 + bank);
        }
    }
}
