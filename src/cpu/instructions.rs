// Z80 instruction decoding and execution
//
// Decode scheme uses the standard x/y/z/p/q bit extraction from the opcode byte:
//   x = bits 7-6  (opcode >> 6) & 0x03
//   y = bits 5-3  (opcode >> 3) & 0x07
//   z = bits 2-0  opcode & 0x07
//   p = bits 5-4  y >> 1
//   q = bit  3    y & 0x01
//
// Register indices (for r[y] / r[z]):
//   0=B, 1=C, 2=D, 3=E, 4=H, 5=L, 6=(HL), 7=A
//
// Register pair indices (rp):
//   0=BC, 1=DE, 2=HL, 3=SP
//
// Register pair indices (rp2):
//   0=BC, 1=DE, 2=HL, 3=AF

use crate::bus::Bus;
use super::{Cpu, FLAG_C, FLAG_N, FLAG_PV, FLAG_X, FLAG_H, FLAG_Y, FLAG_Z, FLAG_S};

// ---------------------------------------------------------------------------
// Parity helper
// ---------------------------------------------------------------------------

fn parity_even(val: u8) -> bool {
    val.count_ones() % 2 == 0
}

// ---------------------------------------------------------------------------
// ALU helper functions
// ---------------------------------------------------------------------------

/// ADD A, val (carry=false) or ADC A, val (carry=true)
fn alu_add(cpu: &mut Cpu, val: u8, carry: bool) {
    let c = if carry && cpu.flag(FLAG_C) { 1u16 } else { 0 };
    let a = cpu.a as u16;
    let v = val as u16;
    let result = a + v + c;
    let half = (a & 0x0F) + (v & 0x0F) + c;
    cpu.f = 0;
    if result & 0xFF == 0 { cpu.f |= FLAG_Z; }
    if result & 0x80 != 0 { cpu.f |= FLAG_S; }
    if result > 0xFF { cpu.f |= FLAG_C; }
    if half > 0x0F { cpu.f |= FLAG_H; }
    if ((a ^ v ^ 0x80) & (a ^ result)) & 0x80 != 0 { cpu.f |= FLAG_PV; }
    let r = result as u8;
    cpu.f |= r & (FLAG_X | FLAG_Y);
    cpu.a = r;
}

/// SUB A, val (carry=false, is_cp=false), SBC A, val (carry=true, is_cp=false),
/// CP A, val (carry=false, is_cp=true)
fn alu_sub(cpu: &mut Cpu, val: u8, carry: bool, is_cp: bool) {
    let c = if carry && cpu.flag(FLAG_C) { 1u16 } else { 0 };
    let a = cpu.a as u16;
    let v = val as u16;
    let result = a.wrapping_sub(v).wrapping_sub(c);
    let half = (a & 0x0F).wrapping_sub(v & 0x0F).wrapping_sub(c);
    cpu.f = FLAG_N;
    if result & 0xFF == 0 { cpu.f |= FLAG_Z; }
    if result & 0x80 != 0 { cpu.f |= FLAG_S; }
    if result > 0xFF { cpu.f |= FLAG_C; }
    if half > 0x0F { cpu.f |= FLAG_H; }
    if ((a ^ v) & (a ^ result)) & 0x80 != 0 { cpu.f |= FLAG_PV; }
    if is_cp {
        cpu.f |= val & (FLAG_X | FLAG_Y); // CP: undoc flags from operand
    } else {
        cpu.f |= (result as u8) & (FLAG_X | FLAG_Y);
        cpu.a = result as u8;
    }
}

/// AND A, val
fn alu_and(cpu: &mut Cpu, val: u8) {
    cpu.a &= val;
    cpu.f = FLAG_H | if cpu.a == 0 { FLAG_Z } else { 0 }
        | (cpu.a & FLAG_S) | (cpu.a & (FLAG_X | FLAG_Y))
        | if parity_even(cpu.a) { FLAG_PV } else { 0 };
}

/// OR A, val
fn alu_or(cpu: &mut Cpu, val: u8) {
    cpu.a |= val;
    cpu.f = if cpu.a == 0 { FLAG_Z } else { 0 }
        | (cpu.a & FLAG_S) | (cpu.a & (FLAG_X | FLAG_Y))
        | if parity_even(cpu.a) { FLAG_PV } else { 0 };
}

/// XOR A, val
fn alu_xor(cpu: &mut Cpu, val: u8) {
    cpu.a ^= val;
    cpu.f = if cpu.a == 0 { FLAG_Z } else { 0 }
        | (cpu.a & FLAG_S) | (cpu.a & (FLAG_X | FLAG_Y))
        | if parity_even(cpu.a) { FLAG_PV } else { 0 };
}

/// Dispatch ALU operation by index y (0=ADD, 1=ADC, 2=SUB, 3=SBC, 4=AND, 5=XOR, 6=OR, 7=CP)
fn alu_op(cpu: &mut Cpu, y: u8, val: u8) {
    match y {
        0 => alu_add(cpu, val, false),  // ADD
        1 => alu_add(cpu, val, true),   // ADC
        2 => alu_sub(cpu, val, false, false), // SUB
        3 => alu_sub(cpu, val, true, false),  // SBC
        4 => alu_and(cpu, val),         // AND
        5 => alu_xor(cpu, val),         // XOR
        6 => alu_or(cpu, val),          // OR
        7 => alu_sub(cpu, val, false, true),  // CP
        _ => unreachable!(),
    }
}

/// INC byte with flags (preserves carry)
fn inc_byte(cpu: &mut Cpu, val: u8) -> u8 {
    let result = val.wrapping_add(1);
    cpu.f = (cpu.f & FLAG_C) // preserve carry
        | if result == 0 { FLAG_Z } else { 0 }
        | (result & FLAG_S)
        | (result & (FLAG_X | FLAG_Y))
        | if val & 0x0F == 0x0F { FLAG_H } else { 0 }
        | if val == 0x7F { FLAG_PV } else { 0 };
    result
}

/// DEC byte with flags (preserves carry)
fn dec_byte(cpu: &mut Cpu, val: u8) -> u8 {
    let result = val.wrapping_sub(1);
    cpu.f = (cpu.f & FLAG_C) | FLAG_N
        | if result == 0 { FLAG_Z } else { 0 }
        | (result & FLAG_S)
        | (result & (FLAG_X | FLAG_Y))
        | if val & 0x0F == 0x00 { FLAG_H } else { 0 }
        | if val == 0x80 { FLAG_PV } else { 0 };
    result
}

/// ADD HL, rp with flags
fn add_hl(cpu: &mut Cpu, val: u16) {
    let hl = cpu.hl() as u32;
    let v = val as u32;
    let result = hl + v;
    let half = (hl & 0x0FFF) + (v & 0x0FFF);
    cpu.f = (cpu.f & (FLAG_Z | FLAG_PV | FLAG_S)) // preserve Z, PV, S
        | if result > 0xFFFF { FLAG_C } else { 0 }
        | if half > 0x0FFF { FLAG_H } else { 0 }
        | ((result >> 8) as u8 & (FLAG_X | FLAG_Y));
    cpu.set_hl(result as u16);
}

/// DAA - Decimal Adjust Accumulator
fn daa(cpu: &mut Cpu) {
    let mut a = cpu.a as u16;
    let mut correction: u16 = 0;
    let mut carry = false;
    if cpu.flag(FLAG_H) || (!cpu.flag(FLAG_N) && (a & 0x0F) > 9) {
        correction |= 0x06;
    }
    if cpu.flag(FLAG_C) || (!cpu.flag(FLAG_N) && a > 0x99) {
        correction |= 0x60;
        carry = true;
    }
    if cpu.flag(FLAG_N) {
        a = a.wrapping_sub(correction);
    } else {
        a = a.wrapping_add(correction);
    }
    let result = (a & 0xFF) as u8;
    cpu.f = (cpu.f & FLAG_N)
        | if carry { FLAG_C } else { 0 }
        | if result == 0 { FLAG_Z } else { 0 }
        | (result & FLAG_S)
        | (result & (FLAG_X | FLAG_Y))
        | if parity_even(result) { FLAG_PV } else { 0 }
        | if (cpu.a ^ result) & 0x10 != 0 { FLAG_H } else { 0 };
    cpu.a = result;
}

// ---------------------------------------------------------------------------
// Stack helpers
// ---------------------------------------------------------------------------

fn push(cpu: &mut Cpu, bus: &mut Bus, val: u16) {
    cpu.sp = cpu.sp.wrapping_sub(1);
    bus.write(cpu.sp, (val >> 8) as u8);
    cpu.sp = cpu.sp.wrapping_sub(1);
    bus.write(cpu.sp, val as u8);
}

fn pop(cpu: &mut Cpu, bus: &Bus) -> u16 {
    let lo = bus.read(cpu.sp) as u16;
    cpu.sp = cpu.sp.wrapping_add(1);
    let hi = bus.read(cpu.sp) as u16;
    cpu.sp = cpu.sp.wrapping_add(1);
    (hi << 8) | lo
}

// ---------------------------------------------------------------------------
// Register helper functions
// ---------------------------------------------------------------------------

/// Read an 8-bit register by index (0=B, 1=C, 2=D, 3=E, 4=H, 5=L, 6=(HL), 7=A).
pub fn get_reg(cpu: &Cpu, bus: &Bus, idx: u8) -> u8 {
    match idx {
        0 => cpu.b,
        1 => cpu.c,
        2 => cpu.d,
        3 => cpu.e,
        4 => cpu.h,
        5 => cpu.l,
        6 => bus.read(cpu.hl()),
        7 => cpu.a,
        _ => unreachable!(),
    }
}

/// Write an 8-bit register by index (0=B, 1=C, 2=D, 3=E, 4=H, 5=L, 6=(HL), 7=A).
pub fn set_reg(cpu: &mut Cpu, bus: &mut Bus, idx: u8, val: u8) {
    match idx {
        0 => cpu.b = val,
        1 => cpu.c = val,
        2 => cpu.d = val,
        3 => cpu.e = val,
        4 => cpu.h = val,
        5 => cpu.l = val,
        6 => bus.write(cpu.hl(), val),
        7 => cpu.a = val,
        _ => unreachable!(),
    }
}

/// Read a 16-bit register pair by index (0=BC, 1=DE, 2=HL, 3=SP).
pub fn get_rp(cpu: &Cpu, idx: u8) -> u16 {
    match idx {
        0 => cpu.bc(),
        1 => cpu.de(),
        2 => cpu.hl(),
        3 => cpu.sp,
        _ => unreachable!(),
    }
}

/// Write a 16-bit register pair by index (0=BC, 1=DE, 2=HL, 3=SP).
pub fn set_rp(cpu: &mut Cpu, idx: u8, val: u16) {
    match idx {
        0 => cpu.set_bc(val),
        1 => cpu.set_de(val),
        2 => cpu.set_hl(val),
        3 => cpu.sp = val,
        _ => unreachable!(),
    }
}

/// Read a 16-bit register pair by index (0=BC, 1=DE, 2=HL, 3=AF).
pub fn get_rp2(cpu: &Cpu, idx: u8) -> u16 {
    match idx {
        0 => cpu.bc(),
        1 => cpu.de(),
        2 => cpu.hl(),
        3 => cpu.af(),
        _ => unreachable!(),
    }
}

/// Write a 16-bit register pair by index (0=BC, 1=DE, 2=HL, 3=AF).
pub fn set_rp2(cpu: &mut Cpu, idx: u8, val: u16) {
    match idx {
        0 => cpu.set_bc(val),
        1 => cpu.set_de(val),
        2 => cpu.set_hl(val),
        3 => cpu.set_af(val),
        _ => unreachable!(),
    }
}

/// Check a condition code (0=NZ, 1=Z, 2=NC, 3=C, 4=PO, 5=PE, 6=P, 7=M).
pub fn check_condition(cpu: &Cpu, cc: u8) -> bool {
    match cc {
        0 => !cpu.flag(FLAG_Z),  // NZ
        1 => cpu.flag(FLAG_Z),   // Z
        2 => !cpu.flag(FLAG_C),  // NC
        3 => cpu.flag(FLAG_C),   // C
        4 => !cpu.flag(FLAG_PV), // PO
        5 => cpu.flag(FLAG_PV),  // PE
        6 => !cpu.flag(FLAG_S),  // P
        7 => cpu.flag(FLAG_S),   // M
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Prefix stub handlers (to be filled in Tasks 7-9)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// CB-prefix rotate/shift/bit helper functions
// ---------------------------------------------------------------------------

fn rlc(cpu: &mut Cpu, val: u8) -> u8 {
    let carry = val >> 7;
    let result = (val << 1) | carry;
    cpu.f = (result & (FLAG_S | FLAG_X | FLAG_Y))
        | if result == 0 { FLAG_Z } else { 0 }
        | if parity_even(result) { FLAG_PV } else { 0 }
        | carry;
    result
}

fn rrc(cpu: &mut Cpu, val: u8) -> u8 {
    let carry = val & 1;
    let result = (val >> 1) | (carry << 7);
    cpu.f = (result & (FLAG_S | FLAG_X | FLAG_Y))
        | if result == 0 { FLAG_Z } else { 0 }
        | if parity_even(result) { FLAG_PV } else { 0 }
        | carry;
    result
}

fn rl(cpu: &mut Cpu, val: u8) -> u8 {
    let old_carry = if cpu.flag(FLAG_C) { 1 } else { 0 };
    let new_carry = val >> 7;
    let result = (val << 1) | old_carry;
    cpu.f = (result & (FLAG_S | FLAG_X | FLAG_Y))
        | if result == 0 { FLAG_Z } else { 0 }
        | if parity_even(result) { FLAG_PV } else { 0 }
        | new_carry;
    result
}

fn rr(cpu: &mut Cpu, val: u8) -> u8 {
    let old_carry = if cpu.flag(FLAG_C) { 1 } else { 0 };
    let new_carry = val & 1;
    let result = (val >> 1) | (old_carry << 7);
    cpu.f = (result & (FLAG_S | FLAG_X | FLAG_Y))
        | if result == 0 { FLAG_Z } else { 0 }
        | if parity_even(result) { FLAG_PV } else { 0 }
        | new_carry;
    result
}

fn sla(cpu: &mut Cpu, val: u8) -> u8 {
    let carry = val >> 7;
    let result = val << 1;
    cpu.f = (result & (FLAG_S | FLAG_X | FLAG_Y))
        | if result == 0 { FLAG_Z } else { 0 }
        | if parity_even(result) { FLAG_PV } else { 0 }
        | carry;
    result
}

fn sra(cpu: &mut Cpu, val: u8) -> u8 {
    let carry = val & 1;
    let result = (val & 0x80) | (val >> 1);
    cpu.f = (result & (FLAG_S | FLAG_X | FLAG_Y))
        | if result == 0 { FLAG_Z } else { 0 }
        | if parity_even(result) { FLAG_PV } else { 0 }
        | carry;
    result
}

fn sll(cpu: &mut Cpu, val: u8) -> u8 {
    let carry = val >> 7;
    let result = (val << 1) | 0x01;
    cpu.f = (result & (FLAG_S | FLAG_X | FLAG_Y))
        | if result == 0 { FLAG_Z } else { 0 }
        | if parity_even(result) { FLAG_PV } else { 0 }
        | carry;
    result
}

fn srl(cpu: &mut Cpu, val: u8) -> u8 {
    let carry = val & 1;
    let result = val >> 1;
    cpu.f = (result & (FLAG_S | FLAG_X | FLAG_Y))
        | if result == 0 { FLAG_Z } else { 0 }
        | if parity_even(result) { FLAG_PV } else { 0 }
        | carry;
    result
}

fn cb_bit(cpu: &mut Cpu, bit_num: u8, val: u8) {
    let tested = val & (1 << bit_num);
    cpu.f = (cpu.f & FLAG_C)
        | FLAG_H
        | if tested == 0 { FLAG_Z | FLAG_PV } else { 0 }
        | if bit_num == 7 && tested != 0 { FLAG_S } else { 0 }
        | (val & (FLAG_X | FLAG_Y));
}

/// CB prefix: rotate/shift, BIT, RES, SET instructions.
fn execute_cb(cpu: &mut Cpu, bus: &mut Bus) -> u32 {
    cpu.increment_r();
    let opcode = cpu.fetch(bus);
    let x = (opcode >> 6) & 0x03;
    let y = (opcode >> 3) & 0x07;
    let z = opcode & 0x07;

    match x {
        0 => {
            // Rotate/shift: rot[y] r[z]
            let val = get_reg(cpu, bus, z);
            let result = match y {
                0 => rlc(cpu, val),
                1 => rrc(cpu, val),
                2 => rl(cpu, val),
                3 => rr(cpu, val),
                4 => sla(cpu, val),
                5 => sra(cpu, val),
                6 => sll(cpu, val),
                7 => srl(cpu, val),
                _ => unreachable!(),
            };
            set_reg(cpu, bus, z, result);
            if z == 6 { 15 } else { 8 }
        }
        1 => {
            // BIT y, r[z]
            let val = get_reg(cpu, bus, z);
            cb_bit(cpu, y, val);
            if z == 6 { 12 } else { 8 }
        }
        2 => {
            // RES y, r[z]
            let val = get_reg(cpu, bus, z);
            set_reg(cpu, bus, z, val & !(1 << y));
            if z == 6 { 15 } else { 8 }
        }
        3 => {
            // SET y, r[z]
            let val = get_reg(cpu, bus, z);
            set_reg(cpu, bus, z, val | (1 << y));
            if z == 6 { 15 } else { 8 }
        }
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// DD/FD prefix (IX/IY) helper functions
// ---------------------------------------------------------------------------

fn get_index_reg(cpu: &Cpu, is_iy: bool) -> u16 {
    if is_iy { cpu.iy } else { cpu.ix }
}

fn set_index_reg(cpu: &mut Cpu, is_iy: bool, val: u16) {
    if is_iy { cpu.iy = val } else { cpu.ix = val }
}

fn get_index_reg_high(cpu: &Cpu, is_iy: bool) -> u8 {
    (get_index_reg(cpu, is_iy) >> 8) as u8
}

fn set_index_reg_high(cpu: &mut Cpu, is_iy: bool, val: u8) {
    let ir = get_index_reg(cpu, is_iy);
    set_index_reg(cpu, is_iy, (ir & 0x00FF) | ((val as u16) << 8));
}

fn get_index_reg_low(cpu: &Cpu, is_iy: bool) -> u8 {
    get_index_reg(cpu, is_iy) as u8
}

fn set_index_reg_low(cpu: &mut Cpu, is_iy: bool, val: u8) {
    let ir = get_index_reg(cpu, is_iy);
    set_index_reg(cpu, is_iy, (ir & 0xFF00) | (val as u16));
}

/// ADD IX/IY, rp with flags (same logic as add_hl but operates on IX/IY)
fn add_index(cpu: &mut Cpu, is_iy: bool, val: u16) {
    let ir = get_index_reg(cpu, is_iy) as u32;
    let v = val as u32;
    let result = ir + v;
    let half = (ir & 0x0FFF) + (v & 0x0FFF);
    cpu.f = (cpu.f & (FLAG_Z | FLAG_PV | FLAG_S)) // preserve Z, PV, S
        | if result > 0xFFFF { FLAG_C } else { 0 }
        | if half > 0x0FFF { FLAG_H } else { 0 }
        | ((result >> 8) as u8 & (FLAG_X | FLAG_Y));
    set_index_reg(cpu, is_iy, result as u16);
}

/// Read an 8-bit register for DD/FD-prefixed instructions.
/// Index 4 -> IXH/IYH, index 5 -> IXL/IYL, index 6 -> (IX/IY+d) at given addr.
/// For index 6, `addr` is the precomputed effective address.
/// For indices 0-3 and 7, returns normal register.
fn get_reg_ix(cpu: &Cpu, bus: &Bus, idx: u8, is_iy: bool, addr: u16) -> u8 {
    match idx {
        0 => cpu.b,
        1 => cpu.c,
        2 => cpu.d,
        3 => cpu.e,
        4 => get_index_reg_high(cpu, is_iy),
        5 => get_index_reg_low(cpu, is_iy),
        6 => bus.read(addr),
        7 => cpu.a,
        _ => unreachable!(),
    }
}

/// Write an 8-bit register for DD/FD-prefixed instructions.
/// Index 4 -> IXH/IYH, index 5 -> IXL/IYL, index 6 -> (IX/IY+d) at given addr.
fn set_reg_ix(cpu: &mut Cpu, bus: &mut Bus, idx: u8, is_iy: bool, addr: u16, val: u8) {
    match idx {
        0 => cpu.b = val,
        1 => cpu.c = val,
        2 => cpu.d = val,
        3 => cpu.e = val,
        4 => set_index_reg_high(cpu, is_iy, val),
        5 => set_index_reg_low(cpu, is_iy, val),
        6 => bus.write(addr, val),
        7 => cpu.a = val,
        _ => unreachable!(),
    }
}

/// Get register pair with IX/IY substitution (index 2 = IX/IY instead of HL).
fn get_rp_ix(cpu: &Cpu, idx: u8, is_iy: bool) -> u16 {
    match idx {
        0 => cpu.bc(),
        1 => cpu.de(),
        2 => get_index_reg(cpu, is_iy),
        3 => cpu.sp,
        _ => unreachable!(),
    }
}

/// Set register pair with IX/IY substitution (index 2 = IX/IY instead of HL).
fn set_rp_ix(cpu: &mut Cpu, idx: u8, is_iy: bool, val: u16) {
    match idx {
        0 => cpu.set_bc(val),
        1 => cpu.set_de(val),
        2 => set_index_reg(cpu, is_iy, val),
        3 => cpu.sp = val,
        _ => unreachable!(),
    }
}

/// Compute effective address: IX/IY + signed displacement
fn compute_ix_addr(cpu: &Cpu, is_iy: bool, d: u8) -> u16 {
    let base = get_index_reg(cpu, is_iy);
    base.wrapping_add((d as i8) as u16)
}

/// DDCB/FDCB prefix handler.
/// Byte sequence: DD CB dd oo (displacement before opcode).
fn execute_ddcb_fdcb(cpu: &mut Cpu, bus: &mut Bus, is_iy: bool) -> u32 {
    let d = cpu.fetch(bus); // displacement byte
    let opcode = cpu.fetch(bus); // opcode byte
    let addr = compute_ix_addr(cpu, is_iy, d);

    let x = (opcode >> 6) & 0x03;
    let y = (opcode >> 3) & 0x07;
    let z = opcode & 0x07;

    match x {
        0 => {
            // Rotate/shift: rot[y] (IX/IY+d), optionally copy to r[z]
            let val = bus.read(addr);
            let result = match y {
                0 => rlc(cpu, val),
                1 => rrc(cpu, val),
                2 => rl(cpu, val),
                3 => rr(cpu, val),
                4 => sla(cpu, val),
                5 => sra(cpu, val),
                6 => sll(cpu, val),
                7 => srl(cpu, val),
                _ => unreachable!(),
            };
            bus.write(addr, result);
            // When z != 6, also store result in register r[z] (undocumented)
            if z != 6 {
                set_reg(cpu, bus, z, result);
            }
            23
        }
        1 => {
            // BIT y, (IX/IY+d)
            let val = bus.read(addr);
            let tested = val & (1 << y);
            // For BIT with (IX+d), undocumented flags come from high byte of addr
            let addr_high = (addr >> 8) as u8;
            cpu.f = (cpu.f & FLAG_C)
                | FLAG_H
                | if tested == 0 { FLAG_Z | FLAG_PV } else { 0 }
                | if y == 7 && tested != 0 { FLAG_S } else { 0 }
                | (addr_high & (FLAG_X | FLAG_Y));
            20
        }
        2 => {
            // RES y, (IX/IY+d), optionally copy to r[z]
            let val = bus.read(addr);
            let result = val & !(1 << y);
            bus.write(addr, result);
            if z != 6 {
                set_reg(cpu, bus, z, result);
            }
            23
        }
        3 => {
            // SET y, (IX/IY+d), optionally copy to r[z]
            let val = bus.read(addr);
            let result = val | (1 << y);
            bus.write(addr, result);
            if z != 6 {
                set_reg(cpu, bus, z, result);
            }
            23
        }
        _ => unreachable!(),
    }
}

/// Shared DD/FD prefix handler. `is_iy` selects IY (true) vs IX (false).
fn execute_dd_fd(cpu: &mut Cpu, bus: &mut Bus, is_iy: bool) -> u32 {
    cpu.increment_r();
    let opcode = cpu.fetch(bus);

    // Prefix chaining: DD DD -> restart as DD, DD FD -> restart as FD
    // DD ED -> delegate to execute_ed (DD/FD ignored before ED)
    match opcode {
        0xDD => return 4 + execute_dd_fd(cpu, bus, false),
        0xFD => return 4 + execute_dd_fd(cpu, bus, true),
        0xED => return 4 + execute_ed(cpu, bus),
        0xCB => return execute_ddcb_fdcb(cpu, bus, is_iy),
        _ => {}
    }

    let x = (opcode >> 6) & 0x03;
    let y = (opcode >> 3) & 0x07;
    let z = opcode & 0x07;
    let p = y >> 1;
    let q = y & 0x01;

    // Determine if this opcode references HL-related registers.
    // If not, execute as a normal unprefixed opcode (no IX/IY substitution).
    match x {
        0 => match z {
            1 => {
                if q == 0 {
                    // LD rp[p], nn - only substitutes if p == 2 (HL)
                    let val = cpu.fetch_word(bus);
                    set_rp_ix(cpu, p, is_iy, val);
                    14 // +4 for prefix
                } else {
                    // ADD HL, rp[p] -> ADD IX/IY, rp[p]
                    let val = get_rp_ix(cpu, p, is_iy);
                    add_index(cpu, is_iy, val);
                    15 // 11 base + 4 prefix
                }
            },
            2 => {
                if q == 0 {
                    match p {
                        0 | 1 | 3 => {
                            // LD (BC),A / LD (DE),A / LD (nn),A - no HL, run unprefixed +4
                            4 + execute_unprefixed(cpu, bus, opcode)
                        },
                        2 => {
                            // LD (nn), HL -> LD (nn), IX/IY
                            let addr = cpu.fetch_word(bus);
                            let ir = get_index_reg(cpu, is_iy);
                            bus.write(addr, ir as u8);
                            bus.write(addr.wrapping_add(1), (ir >> 8) as u8);
                            20 // 16 base + 4 prefix
                        },
                        _ => unreachable!(),
                    }
                } else {
                    match p {
                        0 | 1 | 3 => {
                            // LD A,(BC) / LD A,(DE) / LD A,(nn) - no HL, run unprefixed +4
                            4 + execute_unprefixed(cpu, bus, opcode)
                        },
                        2 => {
                            // LD HL, (nn) -> LD IX/IY, (nn)
                            let addr = cpu.fetch_word(bus);
                            let lo = bus.read(addr) as u16;
                            let hi = bus.read(addr.wrapping_add(1)) as u16;
                            set_index_reg(cpu, is_iy, (hi << 8) | lo);
                            20 // 16 base + 4 prefix
                        },
                        _ => unreachable!(),
                    }
                }
            },
            3 => {
                if p == 2 {
                    // INC/DEC rp[2] -> INC/DEC IX/IY
                    let ir = get_index_reg(cpu, is_iy);
                    if q == 0 {
                        set_index_reg(cpu, is_iy, ir.wrapping_add(1));
                    } else {
                        set_index_reg(cpu, is_iy, ir.wrapping_sub(1));
                    }
                    10 // 6 base + 4 prefix
                } else {
                    // INC/DEC other rp - no HL, run unprefixed +4
                    4 + execute_unprefixed(cpu, bus, opcode)
                }
            },
            4 => {
                // INC r[y] - substitute for y=4(H),5(L),6((HL))
                if y == 4 || y == 5 {
                    // INC IXH/IXL/IYH/IYL
                    let val = get_reg_ix(cpu, bus, y, is_iy, 0);
                    let result = inc_byte(cpu, val);
                    set_reg_ix(cpu, bus, y, is_iy, 0, result);
                    8 // 4 base + 4 prefix
                } else if y == 6 {
                    // INC (IX/IY+d)
                    let d = cpu.fetch(bus);
                    let addr = compute_ix_addr(cpu, is_iy, d);
                    let val = bus.read(addr);
                    let result = inc_byte(cpu, val);
                    bus.write(addr, result);
                    23 // 11 base + 4 prefix + 8 for displacement
                } else {
                    // INC B/C/D/E/A - no HL, run unprefixed +4
                    4 + execute_unprefixed(cpu, bus, opcode)
                }
            },
            5 => {
                // DEC r[y] - substitute for y=4(H),5(L),6((HL))
                if y == 4 || y == 5 {
                    // DEC IXH/IXL/IYH/IYL
                    let val = get_reg_ix(cpu, bus, y, is_iy, 0);
                    let result = dec_byte(cpu, val);
                    set_reg_ix(cpu, bus, y, is_iy, 0, result);
                    8 // 4 base + 4 prefix
                } else if y == 6 {
                    // DEC (IX/IY+d)
                    let d = cpu.fetch(bus);
                    let addr = compute_ix_addr(cpu, is_iy, d);
                    let val = bus.read(addr);
                    let result = dec_byte(cpu, val);
                    bus.write(addr, result);
                    23
                } else {
                    4 + execute_unprefixed(cpu, bus, opcode)
                }
            },
            6 => {
                // LD r[y], n - substitute for y=4(H),5(L),6((HL))
                if y == 4 || y == 5 {
                    // LD IXH/IXL/IYH/IYL, n
                    let n = cpu.fetch(bus);
                    set_reg_ix(cpu, bus, y, is_iy, 0, n);
                    11 // 7 base + 4 prefix
                } else if y == 6 {
                    // LD (IX/IY+d), n - displacement before immediate
                    let d = cpu.fetch(bus);
                    let addr = compute_ix_addr(cpu, is_iy, d);
                    let n = cpu.fetch(bus);
                    bus.write(addr, n);
                    19 // special timing for DD 36 dd nn
                } else {
                    4 + execute_unprefixed(cpu, bus, opcode)
                }
            },
            0 | 7 => {
                // x=0, z=0: NOP, EX AF, DJNZ, JR, JR cc - none involve HL
                // x=0, z=7: RLCA, RRCA, RLA, RRA, DAA, CPL, SCF, CCF - none involve HL
                4 + execute_unprefixed(cpu, bus, opcode)
            },
            _ => unreachable!(),
        },
        1 => {
            // LD r[y], r[z] or HALT
            if y == 6 && z == 6 {
                // HALT - no substitution
                4 + execute_unprefixed(cpu, bus, opcode)
            } else if y == 6 {
                // LD (IX/IY+d), r[z] - destination is (IX+d), source uses NORMAL registers
                let d = cpu.fetch(bus);
                let addr = compute_ix_addr(cpu, is_iy, d);
                let val = get_reg(cpu, bus, z); // Normal register (not IXH/IXL)
                bus.write(addr, val);
                19 // 7 base + 4 prefix + 8 displacement
            } else if z == 6 {
                // LD r[y], (IX/IY+d) - source is (IX+d), destination uses NORMAL registers
                let d = cpu.fetch(bus);
                let addr = compute_ix_addr(cpu, is_iy, d);
                let val = bus.read(addr);
                set_reg(cpu, bus, y, val); // Normal register (not IXH/IXL)
                19
            } else if y == 4 || y == 5 || z == 4 || z == 5 {
                // Both source and dest are H/L type -> use IXH/IXL/IYH/IYL
                let val = get_reg_ix(cpu, bus, z, is_iy, 0);
                set_reg_ix(cpu, bus, y, is_iy, 0, val);
                8 // 4 base + 4 prefix
            } else {
                // Neither involves HL-related registers, execute as unprefixed +4
                4 + execute_unprefixed(cpu, bus, opcode)
            }
        },
        2 => {
            // ALU[y] A, r[z]
            if z == 6 {
                // ALU A, (IX/IY+d)
                let d = cpu.fetch(bus);
                let addr = compute_ix_addr(cpu, is_iy, d);
                let val = bus.read(addr);
                alu_op(cpu, y, val);
                19 // 7 base + 4 prefix + 8 displacement
            } else if z == 4 || z == 5 {
                // ALU A, IXH/IXL/IYH/IYL
                let val = get_reg_ix(cpu, bus, z, is_iy, 0);
                alu_op(cpu, y, val);
                8 // 4 base + 4 prefix
            } else {
                // ALU A, B/C/D/E/A - no HL involvement, run unprefixed +4
                4 + execute_unprefixed(cpu, bus, opcode)
            }
        },
        3 => match z {
            1 => {
                if q == 0 {
                    // POP rp2[p]
                    if p == 2 {
                        // POP IX/IY
                        let val = pop(cpu, bus);
                        set_index_reg(cpu, is_iy, val);
                        14 // 10 base + 4 prefix
                    } else {
                        4 + execute_unprefixed(cpu, bus, opcode)
                    }
                } else {
                    match p {
                        0 => {
                            // RET - no HL
                            4 + execute_unprefixed(cpu, bus, opcode)
                        },
                        1 => {
                            // EXX - not affected by DD/FD
                            4 + execute_unprefixed(cpu, bus, opcode)
                        },
                        2 => {
                            // JP (HL) -> JP (IX/IY)
                            cpu.pc = get_index_reg(cpu, is_iy);
                            8 // 4 base + 4 prefix
                        },
                        3 => {
                            // LD SP, HL -> LD SP, IX/IY
                            cpu.sp = get_index_reg(cpu, is_iy);
                            10 // 6 base + 4 prefix
                        },
                        _ => unreachable!(),
                    }
                }
            },
            3 => {
                match y {
                    4 => {
                        // EX (SP), HL -> EX (SP), IX/IY
                        let lo = bus.read(cpu.sp) as u16;
                        let hi = bus.read(cpu.sp.wrapping_add(1)) as u16;
                        let sp_val = (hi << 8) | lo;
                        let ir = get_index_reg(cpu, is_iy);
                        bus.write(cpu.sp, ir as u8);
                        bus.write(cpu.sp.wrapping_add(1), (ir >> 8) as u8);
                        set_index_reg(cpu, is_iy, sp_val);
                        23 // 19 base + 4 prefix
                    },
                    5 => {
                        // EX DE, HL - NOT affected by DD/FD prefix
                        4 + execute_unprefixed(cpu, bus, opcode)
                    },
                    _ => {
                        // Other x=3,z=3: JP nn, CB prefix, OUT, IN, DI, EI
                        // None of these involve HL
                        4 + execute_unprefixed(cpu, bus, opcode)
                    },
                }
            },
            5 => {
                if q == 0 {
                    // PUSH rp2[p]
                    if p == 2 {
                        // PUSH IX/IY
                        let val = get_index_reg(cpu, is_iy);
                        push(cpu, bus, val);
                        15 // 11 base + 4 prefix
                    } else {
                        4 + execute_unprefixed(cpu, bus, opcode)
                    }
                } else {
                    // CALL nn, DD/FD/ED prefix - these will be handled by chaining above
                    4 + execute_unprefixed(cpu, bus, opcode)
                }
            },
            _ => {
                // z=0 (RET cc), z=2 (JP cc), z=4 (CALL cc), z=6 (ALU imm), z=7 (RST)
                // None of these involve HL
                4 + execute_unprefixed(cpu, bus, opcode)
            },
        },
        _ => unreachable!(),
    }
}

/// DD prefix: IX instructions.
fn execute_dd(cpu: &mut Cpu, bus: &mut Bus) -> u32 {
    execute_dd_fd(cpu, bus, false)
}

// ---------------------------------------------------------------------------
// ED-prefix helper functions
// ---------------------------------------------------------------------------

/// SBC HL, rp - 16-bit subtract with carry
fn sbc_hl(cpu: &mut Cpu, val: u16) {
    let hl = cpu.hl() as u32;
    let v = val as u32;
    let c = if cpu.flag(FLAG_C) { 1u32 } else { 0 };
    let result = hl.wrapping_sub(v).wrapping_sub(c);
    let half = (hl & 0x0FFF).wrapping_sub(v & 0x0FFF).wrapping_sub(c);
    let r16 = result as u16;
    cpu.f = FLAG_N
        | if result > 0xFFFF { FLAG_C } else { 0 }
        | if r16 == 0 { FLAG_Z } else { 0 }
        | ((r16 >> 8) as u8 & (FLAG_S | FLAG_X | FLAG_Y))
        | if half > 0x0FFF { FLAG_H } else { 0 }
        | if ((hl ^ v) & (hl ^ result)) & 0x8000 != 0 { FLAG_PV } else { 0 };
    cpu.set_hl(r16);
}

/// ADC HL, rp - 16-bit add with carry
fn adc_hl(cpu: &mut Cpu, val: u16) {
    let hl = cpu.hl() as u32;
    let v = val as u32;
    let c = if cpu.flag(FLAG_C) { 1u32 } else { 0 };
    let result = hl + v + c;
    let half = (hl & 0x0FFF) + (v & 0x0FFF) + c;
    let r16 = result as u16;
    cpu.f = if result > 0xFFFF { FLAG_C } else { 0 }
        | if r16 == 0 { FLAG_Z } else { 0 }
        | ((r16 >> 8) as u8 & (FLAG_S | FLAG_X | FLAG_Y))
        | if half > 0x0FFF { FLAG_H } else { 0 }
        | if ((hl ^ v ^ 0x10000) & (hl ^ result)) & 0x8000 != 0 { FLAG_PV } else { 0 };
    cpu.set_hl(r16);
}

/// NEG - negate accumulator (A = 0 - A)
fn neg(cpu: &mut Cpu) {
    let val = cpu.a;
    let result = 0u16.wrapping_sub(val as u16);
    cpu.f = FLAG_N
        | if result & 0xFF == 0 { FLAG_Z } else { 0 }
        | ((result as u8) & (FLAG_S | FLAG_X | FLAG_Y))
        | if val != 0 { FLAG_C } else { 0 }
        | if val == 0x80 { FLAG_PV } else { 0 }
        | if (val & 0x0F) != 0 { FLAG_H } else { 0 };
    cpu.a = result as u8;
}

/// RRD - rotate right decimal: low nibble of (HL) -> low nibble of A,
/// low nibble of A -> high nibble of (HL), high nibble of (HL) -> low nibble of (HL)
fn rrd(cpu: &mut Cpu, bus: &mut Bus) {
    let addr = cpu.hl();
    let mem = bus.read(addr);
    let result = (cpu.a << 4) | (mem >> 4);
    cpu.a = (cpu.a & 0xF0) | (mem & 0x0F);
    bus.write(addr, result);
    cpu.f = (cpu.f & FLAG_C) | (cpu.a & (FLAG_S | FLAG_X | FLAG_Y))
        | if cpu.a == 0 { FLAG_Z } else { 0 }
        | if parity_even(cpu.a) { FLAG_PV } else { 0 };
}

/// RLD - rotate left decimal: high nibble of (HL) -> low nibble of A,
/// low nibble of A -> low nibble of (HL), low nibble of (HL) -> high nibble of (HL)
fn rld(cpu: &mut Cpu, bus: &mut Bus) {
    let addr = cpu.hl();
    let mem = bus.read(addr);
    let result = (mem << 4) | (cpu.a & 0x0F);
    cpu.a = (cpu.a & 0xF0) | (mem >> 4);
    bus.write(addr, result);
    cpu.f = (cpu.f & FLAG_C) | (cpu.a & (FLAG_S | FLAG_X | FLAG_Y))
        | if cpu.a == 0 { FLAG_Z } else { 0 }
        | if parity_even(cpu.a) { FLAG_PV } else { 0 };
}

/// LDI - block load increment: (DE) = (HL), DE++, HL++, BC--
fn ldi(cpu: &mut Cpu, bus: &mut Bus) {
    let val = bus.read(cpu.hl());
    bus.write(cpu.de(), val);
    cpu.set_hl(cpu.hl().wrapping_add(1));
    cpu.set_de(cpu.de().wrapping_add(1));
    cpu.set_bc(cpu.bc().wrapping_sub(1));
    let n = val.wrapping_add(cpu.a);
    cpu.f = (cpu.f & (FLAG_C | FLAG_Z | FLAG_S))
        | if cpu.bc() != 0 { FLAG_PV } else { 0 }
        | (n & FLAG_X)
        | if n & 0x02 != 0 { FLAG_Y } else { 0 };
}

/// LDD - block load decrement: (DE) = (HL), DE--, HL--, BC--
fn ldd(cpu: &mut Cpu, bus: &mut Bus) {
    let val = bus.read(cpu.hl());
    bus.write(cpu.de(), val);
    cpu.set_hl(cpu.hl().wrapping_sub(1));
    cpu.set_de(cpu.de().wrapping_sub(1));
    cpu.set_bc(cpu.bc().wrapping_sub(1));
    let n = val.wrapping_add(cpu.a);
    cpu.f = (cpu.f & (FLAG_C | FLAG_Z | FLAG_S))
        | if cpu.bc() != 0 { FLAG_PV } else { 0 }
        | (n & FLAG_X)
        | if n & 0x02 != 0 { FLAG_Y } else { 0 };
}

/// CPI - compare and increment: compare A with (HL), HL++, BC--
fn cpi(cpu: &mut Cpu, bus: &mut Bus) {
    let val = bus.read(cpu.hl());
    let result = cpu.a.wrapping_sub(val);
    let half = (cpu.a & 0x0F).wrapping_sub(val & 0x0F);
    cpu.set_hl(cpu.hl().wrapping_add(1));
    cpu.set_bc(cpu.bc().wrapping_sub(1));
    let n = result.wrapping_sub(if half > 0x0F { 1 } else { 0 });
    cpu.f = (cpu.f & FLAG_C) | FLAG_N
        | if result == 0 { FLAG_Z } else { 0 }
        | (result & FLAG_S)
        | if half > 0x0F { FLAG_H } else { 0 }
        | if cpu.bc() != 0 { FLAG_PV } else { 0 }
        | (n & FLAG_X)
        | if n & 0x02 != 0 { FLAG_Y } else { 0 };
}

/// CPD - compare and decrement: compare A with (HL), HL--, BC--
fn cpd(cpu: &mut Cpu, bus: &mut Bus) {
    let val = bus.read(cpu.hl());
    let result = cpu.a.wrapping_sub(val);
    let half = (cpu.a & 0x0F).wrapping_sub(val & 0x0F);
    cpu.set_hl(cpu.hl().wrapping_sub(1));
    cpu.set_bc(cpu.bc().wrapping_sub(1));
    let n = result.wrapping_sub(if half > 0x0F { 1 } else { 0 });
    cpu.f = (cpu.f & FLAG_C) | FLAG_N
        | if result == 0 { FLAG_Z } else { 0 }
        | (result & FLAG_S)
        | if half > 0x0F { FLAG_H } else { 0 }
        | if cpu.bc() != 0 { FLAG_PV } else { 0 }
        | (n & FLAG_X)
        | if n & 0x02 != 0 { FLAG_Y } else { 0 };
}

/// INI - block input increment: (HL) = IN(C), HL++, B--
fn ini(cpu: &mut Cpu, bus: &mut Bus) {
    let val = bus.io_read(cpu.c);
    bus.write(cpu.hl(), val);
    cpu.set_hl(cpu.hl().wrapping_add(1));
    cpu.b = cpu.b.wrapping_sub(1);
    cpu.f = (cpu.f & FLAG_C) | if cpu.b == 0 { FLAG_Z } else { 0 }
        | FLAG_N | (cpu.b & (FLAG_S | FLAG_X | FLAG_Y));
}

/// IND - block input decrement: (HL) = IN(C), HL--, B--
fn ind(cpu: &mut Cpu, bus: &mut Bus) {
    let val = bus.io_read(cpu.c);
    bus.write(cpu.hl(), val);
    cpu.set_hl(cpu.hl().wrapping_sub(1));
    cpu.b = cpu.b.wrapping_sub(1);
    cpu.f = (cpu.f & FLAG_C) | if cpu.b == 0 { FLAG_Z } else { 0 }
        | FLAG_N | (cpu.b & (FLAG_S | FLAG_X | FLAG_Y));
}

/// OUTI - block output increment: OUT(C) = (HL), HL++, B--
fn outi(cpu: &mut Cpu, bus: &mut Bus) {
    let val = bus.read(cpu.hl());
    cpu.b = cpu.b.wrapping_sub(1);
    bus.io_write(cpu.c, val);
    cpu.set_hl(cpu.hl().wrapping_add(1));
    cpu.f = (cpu.f & FLAG_C) | if cpu.b == 0 { FLAG_Z } else { 0 }
        | FLAG_N | (cpu.b & (FLAG_S | FLAG_X | FLAG_Y));
}

/// OUTD - block output decrement: OUT(C) = (HL), HL--, B--
fn outd(cpu: &mut Cpu, bus: &mut Bus) {
    let val = bus.read(cpu.hl());
    cpu.b = cpu.b.wrapping_sub(1);
    bus.io_write(cpu.c, val);
    cpu.set_hl(cpu.hl().wrapping_sub(1));
    cpu.f = (cpu.f & FLAG_C) | if cpu.b == 0 { FLAG_Z } else { 0 }
        | FLAG_N | (cpu.b & (FLAG_S | FLAG_X | FLAG_Y));
}

/// ED prefix: extended instructions.
fn execute_ed(cpu: &mut Cpu, bus: &mut Bus) -> u32 {
    cpu.increment_r();
    let opcode = cpu.fetch(bus);
    let x = (opcode >> 6) & 0x03;
    let y = (opcode >> 3) & 0x07;
    let z = opcode & 0x07;
    let p = y >> 1;
    let q = y & 0x01;

    match x {
        1 => match z {
            // z=0: IN r[y], (C)
            0 => {
                let val = bus.io_read(cpu.c);
                if y != 6 { set_reg(cpu, bus, y, val); }
                cpu.f = (cpu.f & FLAG_C) | (val & (FLAG_S | FLAG_X | FLAG_Y))
                    | if val == 0 { FLAG_Z } else { 0 }
                    | if parity_even(val) { FLAG_PV } else { 0 };
                12
            }
            // z=1: OUT (C), r[y]
            1 => {
                let val = if y == 6 { 0 } else { get_reg(cpu, bus, y) };
                bus.io_write(cpu.c, val);
                12
            }
            // z=2: SBC HL, rp[p] (q=0) / ADC HL, rp[p] (q=1)
            2 => {
                let val = get_rp(cpu, p);
                if q == 0 {
                    sbc_hl(cpu, val);
                } else {
                    adc_hl(cpu, val);
                }
                15
            }
            // z=3: LD (nn), rp[p] (q=0) / LD rp[p], (nn) (q=1)
            3 => {
                let addr = cpu.fetch_word(bus);
                if q == 0 {
                    let val = get_rp(cpu, p);
                    bus.write(addr, val as u8);
                    bus.write(addr.wrapping_add(1), (val >> 8) as u8);
                } else {
                    let lo = bus.read(addr) as u16;
                    let hi = bus.read(addr.wrapping_add(1)) as u16;
                    set_rp(cpu, p, (hi << 8) | lo);
                }
                20
            }
            // z=4: NEG (all y values)
            4 => {
                neg(cpu);
                8
            }
            // z=5: RETN (y!=1) / RETI (y=1)
            5 => {
                cpu.iff1 = cpu.iff2;
                cpu.pc = pop(cpu, bus);
                14
            }
            // z=6: IM n
            6 => {
                match y {
                    0 | 4 => cpu.im = 0,
                    1 | 5 => cpu.im = 0,
                    2 | 6 => cpu.im = 1,
                    3 | 7 => cpu.im = 2,
                    _ => unreachable!(),
                }
                8
            }
            // z=7: Special register operations
            7 => {
                match y {
                    0 => { cpu.i = cpu.a; 9 } // LD I, A
                    1 => { cpu.r = cpu.a; 9 } // LD R, A
                    2 => { // LD A, I
                        cpu.a = cpu.i;
                        cpu.f = (cpu.f & FLAG_C) | (cpu.a & (FLAG_S | FLAG_X | FLAG_Y))
                            | if cpu.a == 0 { FLAG_Z } else { 0 }
                            | if cpu.iff2 { FLAG_PV } else { 0 };
                        9
                    }
                    3 => { // LD A, R
                        cpu.a = cpu.r;
                        cpu.f = (cpu.f & FLAG_C) | (cpu.a & (FLAG_S | FLAG_X | FLAG_Y))
                            | if cpu.a == 0 { FLAG_Z } else { 0 }
                            | if cpu.iff2 { FLAG_PV } else { 0 };
                        9
                    }
                    4 => { rrd(cpu, bus); 18 } // RRD
                    5 => { rld(cpu, bus); 18 } // RLD
                    6 | 7 => { 8 } // NOP
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        },
        2 => {
            // Block instructions: only valid when z <= 3 AND y >= 4
            if z <= 3 && y >= 4 {
                match z {
                    // z=0: LDI/LDD/LDIR/LDDR
                    0 => match y {
                        4 => { ldi(cpu, bus); 16 }  // LDI
                        5 => { ldd(cpu, bus); 16 }  // LDD
                        6 => { // LDIR
                            ldi(cpu, bus);
                            if cpu.bc() != 0 {
                                cpu.pc = cpu.pc.wrapping_sub(2);
                                21
                            } else {
                                16
                            }
                        }
                        7 => { // LDDR
                            ldd(cpu, bus);
                            if cpu.bc() != 0 {
                                cpu.pc = cpu.pc.wrapping_sub(2);
                                21
                            } else {
                                16
                            }
                        }
                        _ => 8, // NOP (shouldn't reach here due to y >= 4 check)
                    },
                    // z=1: CPI/CPD/CPIR/CPDR
                    1 => match y {
                        4 => { cpi(cpu, bus); 16 }  // CPI
                        5 => { cpd(cpu, bus); 16 }  // CPD
                        6 => { // CPIR
                            cpi(cpu, bus);
                            if cpu.bc() != 0 && !cpu.flag(FLAG_Z) {
                                cpu.pc = cpu.pc.wrapping_sub(2);
                                21
                            } else {
                                16
                            }
                        }
                        7 => { // CPDR
                            cpd(cpu, bus);
                            if cpu.bc() != 0 && !cpu.flag(FLAG_Z) {
                                cpu.pc = cpu.pc.wrapping_sub(2);
                                21
                            } else {
                                16
                            }
                        }
                        _ => 8,
                    },
                    // z=2: INI/IND/INIR/INDR
                    2 => match y {
                        4 => { ini(cpu, bus); 16 }  // INI
                        5 => { ind(cpu, bus); 16 }  // IND
                        6 => { // INIR
                            ini(cpu, bus);
                            if cpu.b != 0 {
                                cpu.pc = cpu.pc.wrapping_sub(2);
                                21
                            } else {
                                16
                            }
                        }
                        7 => { // INDR
                            ind(cpu, bus);
                            if cpu.b != 0 {
                                cpu.pc = cpu.pc.wrapping_sub(2);
                                21
                            } else {
                                16
                            }
                        }
                        _ => 8,
                    },
                    // z=3: OUTI/OUTD/OTIR/OTDR
                    3 => match y {
                        4 => { outi(cpu, bus); 16 } // OUTI
                        5 => { outd(cpu, bus); 16 } // OUTD
                        6 => { // OTIR
                            outi(cpu, bus);
                            if cpu.b != 0 {
                                cpu.pc = cpu.pc.wrapping_sub(2);
                                21
                            } else {
                                16
                            }
                        }
                        7 => { // OTDR
                            outd(cpu, bus);
                            if cpu.b != 0 {
                                cpu.pc = cpu.pc.wrapping_sub(2);
                                21
                            } else {
                                16
                            }
                        }
                        _ => 8,
                    },
                    _ => unreachable!(),
                }
            } else {
                8 // NOP for invalid block instruction encodings
            }
        }
        // x=0 and x=3: all NOPs
        _ => 8,
    }
}

/// FD prefix: IY instructions.
fn execute_fd(cpu: &mut Cpu, bus: &mut Bus) -> u32 {
    execute_dd_fd(cpu, bus, true)
}

// ---------------------------------------------------------------------------
// Main unprefixed opcode execution
// ---------------------------------------------------------------------------

/// Decode and execute a single unprefixed opcode, returning the cycle count.
pub fn execute_unprefixed(cpu: &mut Cpu, bus: &mut Bus, opcode: u8) -> u32 {
    let x = (opcode >> 6) & 0x03;
    let y = (opcode >> 3) & 0x07;
    let z = opcode & 0x07;
    let p = y >> 1;
    let q = y & 0x01;

    match x {
        0 => match z {
            0 => match y {
                // NOP
                0 => 4,
                // EX AF, AF'
                1 => {
                    cpu.ex_af_af_prime();
                    4
                },
                // DJNZ d
                2 => {
                    let d = cpu.fetch(bus) as i8;
                    cpu.b = cpu.b.wrapping_sub(1);
                    if cpu.b != 0 {
                        cpu.pc = (cpu.pc as i16).wrapping_add(d as i16) as u16;
                        13
                    } else {
                        8
                    }
                },
                // JR d
                3 => {
                    let d = cpu.fetch(bus) as i8;
                    cpu.pc = (cpu.pc as i16).wrapping_add(d as i16) as u16;
                    12
                },
                // JR cc[y-4], d
                4..=7 => {
                    let d = cpu.fetch(bus) as i8;
                    if check_condition(cpu, y - 4) {
                        cpu.pc = (cpu.pc as i16).wrapping_add(d as i16) as u16;
                        12
                    } else {
                        7
                    }
                },
                _ => unreachable!(),
            },
            1 => {
                if q == 0 {
                    // LD rp[p], nn
                    let val = cpu.fetch_word(bus);
                    set_rp(cpu, p, val);
                    10
                } else {
                    // ADD HL, rp[p]
                    let val = get_rp(cpu, p);
                    add_hl(cpu, val);
                    11
                }
            },
            2 => {
                if q == 0 {
                    match p {
                        // LD (BC), A
                        0 => {
                            bus.write(cpu.bc(), cpu.a);
                            7
                        },
                        // LD (DE), A
                        1 => {
                            bus.write(cpu.de(), cpu.a);
                            7
                        },
                        // LD (nn), HL
                        2 => {
                            let addr = cpu.fetch_word(bus);
                            let hl = cpu.hl();
                            bus.write(addr, hl as u8);
                            bus.write(addr.wrapping_add(1), (hl >> 8) as u8);
                            16
                        },
                        // LD (nn), A
                        3 => {
                            let addr = cpu.fetch_word(bus);
                            bus.write(addr, cpu.a);
                            13
                        },
                        _ => unreachable!(),
                    }
                } else {
                    match p {
                        // LD A, (BC)
                        0 => {
                            cpu.a = bus.read(cpu.bc());
                            7
                        },
                        // LD A, (DE)
                        1 => {
                            cpu.a = bus.read(cpu.de());
                            7
                        },
                        // LD HL, (nn)
                        2 => {
                            let addr = cpu.fetch_word(bus);
                            let lo = bus.read(addr) as u16;
                            let hi = bus.read(addr.wrapping_add(1)) as u16;
                            cpu.set_hl((hi << 8) | lo);
                            16
                        },
                        // LD A, (nn)
                        3 => {
                            let addr = cpu.fetch_word(bus);
                            cpu.a = bus.read(addr);
                            13
                        },
                        _ => unreachable!(),
                    }
                }
            },
            3 => {
                if q == 0 {
                    // INC rp[p]
                    let val = get_rp(cpu, p).wrapping_add(1);
                    set_rp(cpu, p, val);
                    6
                } else {
                    // DEC rp[p]
                    let val = get_rp(cpu, p).wrapping_sub(1);
                    set_rp(cpu, p, val);
                    6
                }
            },
            4 => {
                // INC r[y]
                let val = get_reg(cpu, bus, y);
                let result = inc_byte(cpu, val);
                set_reg(cpu, bus, y, result);
                if y == 6 { 11 } else { 4 }
            },
            5 => {
                // DEC r[y]
                let val = get_reg(cpu, bus, y);
                let result = dec_byte(cpu, val);
                set_reg(cpu, bus, y, result);
                if y == 6 { 11 } else { 4 }
            },
            6 => {
                // LD r[y], n
                let n = cpu.fetch(bus);
                set_reg(cpu, bus, y, n);
                if y == 6 { 10 } else { 7 }
            },
            7 => match y {
                // RLCA
                0 => {
                    let carry = cpu.a >> 7;
                    cpu.a = (cpu.a << 1) | carry;
                    cpu.f = (cpu.f & (FLAG_Z | FLAG_PV | FLAG_S))
                        | (cpu.a & (FLAG_X | FLAG_Y))
                        | carry;
                    4
                },
                // RRCA
                1 => {
                    let carry = cpu.a & 1;
                    cpu.a = (cpu.a >> 1) | (carry << 7);
                    cpu.f = (cpu.f & (FLAG_Z | FLAG_PV | FLAG_S))
                        | (cpu.a & (FLAG_X | FLAG_Y))
                        | carry;
                    4
                },
                // RLA
                2 => {
                    let old_carry = if cpu.flag(FLAG_C) { 1 } else { 0 };
                    let new_carry = cpu.a >> 7;
                    cpu.a = (cpu.a << 1) | old_carry;
                    cpu.f = (cpu.f & (FLAG_Z | FLAG_PV | FLAG_S))
                        | (cpu.a & (FLAG_X | FLAG_Y))
                        | new_carry;
                    4
                },
                // RRA
                3 => {
                    let old_carry = if cpu.flag(FLAG_C) { 1 } else { 0 };
                    let new_carry = cpu.a & 1;
                    cpu.a = (cpu.a >> 1) | (old_carry << 7);
                    cpu.f = (cpu.f & (FLAG_Z | FLAG_PV | FLAG_S))
                        | (cpu.a & (FLAG_X | FLAG_Y))
                        | new_carry;
                    4
                },
                // DAA
                4 => {
                    daa(cpu);
                    4
                },
                // CPL
                5 => {
                    cpu.a = !cpu.a;
                    cpu.f = (cpu.f & (FLAG_C | FLAG_PV | FLAG_Z | FLAG_S))
                        | FLAG_H | FLAG_N
                        | (cpu.a & (FLAG_X | FLAG_Y));
                    4
                },
                // SCF
                6 => {
                    cpu.f = (cpu.f & (FLAG_Z | FLAG_PV | FLAG_S))
                        | FLAG_C
                        | (cpu.a & (FLAG_X | FLAG_Y));
                    4
                },
                // CCF
                7 => {
                    let old_carry = if cpu.flag(FLAG_C) { FLAG_H } else { 0 };
                    cpu.f = (cpu.f & (FLAG_Z | FLAG_PV | FLAG_S))
                        | old_carry
                        | (if cpu.flag(FLAG_C) { 0 } else { FLAG_C })
                        | (cpu.a & (FLAG_X | FLAG_Y));
                    4
                },
                _ => unreachable!(),
            },
            _ => unreachable!(),
        },
        1 => {
            if z == 6 && y == 6 {
                // HALT
                cpu.halted = true;
                4
            } else {
                // LD r[y], r[z]
                let val = get_reg(cpu, bus, z);
                set_reg(cpu, bus, y, val);
                if y == 6 || z == 6 { 7 } else { 4 }
            }
        },
        2 => {
            // ALU[y] A, r[z]
            let val = get_reg(cpu, bus, z);
            alu_op(cpu, y, val);
            if z == 6 { 7 } else { 4 }
        },
        3 => match z {
            0 => {
                // RET cc[y]
                if check_condition(cpu, y) {
                    cpu.pc = pop(cpu, bus);
                    11
                } else {
                    5
                }
            },
            1 => {
                if q == 0 {
                    // POP rp2[p]
                    let val = pop(cpu, bus);
                    set_rp2(cpu, p, val);
                    10
                } else {
                    match p {
                        0 => {
                            // RET
                            cpu.pc = pop(cpu, bus);
                            10
                        },
                        1 => {
                            // EXX
                            cpu.exx();
                            4
                        },
                        2 => {
                            // JP (HL)
                            cpu.pc = cpu.hl();
                            4
                        },
                        3 => {
                            // LD SP, HL
                            cpu.sp = cpu.hl();
                            6
                        },
                        _ => unreachable!(),
                    }
                }
            },
            2 => {
                // JP cc[y], nn
                let addr = cpu.fetch_word(bus);
                if check_condition(cpu, y) {
                    cpu.pc = addr;
                }
                10
            },
            3 => match y {
                0 => {
                    // JP nn
                    let addr = cpu.fetch_word(bus);
                    cpu.pc = addr;
                    10
                },
                1 => {
                    // CB prefix
                    execute_cb(cpu, bus)
                },
                2 => {
                    // OUT (n), A
                    let port = cpu.fetch(bus);
                    bus.io_write(port, cpu.a);
                    11
                },
                3 => {
                    // IN A, (n)
                    let port = cpu.fetch(bus);
                    cpu.a = bus.io_read(port);
                    11
                },
                4 => {
                    // EX (SP), HL
                    let lo = bus.read(cpu.sp) as u16;
                    let hi = bus.read(cpu.sp.wrapping_add(1)) as u16;
                    let sp_val = (hi << 8) | lo;
                    let hl = cpu.hl();
                    bus.write(cpu.sp, hl as u8);
                    bus.write(cpu.sp.wrapping_add(1), (hl >> 8) as u8);
                    cpu.set_hl(sp_val);
                    19
                },
                5 => {
                    // EX DE, HL
                    let de = cpu.de();
                    let hl = cpu.hl();
                    cpu.set_de(hl);
                    cpu.set_hl(de);
                    4
                },
                6 => {
                    // DI
                    cpu.iff1 = false;
                    cpu.iff2 = false;
                    4
                },
                7 => {
                    // EI
                    cpu.ei_pending = true;
                    4
                },
                _ => unreachable!(),
            },
            4 => {
                // CALL cc[y], nn
                let addr = cpu.fetch_word(bus);
                if check_condition(cpu, y) {
                    push(cpu, bus, cpu.pc);
                    cpu.pc = addr;
                    17
                } else {
                    10
                }
            },
            5 => {
                if q == 0 {
                    // PUSH rp2[p]
                    let val = get_rp2(cpu, p);
                    push(cpu, bus, val);
                    11
                } else {
                    match p {
                        0 => {
                            // CALL nn
                            let addr = cpu.fetch_word(bus);
                            push(cpu, bus, cpu.pc);
                            cpu.pc = addr;
                            17
                        },
                        1 => {
                            // DD prefix (IX instructions)
                            execute_dd(cpu, bus)
                        },
                        2 => {
                            // ED prefix
                            execute_ed(cpu, bus)
                        },
                        3 => {
                            // FD prefix (IY instructions)
                            execute_fd(cpu, bus)
                        },
                        _ => unreachable!(),
                    }
                }
            },
            6 => {
                // ALU[y] A, n
                let n = cpu.fetch(bus);
                alu_op(cpu, y, n);
                7
            },
            7 => {
                // RST y*8
                push(cpu, bus, cpu.pc);
                cpu.pc = (y as u16) * 8;
                11
            },
            _ => unreachable!(),
        },
        _ => unreachable!(),
    }
}
