// Z80 CPU emulation
pub mod instructions;

use crate::bus::Bus;

// Flag bit masks
pub const FLAG_C: u8 = 0x01;  // Carry
pub const FLAG_N: u8 = 0x02;  // Add/Subtract
pub const FLAG_PV: u8 = 0x04; // Parity/Overflow
pub const FLAG_X: u8 = 0x08;  // Undocumented (bit 3)
pub const FLAG_H: u8 = 0x10;  // Half Carry
pub const FLAG_Y: u8 = 0x20;  // Undocumented (bit 5)
pub const FLAG_Z: u8 = 0x40;  // Zero
pub const FLAG_S: u8 = 0x80;  // Sign

pub struct Cpu {
    // Main registers
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,

    // Shadow registers
    pub a_prime: u8,
    pub f_prime: u8,
    pub b_prime: u8,
    pub c_prime: u8,
    pub d_prime: u8,
    pub e_prime: u8,
    pub h_prime: u8,
    pub l_prime: u8,

    // Index registers
    pub ix: u16,
    pub iy: u16,

    // Special registers
    pub sp: u16,
    pub pc: u16,
    pub i: u8,
    pub r: u8,

    // Internal state
    pub iff1: bool,
    pub iff2: bool,
    pub im: u8,
    pub halted: bool,
    pub ei_pending: bool,
    pub wz: u16,

    /// When true, print a trace line after each instruction for debugging.
    /// Enable at runtime by setting the `RUSTYGEAR_TRACE=1` environment variable.
    pub trace: bool,
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            a: 0, f: 0, b: 0, c: 0, d: 0, e: 0, h: 0, l: 0,
            a_prime: 0, f_prime: 0, b_prime: 0, c_prime: 0,
            d_prime: 0, e_prime: 0, h_prime: 0, l_prime: 0,
            ix: 0, iy: 0,
            sp: 0xDFFF, pc: 0,
            i: 0, r: 0,
            iff1: false, iff2: false, im: 0,
            halted: false, ei_pending: false, wz: 0,
            trace: false,
        }
    }

    // --- 16-bit register pair getters ---

    pub fn af(&self) -> u16 {
        ((self.a as u16) << 8) | (self.f as u16)
    }

    pub fn bc(&self) -> u16 {
        ((self.b as u16) << 8) | (self.c as u16)
    }

    pub fn de(&self) -> u16 {
        ((self.d as u16) << 8) | (self.e as u16)
    }

    pub fn hl(&self) -> u16 {
        ((self.h as u16) << 8) | (self.l as u16)
    }

    // --- 16-bit register pair setters ---

    pub fn set_af(&mut self, val: u16) {
        self.a = (val >> 8) as u8;
        self.f = val as u8;
    }

    pub fn set_bc(&mut self, val: u16) {
        self.b = (val >> 8) as u8;
        self.c = val as u8;
    }

    pub fn set_de(&mut self, val: u16) {
        self.d = (val >> 8) as u8;
        self.e = val as u8;
    }

    pub fn set_hl(&mut self, val: u16) {
        self.h = (val >> 8) as u8;
        self.l = val as u8;
    }

    // --- Flag operations ---

    /// Returns true if the flag bit(s) specified by `mask` are set in F.
    pub fn flag(&self, mask: u8) -> bool {
        (self.f & mask) != 0
    }

    /// Sets or clears the flag bit(s) specified by `mask` in F.
    pub fn set_flag(&mut self, mask: u8, val: bool) {
        if val {
            self.f |= mask;
        } else {
            self.f &= !mask;
        }
    }

    // --- Exchange operations ---

    /// EX AF, AF' - swap AF with shadow AF.
    pub fn ex_af_af_prime(&mut self) {
        std::mem::swap(&mut self.a, &mut self.a_prime);
        std::mem::swap(&mut self.f, &mut self.f_prime);
    }

    /// EXX - swap BC, DE, HL with their shadow counterparts.
    pub fn exx(&mut self) {
        std::mem::swap(&mut self.b, &mut self.b_prime);
        std::mem::swap(&mut self.c, &mut self.c_prime);
        std::mem::swap(&mut self.d, &mut self.d_prime);
        std::mem::swap(&mut self.e, &mut self.e_prime);
        std::mem::swap(&mut self.h, &mut self.h_prime);
        std::mem::swap(&mut self.l, &mut self.l_prime);
    }

    /// Increment the lower 7 bits of R, preserving bit 7.
    pub fn increment_r(&mut self) {
        self.r = (self.r & 0x80) | ((self.r.wrapping_add(1)) & 0x7F);
    }

    /// Fetch a byte from memory at PC and advance PC.
    pub fn fetch(&mut self, bus: &Bus) -> u8 {
        let val = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        val
    }

    /// Fetch a 16-bit word (little-endian) from memory at PC and advance PC by 2.
    pub fn fetch_word(&mut self, bus: &Bus) -> u16 {
        let lo = self.fetch(bus) as u16;
        let hi = self.fetch(bus) as u16;
        hi << 8 | lo
    }

    /// Execute one instruction and return the number of T-cycles consumed.
    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        if self.halted {
            self.increment_r();
            return 4;
        }

        // Handle EI delayed effect
        let ei_was_pending = self.ei_pending;
        self.ei_pending = false;

        // Capture PC before fetch for trace output
        let trace_pc = self.pc;

        self.increment_r();
        let opcode = self.fetch(bus);
        let cycles = self.execute(bus, opcode);

        if self.trace {
            eprintln!(
                "PC:{:04X} OP:{:02X} AF:{:04X} BC:{:04X} DE:{:04X} HL:{:04X} SP:{:04X} IX:{:04X} IY:{:04X}",
                trace_pc, opcode, self.af(), self.bc(), self.de(), self.hl(), self.sp, self.ix, self.iy
            );
        }

        if ei_was_pending {
            self.iff1 = true;
            self.iff2 = true;
        }

        cycles
    }

    fn execute(&mut self, bus: &mut Bus, opcode: u8) -> u32 {
        instructions::execute_unprefixed(self, bus, opcode)
    }

    /// Check for and handle pending maskable interrupts.
    /// Returns the number of cycles consumed (0 if no interrupt handled).
    pub fn handle_interrupts(&mut self, bus: &mut Bus) -> u32 {
        if bus.irq_pending && self.iff1 {
            self.halted = false;
            self.iff1 = false;
            self.iff2 = false;
            bus.irq_pending = false;
            self.increment_r();

            match self.im {
                0 | 1 => {
                    // IM 0 and IM 1: Push PC, jump to $0038
                    self.sp = self.sp.wrapping_sub(1);
                    bus.write(self.sp, (self.pc >> 8) as u8);
                    self.sp = self.sp.wrapping_sub(1);
                    bus.write(self.sp, self.pc as u8);
                    self.pc = 0x0038;
                    13
                }
                2 => {
                    // IM 2: Push PC, read vector from (I * 256 + data_bus_value)
                    // Game Gear puts $FF on the data bus during interrupt acknowledge
                    self.sp = self.sp.wrapping_sub(1);
                    bus.write(self.sp, (self.pc >> 8) as u8);
                    self.sp = self.sp.wrapping_sub(1);
                    bus.write(self.sp, self.pc as u8);
                    let vector_addr = ((self.i as u16) << 8) | 0xFF;
                    let lo = bus.read(vector_addr) as u16;
                    let hi = bus.read(vector_addr.wrapping_add(1)) as u16;
                    self.pc = (hi << 8) | lo;
                    19
                }
                _ => 0,
            }
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    use crate::cartridge::Cartridge;

    // -------------------------------------------------------
    // Construction / defaults
    // -------------------------------------------------------

    #[test]
    fn new_cpu_has_zeroed_main_registers() {
        let cpu = Cpu::new();
        assert_eq!(cpu.a, 0);
        assert_eq!(cpu.f, 0);
        assert_eq!(cpu.b, 0);
        assert_eq!(cpu.c, 0);
        assert_eq!(cpu.d, 0);
        assert_eq!(cpu.e, 0);
        assert_eq!(cpu.h, 0);
        assert_eq!(cpu.l, 0);
    }

    #[test]
    fn new_cpu_has_zeroed_shadow_registers() {
        let cpu = Cpu::new();
        assert_eq!(cpu.a_prime, 0);
        assert_eq!(cpu.f_prime, 0);
        assert_eq!(cpu.b_prime, 0);
        assert_eq!(cpu.c_prime, 0);
        assert_eq!(cpu.d_prime, 0);
        assert_eq!(cpu.e_prime, 0);
        assert_eq!(cpu.h_prime, 0);
        assert_eq!(cpu.l_prime, 0);
    }

    #[test]
    fn new_cpu_has_zeroed_index_registers() {
        let cpu = Cpu::new();
        assert_eq!(cpu.ix, 0);
        assert_eq!(cpu.iy, 0);
    }

    #[test]
    fn new_cpu_sp_is_0xdfff() {
        let cpu = Cpu::new();
        assert_eq!(cpu.sp, 0xDFFF);
    }

    #[test]
    fn new_cpu_pc_is_zero() {
        let cpu = Cpu::new();
        assert_eq!(cpu.pc, 0);
    }

    #[test]
    fn new_cpu_special_registers_zeroed() {
        let cpu = Cpu::new();
        assert_eq!(cpu.i, 0);
        assert_eq!(cpu.r, 0);
    }

    #[test]
    fn new_cpu_internal_state_defaults() {
        let cpu = Cpu::new();
        assert!(!cpu.iff1);
        assert!(!cpu.iff2);
        assert_eq!(cpu.im, 0);
        assert!(!cpu.halted);
        assert!(!cpu.ei_pending);
        assert_eq!(cpu.wz, 0);
    }

    // -------------------------------------------------------
    // Flag constants
    // -------------------------------------------------------

    #[test]
    fn flag_constants_are_correct() {
        assert_eq!(FLAG_C, 0x01);
        assert_eq!(FLAG_N, 0x02);
        assert_eq!(FLAG_PV, 0x04);
        assert_eq!(FLAG_X, 0x08);
        assert_eq!(FLAG_H, 0x10);
        assert_eq!(FLAG_Y, 0x20);
        assert_eq!(FLAG_Z, 0x40);
        assert_eq!(FLAG_S, 0x80);
    }

    #[test]
    fn flag_constants_are_distinct_single_bits() {
        let all = [FLAG_C, FLAG_N, FLAG_PV, FLAG_X, FLAG_H, FLAG_Y, FLAG_Z, FLAG_S];
        for &f in &all {
            assert_eq!(f.count_ones(), 1, "flag 0x{:02X} is not a single bit", f);
        }
        // All eight bits of a u8 are covered
        let combined: u8 = all.iter().fold(0u8, |acc, &f| acc | f);
        assert_eq!(combined, 0xFF);
    }

    // -------------------------------------------------------
    // 16-bit pair getters
    // -------------------------------------------------------

    #[test]
    fn af_getter_combines_a_and_f() {
        let mut cpu = Cpu::new();
        cpu.a = 0x12;
        cpu.f = 0x34;
        assert_eq!(cpu.af(), 0x1234);
    }

    #[test]
    fn bc_getter_combines_b_and_c() {
        let mut cpu = Cpu::new();
        cpu.b = 0xAB;
        cpu.c = 0xCD;
        assert_eq!(cpu.bc(), 0xABCD);
    }

    #[test]
    fn de_getter_combines_d_and_e() {
        let mut cpu = Cpu::new();
        cpu.d = 0x56;
        cpu.e = 0x78;
        assert_eq!(cpu.de(), 0x5678);
    }

    #[test]
    fn hl_getter_combines_h_and_l() {
        let mut cpu = Cpu::new();
        cpu.h = 0x9A;
        cpu.l = 0xBC;
        assert_eq!(cpu.hl(), 0x9ABC);
    }

    #[test]
    fn pair_getters_on_fresh_cpu_are_zero() {
        let cpu = Cpu::new();
        assert_eq!(cpu.af(), 0);
        assert_eq!(cpu.bc(), 0);
        assert_eq!(cpu.de(), 0);
        assert_eq!(cpu.hl(), 0);
    }

    // -------------------------------------------------------
    // 16-bit pair setters
    // -------------------------------------------------------

    #[test]
    fn set_af_splits_into_a_and_f() {
        let mut cpu = Cpu::new();
        cpu.set_af(0xBEEF);
        assert_eq!(cpu.a, 0xBE);
        assert_eq!(cpu.f, 0xEF);
        assert_eq!(cpu.af(), 0xBEEF);
    }

    #[test]
    fn set_bc_splits_into_b_and_c() {
        let mut cpu = Cpu::new();
        cpu.set_bc(0xCAFE);
        assert_eq!(cpu.b, 0xCA);
        assert_eq!(cpu.c, 0xFE);
        assert_eq!(cpu.bc(), 0xCAFE);
    }

    #[test]
    fn set_de_splits_into_d_and_e() {
        let mut cpu = Cpu::new();
        cpu.set_de(0xDEAD);
        assert_eq!(cpu.d, 0xDE);
        assert_eq!(cpu.e, 0xAD);
        assert_eq!(cpu.de(), 0xDEAD);
    }

    #[test]
    fn set_hl_splits_into_h_and_l() {
        let mut cpu = Cpu::new();
        cpu.set_hl(0xF00D);
        assert_eq!(cpu.h, 0xF0);
        assert_eq!(cpu.l, 0x0D);
        assert_eq!(cpu.hl(), 0xF00D);
    }

    #[test]
    fn setter_then_getter_roundtrip() {
        let mut cpu = Cpu::new();
        for val in [0x0000u16, 0xFFFF, 0x00FF, 0xFF00, 0x1234] {
            cpu.set_af(val);
            assert_eq!(cpu.af(), val);
            cpu.set_bc(val);
            assert_eq!(cpu.bc(), val);
            cpu.set_de(val);
            assert_eq!(cpu.de(), val);
            cpu.set_hl(val);
            assert_eq!(cpu.hl(), val);
        }
    }

    // -------------------------------------------------------
    // Flag operations
    // -------------------------------------------------------

    #[test]
    fn flag_returns_false_when_not_set() {
        let cpu = Cpu::new();
        assert!(!cpu.flag(FLAG_C));
        assert!(!cpu.flag(FLAG_Z));
        assert!(!cpu.flag(FLAG_S));
    }

    #[test]
    fn set_flag_true_sets_bit() {
        let mut cpu = Cpu::new();
        cpu.set_flag(FLAG_Z, true);
        assert!(cpu.flag(FLAG_Z));
        assert_eq!(cpu.f & FLAG_Z, FLAG_Z);
    }

    #[test]
    fn set_flag_false_clears_bit() {
        let mut cpu = Cpu::new();
        cpu.f = 0xFF;
        cpu.set_flag(FLAG_Z, false);
        assert!(!cpu.flag(FLAG_Z));
        // Other bits untouched
        assert!(cpu.flag(FLAG_C));
        assert!(cpu.flag(FLAG_S));
    }

    #[test]
    fn set_flag_does_not_affect_other_bits() {
        let mut cpu = Cpu::new();
        cpu.f = 0x00;
        cpu.set_flag(FLAG_C, true);
        assert_eq!(cpu.f, FLAG_C);
        cpu.set_flag(FLAG_Z, true);
        assert_eq!(cpu.f, FLAG_C | FLAG_Z);
        cpu.set_flag(FLAG_C, false);
        assert_eq!(cpu.f, FLAG_Z);
    }

    #[test]
    fn flag_with_all_flags_set() {
        let mut cpu = Cpu::new();
        cpu.f = 0xFF;
        assert!(cpu.flag(FLAG_C));
        assert!(cpu.flag(FLAG_N));
        assert!(cpu.flag(FLAG_PV));
        assert!(cpu.flag(FLAG_X));
        assert!(cpu.flag(FLAG_H));
        assert!(cpu.flag(FLAG_Y));
        assert!(cpu.flag(FLAG_Z));
        assert!(cpu.flag(FLAG_S));
    }

    // -------------------------------------------------------
    // EX AF, AF'
    // -------------------------------------------------------

    #[test]
    fn ex_af_af_prime_swaps_a_and_f() {
        let mut cpu = Cpu::new();
        cpu.a = 0x11;
        cpu.f = 0x22;
        cpu.a_prime = 0x33;
        cpu.f_prime = 0x44;

        cpu.ex_af_af_prime();

        assert_eq!(cpu.a, 0x33);
        assert_eq!(cpu.f, 0x44);
        assert_eq!(cpu.a_prime, 0x11);
        assert_eq!(cpu.f_prime, 0x22);
    }

    #[test]
    fn ex_af_af_prime_double_swap_restores() {
        let mut cpu = Cpu::new();
        cpu.a = 0xAA;
        cpu.f = 0xBB;
        cpu.a_prime = 0xCC;
        cpu.f_prime = 0xDD;

        cpu.ex_af_af_prime();
        cpu.ex_af_af_prime();

        assert_eq!(cpu.a, 0xAA);
        assert_eq!(cpu.f, 0xBB);
        assert_eq!(cpu.a_prime, 0xCC);
        assert_eq!(cpu.f_prime, 0xDD);
    }

    #[test]
    fn ex_af_does_not_affect_other_registers() {
        let mut cpu = Cpu::new();
        cpu.b = 0x11;
        cpu.c = 0x22;
        cpu.b_prime = 0x33;
        cpu.c_prime = 0x44;

        cpu.ex_af_af_prime();

        assert_eq!(cpu.b, 0x11);
        assert_eq!(cpu.c, 0x22);
        assert_eq!(cpu.b_prime, 0x33);
        assert_eq!(cpu.c_prime, 0x44);
    }

    // -------------------------------------------------------
    // EXX
    // -------------------------------------------------------

    #[test]
    fn exx_swaps_bc_de_hl_with_shadows() {
        let mut cpu = Cpu::new();
        cpu.b = 0x01; cpu.c = 0x02;
        cpu.d = 0x03; cpu.e = 0x04;
        cpu.h = 0x05; cpu.l = 0x06;
        cpu.b_prime = 0xA1; cpu.c_prime = 0xA2;
        cpu.d_prime = 0xA3; cpu.e_prime = 0xA4;
        cpu.h_prime = 0xA5; cpu.l_prime = 0xA6;

        cpu.exx();

        assert_eq!(cpu.b, 0xA1); assert_eq!(cpu.c, 0xA2);
        assert_eq!(cpu.d, 0xA3); assert_eq!(cpu.e, 0xA4);
        assert_eq!(cpu.h, 0xA5); assert_eq!(cpu.l, 0xA6);
        assert_eq!(cpu.b_prime, 0x01); assert_eq!(cpu.c_prime, 0x02);
        assert_eq!(cpu.d_prime, 0x03); assert_eq!(cpu.e_prime, 0x04);
        assert_eq!(cpu.h_prime, 0x05); assert_eq!(cpu.l_prime, 0x06);
    }

    #[test]
    fn exx_double_swap_restores() {
        let mut cpu = Cpu::new();
        cpu.set_bc(0x1234);
        cpu.set_de(0x5678);
        cpu.set_hl(0x9ABC);

        cpu.exx();
        cpu.exx();

        assert_eq!(cpu.bc(), 0x1234);
        assert_eq!(cpu.de(), 0x5678);
        assert_eq!(cpu.hl(), 0x9ABC);
    }

    #[test]
    fn exx_does_not_affect_af() {
        let mut cpu = Cpu::new();
        cpu.a = 0xAA;
        cpu.f = 0xBB;
        cpu.a_prime = 0xCC;
        cpu.f_prime = 0xDD;

        cpu.exx();

        assert_eq!(cpu.a, 0xAA);
        assert_eq!(cpu.f, 0xBB);
        assert_eq!(cpu.a_prime, 0xCC);
        assert_eq!(cpu.f_prime, 0xDD);
    }

    // -------------------------------------------------------
    // increment_r
    // -------------------------------------------------------

    #[test]
    fn increment_r_from_zero() {
        let mut cpu = Cpu::new();
        cpu.r = 0x00;
        cpu.increment_r();
        assert_eq!(cpu.r, 0x01);
    }

    #[test]
    fn increment_r_wraps_lower_7_bits() {
        let mut cpu = Cpu::new();
        cpu.r = 0x7F; // lower 7 bits all set, bit 7 = 0
        cpu.increment_r();
        assert_eq!(cpu.r, 0x00); // wraps to 0, bit 7 stays 0
    }

    #[test]
    fn increment_r_preserves_bit_7_when_set() {
        let mut cpu = Cpu::new();
        cpu.r = 0x80; // bit 7 set, lower 7 = 0
        cpu.increment_r();
        assert_eq!(cpu.r, 0x81); // bit 7 preserved, lower 7 incremented
    }

    #[test]
    fn increment_r_wraps_lower_7_with_bit_7_set() {
        let mut cpu = Cpu::new();
        cpu.r = 0xFF; // bit 7 set, lower 7 all set
        cpu.increment_r();
        assert_eq!(cpu.r, 0x80); // wraps lower 7 to 0, bit 7 stays
    }

    #[test]
    fn increment_r_preserves_bit_7_when_clear() {
        let mut cpu = Cpu::new();
        cpu.r = 0x05;
        cpu.increment_r();
        assert_eq!(cpu.r, 0x06);
        // Bit 7 remains clear
        assert_eq!(cpu.r & 0x80, 0x00);
    }

    #[test]
    fn increment_r_sequential() {
        let mut cpu = Cpu::new();
        cpu.r = 0x80; // bit 7 set
        for expected_low in 1u8..=10 {
            cpu.increment_r();
            assert_eq!(cpu.r, 0x80 | expected_low);
        }
    }

    // -------------------------------------------------------
    // Fetch-decode-execute framework
    // -------------------------------------------------------

    #[test]
    fn test_nop() {
        let cart = Cartridge::new(vec![0x00; 0x8000]); // NOP
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 4);
        assert_eq!(cpu.pc, 1);
    }

    #[test]
    fn test_ld_bc_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x01; // LD BC, nn
        rom[1] = 0x34; // low byte
        rom[2] = 0x12; // high byte
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        let cycles = cpu.step(&mut bus);
        assert_eq!(cpu.bc(), 0x1234);
        assert_eq!(cycles, 10);
        assert_eq!(cpu.pc, 3);
    }

    #[test]
    fn test_ld_de_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x11; rom[1] = 0x78; rom[2] = 0x56;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!(cpu.de(), 0x5678);
    }

    #[test]
    fn test_ld_hl_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0xBC; rom[2] = 0x9A;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!(cpu.hl(), 0x9ABC);
    }

    #[test]
    fn test_ld_sp_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x31; rom[1] = 0x00; rom[2] = 0xE0;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!(cpu.sp, 0xE000);
    }

    #[test]
    fn test_ld_a_n() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; // LD A, n
        rom[1] = 0x42;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x42);
    }

    #[test]
    fn test_ld_b_n() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x06; rom[1] = 0xAB;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!(cpu.b, 0xAB);
    }

    #[test]
    fn test_halt_returns_4_cycles() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x76; // HALT
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // execute HALT
        assert!(cpu.halted);
        let cycles = cpu.step(&mut bus); // step while halted
        assert_eq!(cycles, 4);
        // PC should not advance while halted
        assert_eq!(cpu.pc, 1);
    }

    #[test]
    fn test_fetch_increments_pc() {
        let cart = Cartridge::new(vec![0xAA; 0x8000]);
        let bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.pc = 0x100;
        let val = cpu.fetch(&bus);
        assert_eq!(val, 0xAA);
        assert_eq!(cpu.pc, 0x101);
    }

    #[test]
    fn test_fetch_word_reads_little_endian() {
        let mut rom = vec![0x00; 0x8000];
        rom[0x100] = 0x34; // low byte
        rom[0x101] = 0x12; // high byte
        let cart = Cartridge::new(rom);
        let bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.pc = 0x100;
        let val = cpu.fetch_word(&bus);
        assert_eq!(val, 0x1234);
        assert_eq!(cpu.pc, 0x102);
    }

    #[test]
    fn test_step_increments_r() {
        let cart = Cartridge::new(vec![0x00; 0x8000]); // NOP
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.r = 0;
        cpu.step(&mut bus);
        assert_eq!(cpu.r & 0x7F, 1);
    }

    #[test]
    fn test_ei_delayed() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xFB; // EI
        rom[1] = 0x00; // NOP
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // EI
        // IFF should not be set yet (delayed)
        assert!(!cpu.iff1);
        assert!(cpu.ei_pending || cpu.iff1); // either pending or set
        cpu.step(&mut bus); // NOP, EI takes effect
        assert!(cpu.iff1);
        assert!(cpu.iff2);
    }

    // -------------------------------------------------------
    // Unprefixed instruction tests
    // -------------------------------------------------------

    #[test]
    fn test_add_a_r() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x10; // LD A, $10
        rom[2] = 0x06; rom[3] = 0x20; // LD B, $20
        rom[4] = 0x80;                 // ADD A, B
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x30);
        assert!(!cpu.flag(FLAG_Z));
        assert!(!cpu.flag(FLAG_C));
    }

    #[test]
    fn test_add_overflow() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0xFF;
        rom[2] = 0xC6; rom[3] = 0x01; // ADD A, 1
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.flag(FLAG_Z));
        assert!(cpu.flag(FLAG_C));
    }

    #[test]
    fn test_sub_a_b() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x30;
        rom[2] = 0x06; rom[3] = 0x10;
        rom[4] = 0x90; // SUB B
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x20);
        assert!(cpu.flag(FLAG_N));
    }

    #[test]
    fn test_cp_flags() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x42;
        rom[2] = 0xFE; rom[3] = 0x42; // CP $42
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert!(cpu.flag(FLAG_Z));
        assert!(!cpu.flag(FLAG_C));
        assert_eq!(cpu.a, 0x42); // A unchanged
    }

    #[test]
    fn test_and_a_b() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0xF0;
        rom[2] = 0x06; rom[3] = 0x0F;
        rom[4] = 0xA0; // AND B
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.flag(FLAG_Z));
        assert!(cpu.flag(FLAG_H));
    }

    #[test]
    fn test_or_a_b() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0xF0;
        rom[2] = 0x06; rom[3] = 0x0F;
        rom[4] = 0xB0; // OR B
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0xFF);
    }

    #[test]
    fn test_xor_a_a() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x42;
        rom[2] = 0xAF; // XOR A
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.flag(FLAG_Z));
    }

    #[test]
    fn test_inc_b() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x06; rom[1] = 0xFF;
        rom[2] = 0x04; // INC B
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.b, 0x00);
        assert!(cpu.flag(FLAG_Z));
        assert!(cpu.flag(FLAG_H));
    }

    #[test]
    fn test_dec_a() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x01;
        rom[2] = 0x3D; // DEC A
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.flag(FLAG_Z));
        assert!(cpu.flag(FLAG_N));
    }

    #[test]
    fn test_jr_nz() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x01;
        rom[2] = 0x3D;                 // DEC A -> 0, Z set
        rom[3] = 0x20; rom[4] = 0x02; // JR NZ, +2 (not taken)
        rom[5] = 0x3E; rom[6] = 0xFF; // LD A, $FF
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0xFF);
    }

    #[test]
    fn test_push_pop() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x01; rom[1] = 0x34; rom[2] = 0x12; // LD BC, $1234
        rom[3] = 0xC5;                                  // PUSH BC
        rom[4] = 0x01; rom[5] = 0x00; rom[6] = 0x00;  // LD BC, 0
        rom[7] = 0xC1;                                  // POP BC
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.sp = 0xDFF0;
        for _ in 0..4 { cpu.step(&mut bus); }
        assert_eq!(cpu.bc(), 0x1234);
    }

    #[test]
    fn test_call_ret() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xCD; rom[1] = 0x10; rom[2] = 0x00; // CALL $0010
        rom[0x10] = 0x3E; rom[0x11] = 0x42;           // LD A, $42
        rom[0x12] = 0xC9;                               // RET
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.sp = 0xDFF0;
        cpu.step(&mut bus); // CALL
        assert_eq!(cpu.pc, 0x0010);
        cpu.step(&mut bus); // LD A, $42
        cpu.step(&mut bus); // RET
        assert_eq!(cpu.pc, 0x0003);
        assert_eq!(cpu.a, 0x42);
    }

    #[test]
    fn test_djnz() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x06; rom[1] = 0x03;  // LD B, 3
        rom[2] = 0x3C;                   // INC A
        rom[3] = 0x10; rom[4] = 0xFD;  // DJNZ -3 (back to $0002)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        for _ in 0..7 { cpu.step(&mut bus); }
        assert_eq!(cpu.a, 3);
        assert_eq!(cpu.b, 0);
    }

    #[test]
    fn test_jp_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xC3; rom[1] = 0x00; rom[2] = 0x10; // JP $1000
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x1000);
    }

    #[test]
    fn test_out_in() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x42; // LD A, $42
        rom[2] = 0xD3; rom[3] = 0xBF; // OUT ($BF), A  (VDP control write)
        rom[4] = 0xDB; rom[5] = 0xDC; // IN A, ($DC)   (joypad)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD A, $42
        cpu.step(&mut bus); // OUT ($BF), A
        cpu.step(&mut bus); // IN A, ($DC) -> should read 0xFF (no buttons pressed)
        assert_eq!(cpu.a, 0xFF);
    }

    #[test]
    fn test_ex_de_hl() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0x34; rom[2] = 0x12; // LD HL, $1234
        rom[3] = 0x11; rom[4] = 0x78; rom[5] = 0x56; // LD DE, $5678
        rom[6] = 0xEB; // EX DE, HL
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.hl(), 0x5678);
        assert_eq!(cpu.de(), 0x1234);
    }

    #[test]
    fn test_rst_38() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xFF; // RST $38
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.sp = 0xDFF0;
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0038);
    }

    #[test]
    fn test_add_hl_bc() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0x00; rom[2] = 0x10; // LD HL, $1000
        rom[3] = 0x01; rom[4] = 0x00; rom[5] = 0x20; // LD BC, $2000
        rom[6] = 0x09; // ADD HL, BC
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.hl(), 0x3000);
        assert!(!cpu.flag(FLAG_C));
    }

    // -------------------------------------------------------
    // Interrupt handling
    // -------------------------------------------------------

    #[test]
    fn test_maskable_interrupt_im1() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xFB;                 // EI
        rom[1] = 0x00;                 // NOP (EI takes effect after this)
        rom[2] = 0x00;                 // NOP (interrupt should fire before this)
        rom[0x38] = 0x3E; rom[0x39] = 0x42; // LD A, $42 (ISR at $0038)
        rom[0x3A] = 0xC9;              // RET (actually ED 4D for RETI, but RET works for now)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.im = 1;
        cpu.sp = 0xDFF0;
        cpu.step(&mut bus); // EI (sets ei_pending)
        cpu.step(&mut bus); // NOP (IFF1/IFF2 now enabled from delayed EI)
        assert!(cpu.iff1);
        bus.irq_pending = true;
        let irq_cycles = cpu.handle_interrupts(&mut bus);
        assert_eq!(irq_cycles, 13);
        assert_eq!(cpu.pc, 0x0038);
        assert!(!cpu.iff1); // disabled during ISR
        assert!(!bus.irq_pending); // cleared
        cpu.step(&mut bus); // LD A, $42
        assert_eq!(cpu.a, 0x42);
        cpu.step(&mut bus); // RET
        assert_eq!(cpu.pc, 0x0002); // returns to where we were
    }

    #[test]
    fn test_interrupt_not_handled_when_disabled() {
        let cart = Cartridge::new(vec![0x00; 0x8000]);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.im = 1;
        cpu.sp = 0xDFF0;
        // IFF1 is false by default
        bus.irq_pending = true;
        let cycles = cpu.handle_interrupts(&mut bus);
        assert_eq!(cycles, 0);
        assert_eq!(cpu.pc, 0); // no jump
        assert!(bus.irq_pending); // not cleared
    }

    #[test]
    fn test_halt_resumes_on_interrupt() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xFB; // EI
        rom[1] = 0x76; // HALT
        rom[0x38] = 0xC9; // RET (ISR)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.im = 1;
        cpu.sp = 0xDFF0;
        cpu.step(&mut bus); // EI
        cpu.step(&mut bus); // HALT (also enables interrupts from EI)
        assert!(cpu.halted);
        assert!(cpu.iff1);
        bus.irq_pending = true;
        let irq_cycles = cpu.handle_interrupts(&mut bus);
        assert_eq!(irq_cycles, 13);
        assert!(!cpu.halted); // un-halted
        assert_eq!(cpu.pc, 0x0038);
        cpu.step(&mut bus); // RET
        assert_eq!(cpu.pc, 0x0002); // returns after HALT
    }

    #[test]
    fn test_interrupt_im0_same_as_im1() {
        let mut rom = vec![0x00; 0x8000];
        rom[0x38] = 0x3E; rom[0x39] = 0x99; // LD A, $99
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.im = 0;
        cpu.sp = 0xDFF0;
        cpu.iff1 = true;
        cpu.iff2 = true;
        bus.irq_pending = true;
        cpu.handle_interrupts(&mut bus);
        assert_eq!(cpu.pc, 0x0038);
        cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x99);
    }

    #[test]
    fn test_interrupt_clears_irq_pending() {
        let cart = Cartridge::new(vec![0x00; 0x8000]);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.im = 1;
        cpu.sp = 0xDFF0;
        cpu.iff1 = true;
        bus.irq_pending = true;
        cpu.handle_interrupts(&mut bus);
        assert!(!bus.irq_pending);
    }

    #[test]
    fn test_interrupt_pushes_correct_return_address() {
        let rom = vec![0x00; 0x8000];
        // NOP NOP NOP at 0,1,2
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.im = 1;
        cpu.sp = 0xDFF0;
        cpu.iff1 = true;
        cpu.pc = 0x1234; // simulate being at some address
        bus.irq_pending = true;
        cpu.handle_interrupts(&mut bus);
        // SP should have been decremented by 2
        assert_eq!(cpu.sp, 0xDFEE);
        // Stack should contain return address $1234 (little-endian)
        assert_eq!(bus.read(0xDFEE), 0x34); // low byte
        assert_eq!(bus.read(0xDFEF), 0x12); // high byte
    }

    // -------------------------------------------------------
    // CB prefix instruction tests
    // -------------------------------------------------------

    #[test]
    fn test_rlc_b() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x06; rom[1] = 0x85; // LD B, $85 (10000101)
        rom[2] = 0xCB; rom[3] = 0x00; // RLC B -> 0x0B (00001011), C=1
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.b, 0x0B);
        assert!(cpu.flag(FLAG_C));
    }

    #[test]
    fn test_rrc_a() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x01; // LD A, $01
        rom[2] = 0xCB; rom[3] = 0x0F; // RRC A -> $80, C=1
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x80);
        assert!(cpu.flag(FLAG_C));
    }

    #[test]
    fn test_rl_a_through_carry() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x80; // LD A, $80
        rom[2] = 0x37;                  // SCF (set carry)
        rom[3] = 0xCB; rom[4] = 0x17; // RL A -> $01 (shifted left, old carry into bit 0), new C=1
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x01);
        assert!(cpu.flag(FLAG_C));
    }

    #[test]
    fn test_srl_a() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x81; // LD A, $81
        rom[2] = 0xCB; rom[3] = 0x3F; // SRL A -> $40, C=1
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x40);
        assert!(cpu.flag(FLAG_C));
    }

    #[test]
    fn test_sla_b() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x06; rom[1] = 0xC0; // LD B, $C0
        rom[2] = 0xCB; rom[3] = 0x20; // SLA B -> $80, C=1
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.b, 0x80);
        assert!(cpu.flag(FLAG_C));
    }

    #[test]
    fn test_sra_preserves_bit7() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x80; // LD A, $80
        rom[2] = 0xCB; rom[3] = 0x2F; // SRA A -> $C0 (bit 7 preserved), C=0
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0xC0);
        assert!(!cpu.flag(FLAG_C));
    }

    #[test]
    fn test_bit_7_a_set() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x80; // LD A, $80
        rom[2] = 0xCB; rom[3] = 0x7F; // BIT 7, A
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert!(!cpu.flag(FLAG_Z)); // bit 7 is set
    }

    #[test]
    fn test_bit_0_a_clear() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0xFE; // LD A, $FE (bit 0 clear)
        rom[2] = 0xCB; rom[3] = 0x47; // BIT 0, A
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert!(cpu.flag(FLAG_Z)); // bit 0 is clear
    }

    #[test]
    fn test_set_and_res() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x00; // LD A, 0
        rom[2] = 0xCB; rom[3] = 0xC7; // SET 0, A
        rom[4] = 0xCB; rom[5] = 0xEF; // SET 5, A
        rom[6] = 0xCB; rom[7] = 0x87; // RES 0, A
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        for _ in 0..4 { cpu.step(&mut bus); }
        assert_eq!(cpu.a, 0x20); // bit 5 set, bit 0 cleared
    }

    #[test]
    fn test_cb_with_hl_indirect() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0x00; rom[2] = 0xC0; // LD HL, $C000
        rom[3] = 0xCB; rom[4] = 0x06; // RLC (HL)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC000, 0x85);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD HL
        cpu.step(&mut bus); // RLC (HL)
        assert_eq!(bus.read(0xC000), 0x0B);
        assert!(cpu.flag(FLAG_C));
    }

    #[test]
    fn test_ld_bc_a_and_a_bc() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x42; // LD A, $42
        rom[2] = 0x01; rom[3] = 0x00; rom[4] = 0xC0; // LD BC, $C000
        rom[5] = 0x02; // LD (BC), A
        rom[6] = 0x3E; rom[7] = 0x00; // LD A, 0
        rom[8] = 0x0A; // LD A, (BC)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        for _ in 0..5 { cpu.step(&mut bus); }
        assert_eq!(cpu.a, 0x42);
    }

    #[test]
    fn test_ld_nn_hl_and_hl_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0x34; rom[2] = 0x12; // LD HL, $1234
        rom[3] = 0x22; rom[4] = 0x00; rom[5] = 0xC0; // LD ($C000), HL
        rom[6] = 0x21; rom[7] = 0x00; rom[8] = 0x00; // LD HL, 0
        rom[9] = 0x2A; rom[10] = 0x00; rom[11] = 0xC0; // LD HL, ($C000)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        for _ in 0..4 { cpu.step(&mut bus); }
        assert_eq!(cpu.hl(), 0x1234);
    }

    #[test]
    fn test_inc_dec_rp() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x01; rom[1] = 0xFF; rom[2] = 0xFF; // LD BC, $FFFF
        rom[3] = 0x03; // INC BC
        rom[4] = 0x0B; // DEC BC
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD BC, $FFFF
        cpu.step(&mut bus); // INC BC -> 0
        assert_eq!(cpu.bc(), 0x0000);
        cpu.step(&mut bus); // DEC BC -> 0xFFFF
        assert_eq!(cpu.bc(), 0xFFFF);
    }

    #[test]
    fn test_scf_ccf() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x37; // SCF
        rom[1] = 0x3F; // CCF
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // SCF
        assert!(cpu.flag(FLAG_C));
        assert!(!cpu.flag(FLAG_N));
        assert!(!cpu.flag(FLAG_H));
        cpu.step(&mut bus); // CCF
        assert!(!cpu.flag(FLAG_C));
    }

    // -------------------------------------------------------
    // ED prefix instruction tests
    // -------------------------------------------------------

    #[test]
    fn test_ldir() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0x00; rom[2] = 0xC0;  // LD HL, $C000
        rom[3] = 0x11; rom[4] = 0x10; rom[5] = 0xC0;  // LD DE, $C010
        rom[6] = 0x01; rom[7] = 0x03; rom[8] = 0x00;  // LD BC, 3
        rom[9] = 0xED; rom[10] = 0xB0;                  // LDIR
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC000, 0xAA); bus.write(0xC001, 0xBB); bus.write(0xC002, 0xCC);
        let mut cpu = Cpu::new();
        for _ in 0..6 { cpu.step(&mut bus); } // 3 setup + 3 LDIR iterations
        assert_eq!(bus.read(0xC010), 0xAA);
        assert_eq!(bus.read(0xC011), 0xBB);
        assert_eq!(bus.read(0xC012), 0xCC);
        assert_eq!(cpu.bc(), 0);
    }

    #[test]
    fn test_lddr() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0x02; rom[2] = 0xC0;  // LD HL, $C002
        rom[3] = 0x11; rom[4] = 0x12; rom[5] = 0xC0;  // LD DE, $C012
        rom[6] = 0x01; rom[7] = 0x03; rom[8] = 0x00;  // LD BC, 3
        rom[9] = 0xED; rom[10] = 0xB8;                  // LDDR
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC000, 0x11); bus.write(0xC001, 0x22); bus.write(0xC002, 0x33);
        let mut cpu = Cpu::new();
        for _ in 0..6 { cpu.step(&mut bus); } // 3 setup + 3 LDDR iterations
        assert_eq!(bus.read(0xC010), 0x11);
        assert_eq!(bus.read(0xC011), 0x22);
        assert_eq!(bus.read(0xC012), 0x33);
        assert_eq!(cpu.bc(), 0);
    }

    #[test]
    fn test_sbc_hl_bc() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0x00; rom[2] = 0x10; // LD HL, $1000
        rom[3] = 0x01; rom[4] = 0x01; rom[5] = 0x00; // LD BC, $0001
        rom[6] = 0xED; rom[7] = 0x42;                  // SBC HL, BC
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        for _ in 0..3 { cpu.step(&mut bus); }
        assert_eq!(cpu.hl(), 0x0FFF);
        assert!(cpu.flag(FLAG_N));
    }

    #[test]
    fn test_adc_hl_de() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0xFF; rom[2] = 0x7F; // LD HL, $7FFF
        rom[3] = 0x11; rom[4] = 0x01; rom[5] = 0x00; // LD DE, $0001
        rom[6] = 0xED; rom[7] = 0x5A;                  // ADC HL, DE
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        for _ in 0..3 { cpu.step(&mut bus); }
        assert_eq!(cpu.hl(), 0x8000);
    }

    #[test]
    fn test_neg() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x01;
        rom[2] = 0xED; rom[3] = 0x44; // NEG
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0xFF);
        assert!(cpu.flag(FLAG_C));
        assert!(cpu.flag(FLAG_N));
    }

    #[test]
    fn test_neg_zero() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x00;
        rom[2] = 0xED; rom[3] = 0x44; // NEG
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.flag(FLAG_Z));
        assert!(!cpu.flag(FLAG_C));
    }

    #[test]
    fn test_ld_nn_rp_and_rp_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x01; rom[1] = 0x78; rom[2] = 0x56; // LD BC, $5678
        rom[3] = 0xED; rom[4] = 0x43; rom[5] = 0x00; rom[6] = 0xC0; // LD ($C000), BC
        rom[7] = 0x01; rom[8] = 0x00; rom[9] = 0x00; // LD BC, 0
        rom[10] = 0xED; rom[11] = 0x4B; rom[12] = 0x00; rom[13] = 0xC0; // LD BC, ($C000)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        for _ in 0..4 { cpu.step(&mut bus); }
        assert_eq!(cpu.bc(), 0x5678);
    }

    #[test]
    fn test_im_set() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xED; rom[1] = 0x56; // IM 1
        rom[2] = 0xED; rom[3] = 0x5E; // IM 2
        rom[4] = 0xED; rom[5] = 0x46; // IM 0
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); assert_eq!(cpu.im, 1);
        cpu.step(&mut bus); assert_eq!(cpu.im, 2);
        cpu.step(&mut bus); assert_eq!(cpu.im, 0);
    }

    #[test]
    fn test_reti() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xCD; rom[1] = 0x10; rom[2] = 0x00; // CALL $0010
        rom[0x10] = 0xED; rom[0x11] = 0x4D; // RETI
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.sp = 0xDFF0;
        cpu.step(&mut bus); // CALL
        cpu.step(&mut bus); // RETI
        assert_eq!(cpu.pc, 0x0003);
    }

    #[test]
    fn test_cpir() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0xBB; // LD A, $BB
        rom[2] = 0x21; rom[3] = 0x00; rom[4] = 0xC0; // LD HL, $C000
        rom[5] = 0x01; rom[6] = 0x05; rom[7] = 0x00; // LD BC, 5
        rom[8] = 0xED; rom[9] = 0xB1; // CPIR
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC000, 0xAA);
        bus.write(0xC001, 0xAA);
        bus.write(0xC002, 0xBB); // match at C002
        let mut cpu = Cpu::new();
        for _ in 0..6 { cpu.step(&mut bus); } // 3 setup + 3 CPIR iterations (match at 3rd)
        assert!(cpu.flag(FLAG_Z)); // found match
        assert_eq!(cpu.hl(), 0xC003); // HL advanced past match
    }

    // -------------------------------------------------------
    // DD/FD prefix (IX/IY) tests
    // -------------------------------------------------------

    #[test]
    fn test_ld_ix_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x34; rom[3] = 0x12;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        let cycles = cpu.step(&mut bus);
        assert_eq!(cpu.ix, 0x1234);
        assert_eq!(cycles, 14); // 10 base + 4 prefix
    }

    #[test]
    fn test_ld_iy_nn() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xFD; rom[1] = 0x21; rom[2] = 0x78; rom[3] = 0x56;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        let cycles = cpu.step(&mut bus);
        assert_eq!(cpu.iy, 0x5678);
        assert_eq!(cycles, 14);
    }

    #[test]
    fn test_ld_ix_d_n() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0;
        rom[4] = 0xDD; rom[5] = 0x36; rom[6] = 0x05; rom[7] = 0x42;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.read(0xC005), 0x42);
    }

    #[test]
    fn test_ld_ix_d_negative_displacement() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x10; rom[3] = 0xC0; // LD IX, $C010
        rom[4] = 0xDD; rom[5] = 0x36; rom[6] = 0xFB; rom[7] = 0x99; // LD (IX-5), $99
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.read(0xC00B), 0x99); // C010 + (-5) = C00B
    }

    #[test]
    fn test_add_ix_bc() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0x10; // LD IX, $1000
        rom[4] = 0x01; rom[5] = 0x34; rom[6] = 0x12; // LD BC, $1234
        rom[7] = 0xDD; rom[8] = 0x09; // ADD IX, BC
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.ix, 0x2234);
    }

    #[test]
    fn test_push_pop_ix() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0xCD; rom[3] = 0xAB; // LD IX, $ABCD
        rom[4] = 0xDD; rom[5] = 0xE5; // PUSH IX
        rom[6] = 0xDD; rom[7] = 0x21; rom[8] = 0x00; rom[9] = 0x00; // LD IX, $0000
        rom[10] = 0xDD; rom[11] = 0xE1; // POP IX
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.ix, 0xABCD);
    }

    #[test]
    fn test_inc_dec_ix() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0x10; // LD IX, $1000
        rom[4] = 0xDD; rom[5] = 0x23; // INC IX
        rom[6] = 0xDD; rom[7] = 0x23; // INC IX
        rom[8] = 0xDD; rom[9] = 0x2B; // DEC IX
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.ix, 0x1001);
    }

    #[test]
    fn test_ld_a_ix_d() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, $C000
        rom[4] = 0xDD; rom[5] = 0x7E; rom[6] = 0x03; // LD A, (IX+3)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        bus.write(0xC003, 0x55);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x55);
    }

    #[test]
    fn test_ddcb_bit() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, $C000
        rom[4] = 0xDD; rom[5] = 0xCB; rom[6] = 0x03; rom[7] = 0x46; // BIT 0, (IX+3)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC003, 0x01);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert!(!cpu.flag(FLAG_Z)); // bit 0 is set
    }

    #[test]
    fn test_ddcb_set() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, $C000
        rom[4] = 0xDD; rom[5] = 0xCB; rom[6] = 0x02; rom[7] = 0xD6; // SET 2, (IX+2)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC002, 0x00);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.read(0xC002), 0x04); // bit 2 set
    }

    #[test]
    fn test_ddcb_res() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, $C000
        rom[4] = 0xDD; rom[5] = 0xCB; rom[6] = 0x01; rom[7] = 0x8E; // RES 1, (IX+1)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC001, 0xFF);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.read(0xC001), 0xFD); // bit 1 cleared
    }

    #[test]
    fn test_ddcb_rlc() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, $C000
        rom[4] = 0xDD; rom[5] = 0xCB; rom[6] = 0x00; rom[7] = 0x06; // RLC (IX+0)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC000, 0x81); // 1000_0001
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.read(0xC000), 0x03); // rotated left: 0000_0011
        assert!(cpu.flag(FLAG_C)); // old bit 7 was 1
    }

    #[test]
    fn test_ex_sp_ix() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x34; rom[3] = 0x12; // LD IX, $1234
        rom[4] = 0x31; rom[5] = 0xFE; rom[6] = 0xDF; // LD SP, $DFFE
        rom[7] = 0xDD; rom[8] = 0xE3; // EX (SP), IX
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xDFFE, 0x78);
        bus.write(0xDFFF, 0x56);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD IX, $1234
        cpu.step(&mut bus); // LD SP, $DFFE
        cpu.step(&mut bus); // EX (SP), IX
        assert_eq!(cpu.ix, 0x5678);
        assert_eq!(bus.read(0xDFFE), 0x34);
        assert_eq!(bus.read(0xDFFF), 0x12);
    }

    #[test]
    fn test_jp_ix() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0x01; // LD IX, $0100
        rom[4] = 0xDD; rom[5] = 0xE9; // JP (IX)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x0100);
    }

    #[test]
    fn test_ld_sp_ix() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xDF; // LD IX, $DF00
        rom[4] = 0xDD; rom[5] = 0xF9; // LD SP, IX
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.sp, 0xDF00);
    }

    #[test]
    fn test_inc_ixh() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0x10; // LD IX, $1000
        rom[4] = 0xDD; rom[5] = 0x24; // INC IXH
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.ix, 0x1100);
    }

    #[test]
    fn test_inc_ixl() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x34; rom[3] = 0x12; // LD IX, $1234
        rom[4] = 0xDD; rom[5] = 0x2C; // INC IXL
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.ix, 0x1235);
    }

    #[test]
    fn test_ld_ixh_n() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x26; rom[2] = 0x42; // LD IXH, $42
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!((cpu.ix >> 8) as u8, 0x42);
    }

    #[test]
    fn test_add_a_ixl() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x05; rom[3] = 0x00; // LD IX, $0005
        rom[4] = 0x3E; rom[5] = 0x10; // LD A, $10
        rom[6] = 0xDD; rom[7] = 0x85; // ADD A, IXL
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x15);
    }

    #[test]
    fn test_ld_nn_ix() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0xCD; rom[3] = 0xAB; // LD IX, $ABCD
        rom[4] = 0xDD; rom[5] = 0x22; rom[6] = 0x00; rom[7] = 0xC0; // LD ($C000), IX
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.read(0xC000), 0xCD);
        assert_eq!(bus.read(0xC001), 0xAB);
    }

    #[test]
    fn test_ld_ix_nn_indirect() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x2A; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, ($C000)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC000, 0x78);
        bus.write(0xC001, 0x56);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!(cpu.ix, 0x5678);
    }

    #[test]
    fn test_dd_fd_prefix_chaining() {
        // DD FD 21 nn nn -> LD IY, nn (FD wins, restarts from DD)
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0xFD; rom[2] = 0x21; rom[3] = 0x34; rom[4] = 0x12;
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert_eq!(cpu.iy, 0x1234); // FD wins, so IY is loaded
        assert_eq!(cpu.ix, 0x0000); // IX unchanged
    }

    #[test]
    fn test_dd_ed_prefix_chaining() {
        // DD ED 44 -> NEG (DD ignored before ED)
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x3E; rom[1] = 0x05; // LD A, 5
        rom[2] = 0xDD; rom[3] = 0xED; rom[4] = 0x44; // DD ED NEG
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.a, 0xFB); // 0 - 5 = -5 = 0xFB
    }

    #[test]
    fn test_ddcb_rlc_undocumented_store() {
        // DD CB dd C0 = RLC (IX+d), B  (result stored to BOTH memory AND register B)
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, $C000
        rom[4] = 0xDD; rom[5] = 0xCB; rom[6] = 0x02; rom[7] = 0x00; // RLC (IX+2), B
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC002, 0x81); // 1000_0001
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD IX, $C000
        cpu.step(&mut bus); // RLC (IX+2), B
        assert_eq!(bus.read(0xC002), 0x03); // rotated left: 0000_0011
        assert_eq!(cpu.b, 0x03); // undocumented: result also stored in B
    }

    #[test]
    fn test_ld_l_ix_d_uses_normal_l() {
        // DD 6E dd = LD L, (IX+d) - should store to NORMAL L, not IXL
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, $C000
        rom[4] = 0xDD; rom[5] = 0x6E; rom[6] = 0x05; // LD L, (IX+5)
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        bus.write(0xC005, 0x42);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD IX, $C000
        cpu.step(&mut bus); // LD L, (IX+5)
        assert_eq!(cpu.l, 0x42); // normal L register
        assert_eq!(cpu.ix, 0xC000); // IX unchanged (IXL not modified)
    }

    #[test]
    fn test_ld_ix_d_h_uses_normal_h() {
        // DD 74 dd = LD (IX+d), H - should read from NORMAL H, not IXH
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IX, $C000
        rom[4] = 0x26; rom[5] = 0x99; // LD H, $99
        rom[6] = 0xDD; rom[7] = 0x74; rom[8] = 0x03; // LD (IX+3), H
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD IX, $C000
        cpu.step(&mut bus); // LD H, $99
        cpu.step(&mut bus); // LD (IX+3), H
        assert_eq!(bus.read(0xC003), 0x99); // stored normal H value, not IXH (0xC0)
    }

    #[test]
    fn test_ld_ixh_ixl_uses_half_regs() {
        // DD 65 = LD IXH, IXL
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xDD; rom[1] = 0x21; rom[2] = 0xAB; rom[3] = 0xCD; // LD IX, $CDAB
        rom[4] = 0xDD; rom[5] = 0x65; // LD IXH, IXL
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD IX, $CDAB
        cpu.step(&mut bus); // LD IXH, IXL
        assert_eq!(cpu.ix, 0xABAB); // high byte becomes same as low byte
    }

    #[test]
    fn test_ex_de_hl_unaffected_by_dd() {
        // DD EB = EX DE, HL (DD prefix should NOT make this EX DE, IX)
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0x21; rom[1] = 0x34; rom[2] = 0x12; // LD HL, $1234
        rom[3] = 0x11; rom[4] = 0x78; rom[5] = 0x56; // LD DE, $5678
        rom[6] = 0xDD; rom[7] = 0x21; rom[8] = 0xCD; rom[9] = 0xAB; // LD IX, $ABCD
        rom[10] = 0xDD; rom[11] = 0xEB; // DD EX DE, HL
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD HL
        cpu.step(&mut bus); // LD DE
        cpu.step(&mut bus); // LD IX
        cpu.step(&mut bus); // DD EX DE, HL
        assert_eq!(cpu.hl(), 0x5678); // HL and DE swapped
        assert_eq!(cpu.de(), 0x1234);
        assert_eq!(cpu.ix, 0xABCD); // IX unaffected
    }

    #[test]
    fn test_push_pop_iy() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xFD; rom[1] = 0x21; rom[2] = 0xEF; rom[3] = 0xBE; // LD IY, $BEEF
        rom[4] = 0xFD; rom[5] = 0xE5; // PUSH IY
        rom[6] = 0xFD; rom[7] = 0x21; rom[8] = 0x00; rom[9] = 0x00; // LD IY, $0000
        rom[10] = 0xFD; rom[11] = 0xE1; // POP IY
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.iy, 0xBEEF);
    }

    #[test]
    fn test_ld_iy_d_n() {
        let mut rom = vec![0x00; 0x8000];
        rom[0] = 0xFD; rom[1] = 0x21; rom[2] = 0x00; rom[3] = 0xC0; // LD IY, $C000
        rom[4] = 0xFD; rom[5] = 0x36; rom[6] = 0x07; rom[7] = 0xAA; // LD (IY+7), $AA
        let cart = Cartridge::new(rom);
        let mut bus = Bus::new(cart, None);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.read(0xC007), 0xAA);
    }
}
