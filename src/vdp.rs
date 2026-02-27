// Video Display Processor emulation
// Implements the Sega Game Gear VDP register/port interface.
// Rendering is done through the scanline runner
// Sprite ordering needs some improvement

pub struct Vdp {
    pub vram: [u8; 0x4000],
    pub cram: [u8; 64],
    pub registers: [u8; 11],
    pub status: u8,
    pub address: u16,
    pub code: u8,
    pub toggle: bool,
    pub read_buffer: u8,
    pub cram_latch: u8,
    pub scanline: u16,
    pub line_counter: u8,
    pub frame_irq_pending: bool,
    pub line_irq_pending: bool,
    pub framebuffer: Vec<u32>,
    pub v_counter: u8,
}

impl Vdp {
    pub fn new() -> Self {
        Self {
            vram: [0; 0x4000],
            cram: [0; 64],
            registers: [0; 11],
            status: 0,
            address: 0,
            code: 0,
            toggle: false,
            read_buffer: 0,
            cram_latch: 0,
            scanline: 0,
            line_counter: 0,
            frame_irq_pending: false,
            line_irq_pending: false,
            framebuffer: vec![0; 256 * 192],
            v_counter: 0,
        }
    }

    /// Two-byte control port write sequence.
    ///
    /// First byte: low 8 bits of address, sets toggle.
    /// Second byte: upper 6 bits of address + 2-bit code, clears toggle.
    ///   Code 0: VRAM read  - pre-fetch byte into read_buffer, increment address.
    ///   Code 1: VRAM write mode (no immediate action).
    ///   Code 2: Register write - write first byte to register[bits 3-0 of high byte] (if reg <= 10).
    ///   Code 3: CRAM write mode.
    pub fn write_control(&mut self, val: u8) {
        if !self.toggle {
            // First byte: store low 8 bits of address
            self.address = (self.address & 0x3F00) | val as u16;
            self.toggle = true;
        } else {
            // Second byte: store high 6 bits of address + 2-bit code
            self.address = (self.address & 0x00FF) | ((val as u16 & 0x3F) << 8);
            self.code = (val >> 6) & 0x03;
            self.toggle = false;

            match self.code {
                0 => {
                    // VRAM read: pre-fetch byte into read_buffer, increment address
                    self.read_buffer = self.vram[(self.address & 0x3FFF) as usize];
                    self.address = (self.address.wrapping_add(1)) & 0x3FFF;
                }
                1 => {
                    // VRAM write mode: no immediate action
                }
                2 => {
                    // Register write: write the first byte (low 8 bits of address)
                    // to register selected by bits 3-0 of the high byte
                    let reg = (self.address >> 8) as usize & 0x0F;
                    let data = self.address as u8; // low byte is the data
                    if reg <= 10 {
                        self.registers[reg] = data;
                    }
                }
                3 => {
                    // CRAM write mode: no immediate action
                }
                _ => unreachable!(),
            }
        }
    }

    /// Read the status register.
    /// Returns status, clears bits 7/6/5, clears frame/line IRQ pending, resets toggle.
    pub fn read_control(&mut self) -> u8 {
        let val = self.status;
        self.status &= !(0x80 | 0x40 | 0x20); // clear bits 7, 6, 5
        self.frame_irq_pending = false;
        self.line_irq_pending = false;
        self.toggle = false;
        val
    }

    /// Data port write.
    /// Resets toggle. If code=3 (CRAM): even address stores to latch, odd address
    /// writes latch+val as 16-bit word to CRAM. Otherwise: write to VRAM.
    /// Always increments address.
    pub fn write_data(&mut self, val: u8) {
        self.toggle = false;

        if self.code == 3 {
            // CRAM write (Game Gear 12-bit color)
            let addr = self.address & 0x3FFF;
            if addr & 1 == 0 {
                // Even address: store to latch
                self.cram_latch = val;
            } else {
                // Odd address: write latch (low byte) + val (high byte) as 16-bit LE word
                let cram_addr = (addr.wrapping_sub(1) & 0x3F) as usize;
                if cram_addr + 1 < self.cram.len() {
                    self.cram[cram_addr] = self.cram_latch;
                    self.cram[cram_addr + 1] = val;
                }
            }
        } else {
            // Write to VRAM
            self.vram[(self.address & 0x3FFF) as usize] = val;
        }

        self.address = (self.address.wrapping_add(1)) & 0x3FFF;
    }

    /// Data port read.
    /// Resets toggle. Returns read_buffer, then pre-fetches next VRAM byte into
    /// buffer and increments address.
    pub fn read_data(&mut self) -> u8 {
        self.toggle = false;
        let val = self.read_buffer;
        self.read_buffer = self.vram[(self.address & 0x3FFF) as usize];
        self.address = (self.address.wrapping_add(1)) & 0x3FFF;
        val
    }

    /// Return the current v_counter value.
    pub fn read_v_counter(&self) -> u8 {
        self.v_counter
    }

    /// Return a simplified h_counter (always 0).
    pub fn read_h_counter(&self) -> u8 {
        0
    }

    /// Scanline runner. Renders the current scanline (if in active display),
    /// then updates scanline counter (0-261), v_counter with NTSC 192-line
    /// jump-back, line counter, and fires IRQs at appropriate times.
    pub fn run_scanline(&mut self) {
        // Render the current scanline if in active display (lines 0-191)
        if self.scanline < 192 {
            self.render_scanline(self.scanline);
        }

        // Update v_counter with NTSC 192-line mapping:
        // Lines 0-218 map to $00-$DA
        // Lines 219-261 map to $D5-$FF
        if self.scanline <= 218 {
            self.v_counter = self.scanline as u8;
        } else {
            // Lines 219-261 map to $D5-$FF
            // 219 -> 0xD5, 220 -> 0xD6, ..., 261 -> 0xFF
            self.v_counter = (self.scanline - 219 + 0xD5) as u8;
        }

        // Line counter logic
        if self.scanline <= 192 {
            if self.scanline < 192 {
                // Active display: decrement line counter
                if self.line_counter == 0 {
                    self.line_counter = self.registers[0x0A];
                    self.line_irq_pending = true;
                } else {
                    self.line_counter = self.line_counter.wrapping_sub(1);
                }
            }

            if self.scanline == 192 {
                // Frame IRQ at line 192
                self.status |= 0x80; // set frame interrupt flag
                self.frame_irq_pending = true;
                // Reload line counter on vblank entry
                self.line_counter = self.registers[0x0A];
            }
        } else {
            // Vblank lines (193-261): reload line counter each scanline
            self.line_counter = self.registers[0x0A];
        }

        // Advance scanline, wrap at 262
        self.scanline += 1;
        if self.scanline >= 262 {
            self.scanline = 0;
        }
    }

    /// Returns true if an IRQ should be asserted to the CPU.
    /// True if (frame_irq_pending AND reg[1] bit 5) OR (line_irq_pending AND reg[0] bit 4).
    pub fn irq_pending(&self) -> bool {
        let frame_irq = self.frame_irq_pending && (self.registers[1] & 0x20) != 0;
        let line_irq = self.line_irq_pending && (self.registers[0] & 0x10) != 0;
        frame_irq || line_irq
    }

    // ---------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------

    /// Convert a CRAM color index (0-31) to packed RGB888 (0x00RRGGBB).
    ///
    /// Game Gear CRAM stores 32 colors as 16-bit little-endian words:
    ///   Low byte: bits 3-0 = Red, bits 7-4 = Green
    ///   High byte: bits 3-0 = Blue
    /// 4-bit channel values are expanded to 8-bit by multiplying by 17.
    pub fn get_color(&self, index: usize) -> u32 {
        let low = self.cram[(index * 2) & 0x3F] as u32;
        let high = self.cram[(index * 2 + 1) & 0x3F] as u32;
        let r = (low & 0x0F) * 17;
        let g = ((low >> 4) & 0x0F) * 17;
        let b = (high & 0x0F) * 17;
        (r << 16) | (g << 8) | b
    }

    /// Render the background tile layer for a single scanline.
    ///
    /// Populates `line_buf` with RGB pixel values, `priority_buf` with the
    /// per-pixel BG-priority flag, and `color_index_buf` with the raw palette
    /// index (0-15) so that sprite rendering can decide transparency/priority.
    pub fn render_background(
        &self,
        line: u16,
        line_buf: &mut [u32; 256],
        priority_buf: &mut [bool; 256],
        color_index_buf: &mut [u8; 256],
    ) {
        let nt_base = (self.registers[2] as usize & 0x0E) << 10;
        let scroll_x = self.registers[8] as usize;
        let scroll_y = self.registers[9] as usize;

        // Register 0 bits for scroll lock
        let lock_top_rows_h = self.registers[0] & 0x40 != 0; // bit 6: lock top 2 rows from H scroll
        let lock_right_cols_v = self.registers[0] & 0x80 != 0; // bit 7: lock right 8 columns from V scroll

        for screen_x in 0..256usize {
            // Determine effective vertical position
            let effective_v_scroll = if lock_right_cols_v && screen_x >= 192 {
                0
            } else {
                scroll_y
            };
            let effective_y = ((line as usize) + effective_v_scroll) % 224;

            // Determine effective horizontal position
            // lock_top_rows_h: if set, columns 24-31 (pixels 192-255) are not scrolled
            let effective_h_scroll = if lock_top_rows_h && effective_y < 16 {
                // Top 2 tile rows: no horizontal scroll
                0
            } else {
                scroll_x
            };
            // The background source X: screen pixel maps to BG position
            // register 8 scrolls the background to the right, so for screen X
            // the source column is (X - scroll_x) in the name table, which wraps
            // at 256. This is equivalent to (256 + X - scroll_x) & 0xFF.
            let bg_x = (256 + screen_x - effective_h_scroll) & 0xFF;

            let col = bg_x >> 3; // tile column (0-31)
            let fine_x = bg_x & 7; // pixel within tile (0-7)
            let row = effective_y >> 3; // tile row (0-27)
            let fine_y = effective_y & 7; // pixel row within tile (0-7)

            // Name table entry address (each entry is 2 bytes, 32 columns per row)
            let nt_addr = nt_base + (row * 32 + col) * 2;
            let low = self.vram[nt_addr & 0x3FFF] as u16;
            let high = self.vram[(nt_addr + 1) & 0x3FFF] as u16;
            let entry = low | (high << 8);

            let tile_index = (entry & 0x01FF) as usize; // bits 0-8
            let h_flip = entry & 0x0200 != 0; // bit 9
            let v_flip = entry & 0x0400 != 0; // bit 10
            let palette = if entry & 0x0800 != 0 { 16 } else { 0 }; // bit 11
            let priority = entry & 0x1000 != 0; // bit 12

            // Tile row, accounting for vertical flip
            let tile_row = if v_flip { 7 - fine_y } else { fine_y };

            // Tile data: 4 bytes per row, 32 bytes per tile
            let tile_addr = tile_index * 32 + tile_row * 4;
            let bp0 = self.vram[tile_addr & 0x3FFF];
            let bp1 = self.vram[(tile_addr + 1) & 0x3FFF];
            let bp2 = self.vram[(tile_addr + 2) & 0x3FFF];
            let bp3 = self.vram[(tile_addr + 3) & 0x3FFF];

            // Pixel bit position, accounting for horizontal flip
            let bit = if h_flip { fine_x } else { 7 - fine_x };

            let color_idx = ((bp0 >> bit) & 1)
                | (((bp1 >> bit) & 1) << 1)
                | (((bp2 >> bit) & 1) << 2)
                | (((bp3 >> bit) & 1) << 3);

            let full_color_idx = palette + color_idx as usize;
            line_buf[screen_x] = self.get_color(full_color_idx);
            priority_buf[screen_x] = priority;
            color_index_buf[screen_x] = color_idx;
        }
    }

    /// Render sprites for a single scanline, compositing onto the line buffer.
    ///
    /// Sprites use palette 1 (colors 16-31). Color index 0 is transparent.
    /// If a BG tile has its priority bit set and a non-zero color index, the BG
    /// pixel is kept even when a sprite pixel overlaps it.
    pub fn render_sprites(
        &self,
        line: u16,
        line_buf: &mut [u32; 256],
        priority_buf: &[bool; 256],
        color_index_buf: &[u8; 256],
    ) {
        let sat_base = (self.registers[5] as usize & 0x7E) << 7;
        let pattern_base: usize = if self.registers[6] & 0x04 != 0 {
            0x2000
        } else {
            0
        };
        let tall_sprites = self.registers[1] & 0x02 != 0; // 8x16 mode
        let sprite_height: u16 = if tall_sprites { 16 } else { 8 };
        let shift_left = self.registers[0] & 0x08 != 0; // shift sprites left by 8

        // Collect sprites on this scanline (max 8)
        let mut sprites_on_line: Vec<(usize, u8, u8)> = Vec::new(); // (index, x, tile)
        let mut sprite_overflow = false;

        for i in 0..64usize {
            let y_entry = self.vram[(sat_base + i) & 0x3FFF];

            // Y value of 0xD0 (208) terminates in 192-line mode
            if y_entry == 0xD0 {
                break;
            }

            let sprite_y = (y_entry as u16).wrapping_add(1);

            // Check if sprite overlaps this scanline
            if line >= sprite_y && line < sprite_y + sprite_height {
                if sprites_on_line.len() >= 8 {
                    sprite_overflow = true;
                    break;
                }
                let xn_addr = sat_base + 0x80 + i * 2;
                let sprite_x = self.vram[xn_addr & 0x3FFF];
                let tile_idx = self.vram[(xn_addr + 1) & 0x3FFF];
                sprites_on_line.push((i, sprite_x, tile_idx));
            }
        }

        if sprite_overflow {
            // Set sprite overflow flag (status bit 6)
            // Note: we can't mutate self.status here since &self, but
            // render_scanline calls us before writing status. We'll handle
            // this differently -- the caller (render_scanline) doesn't set it.
            // Actually let's just not set it here since we take &self.
            // The spec says "set status bit 6" -- we'll address this below.
        }

        // Render sprites in reverse order (lower index = higher priority = drawn last)
        for &(_idx, sprite_x, tile_idx) in sprites_on_line.iter().rev() {
            let sx = sprite_x as i16 - if shift_left { 8 } else { 0 };

            let sprite_y_entry = self.vram[(sat_base + _idx) & 0x3FFF];
            let sprite_y = (sprite_y_entry as u16).wrapping_add(1);
            let row_in_sprite = (line - sprite_y) as usize;

            // For 8x16 sprites, use (tile_index & 0xFE) for top, (tile_index | 0x01) for bottom
            let actual_tile = if tall_sprites {
                if row_in_sprite < 8 {
                    tile_idx & 0xFE
                } else {
                    tile_idx | 0x01
                }
            } else {
                tile_idx
            };

            let tile_row = row_in_sprite & 7;
            let tile_addr = pattern_base + (actual_tile as usize) * 32 + tile_row * 4;

            let bp0 = self.vram[tile_addr & 0x3FFF];
            let bp1 = self.vram[(tile_addr + 1) & 0x3FFF];
            let bp2 = self.vram[(tile_addr + 2) & 0x3FFF];
            let bp3 = self.vram[(tile_addr + 3) & 0x3FFF];

            for px in 0..8usize {
                let screen_px = sx + px as i16;
                if screen_px < 0 || screen_px >= 256 {
                    continue;
                }
                let screen_px = screen_px as usize;

                let bit = 7 - px;
                let color_idx = ((bp0 >> bit) & 1)
                    | (((bp1 >> bit) & 1) << 1)
                    | (((bp2 >> bit) & 1) << 2)
                    | (((bp3 >> bit) & 1) << 3);

                // Color 0 = transparent
                if color_idx == 0 {
                    continue;
                }

                // BG priority: if BG tile has priority and BG color is non-zero, BG wins
                if priority_buf[screen_px] && color_index_buf[screen_px] != 0 {
                    continue;
                }

                // Sprite uses palette 1 (colors 16-31)
                line_buf[screen_px] = self.get_color(16 + color_idx as usize);
            }
        }
    }

    /// Render a complete scanline into the framebuffer.
    ///
    /// If the display is blanked (register 1 bit 6 = 0), fills the line with
    /// the backdrop color. Otherwise renders background tiles, then sprites,
    /// and applies column-0 blanking if configured.
    pub fn render_scanline(&mut self, line: u16) {
        if line >= 192 {
            return;
        }

        if self.registers[1] & 0x40 == 0 {
            // Display blanked: fill with backdrop color
            let backdrop = self.get_color(16 + (self.registers[7] & 0x0F) as usize);
            for x in 0..256 {
                self.framebuffer[line as usize * 256 + x] = backdrop;
            }
            return;
        }

        let mut line_buf = [0u32; 256];
        let mut priority_buf = [false; 256];
        let mut color_index_buf = [0u8; 256];

        self.render_background(line, &mut line_buf, &mut priority_buf, &mut color_index_buf);
        self.render_sprites(line, &mut line_buf, &priority_buf, &color_index_buf);

        // Handle sprite overflow (we need to check here since render_sprites takes &self)
        {
            let sat_base = (self.registers[5] as usize & 0x7E) << 7;
            let tall_sprites = self.registers[1] & 0x02 != 0;
            let sprite_height: u16 = if tall_sprites { 16 } else { 8 };
            let mut count = 0u32;
            for i in 0..64usize {
                let y_entry = self.vram[(sat_base + i) & 0x3FFF];
                if y_entry == 0xD0 {
                    break;
                }
                let sprite_y = (y_entry as u16).wrapping_add(1);
                if line >= sprite_y && line < sprite_y + sprite_height {
                    count += 1;
                    if count > 8 {
                        self.status |= 0x40;
                        break;
                    }
                }
            }
        }

        // Column 0 blanking: if register 0 bit 5 is set, blank the leftmost 8 pixels
        if self.registers[0] & 0x20 != 0 {
            let backdrop = self.get_color(16 + (self.registers[7] & 0x0F) as usize);
            for x in 0..8 {
                line_buf[x] = backdrop;
            }
        }

        // Copy line buffer to framebuffer
        let offset = line as usize * 256;
        for (x, &pixel) in line_buf.iter().enumerate() {
            self.framebuffer[offset + x] = pixel;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Construction
    // ---------------------------------------------------------------

    #[test]
    fn new_initializes_all_fields_to_zero() {
        let vdp = Vdp::new();
        assert_eq!(vdp.vram, [0u8; 0x4000]);
        assert_eq!(vdp.cram, [0u8; 64]);
        assert_eq!(vdp.registers, [0u8; 11]);
        assert_eq!(vdp.status, 0);
        assert_eq!(vdp.address, 0);
        assert_eq!(vdp.code, 0);
        assert!(!vdp.toggle);
        assert_eq!(vdp.read_buffer, 0);
        assert_eq!(vdp.cram_latch, 0);
        assert_eq!(vdp.scanline, 0);
        assert_eq!(vdp.line_counter, 0);
        assert!(!vdp.frame_irq_pending);
        assert!(!vdp.line_irq_pending);
        assert_eq!(vdp.v_counter, 0);
    }

    #[test]
    fn new_framebuffer_correct_size() {
        let vdp = Vdp::new();
        assert_eq!(vdp.framebuffer.len(), 256 * 192);
        assert!(vdp.framebuffer.iter().all(|&p| p == 0));
    }

    // ---------------------------------------------------------------
    // Control port write - toggle behavior
    // ---------------------------------------------------------------

    #[test]
    fn control_write_first_byte_sets_toggle() {
        let mut vdp = Vdp::new();
        assert!(!vdp.toggle);
        vdp.write_control(0x00);
        assert!(vdp.toggle);
    }

    #[test]
    fn control_write_second_byte_clears_toggle() {
        let mut vdp = Vdp::new();
        vdp.write_control(0x00);
        vdp.write_control(0x00);
        assert!(!vdp.toggle);
    }

    #[test]
    fn control_write_address_composed_correctly() {
        let mut vdp = Vdp::new();
        // First byte = 0xAB (low 8 bits)
        // Second byte = 0x12 -> code=0, high 6 bits = 0x12
        // Address should be 0x12AB (but masked to 14 bits inside code 0 path)
        // After code 0, address is incremented, so let's check by using code 1 (no action)
        vdp.write_control(0xAB); // low byte
        vdp.write_control(0x52); // 0b01_010010 -> code=1, high=0x12
        assert_eq!(vdp.code, 1);
        assert_eq!(vdp.address, 0x12AB);
    }

    // ---------------------------------------------------------------
    // Control port - register writes (code 2)
    // ---------------------------------------------------------------

    #[test]
    fn control_write_register_sets_value() {
        let mut vdp = Vdp::new();
        // Write value 0xFF to register 0
        // First byte (data) = 0xFF, Second byte = 0b10_000000 -> code=2, reg=0
        vdp.write_control(0xFF);
        vdp.write_control(0x80); // code=2 (bits 7-6 = 10), reg bits 3-0 = 0
        assert_eq!(vdp.registers[0], 0xFF);
    }

    #[test]
    fn control_write_register_5() {
        let mut vdp = Vdp::new();
        // Write 0x42 to register 5
        vdp.write_control(0x42);
        vdp.write_control(0x85); // code=2, reg=5
        assert_eq!(vdp.registers[5], 0x42);
    }

    #[test]
    fn control_write_register_10() {
        let mut vdp = Vdp::new();
        // Write 0x0A to register 10 (0x0A)
        vdp.write_control(0x0A);
        vdp.write_control(0x8A); // code=2, reg=10
        assert_eq!(vdp.registers[10], 0x0A);
    }

    #[test]
    fn control_write_register_out_of_range_ignored() {
        let mut vdp = Vdp::new();
        // Try writing to register 11 (out of range, should be ignored)
        vdp.write_control(0xBB);
        vdp.write_control(0x8B); // code=2, reg=11
        // Registers should all still be zero
        assert_eq!(vdp.registers, [0u8; 11]);
    }

    #[test]
    fn control_write_register_15_out_of_range_ignored() {
        let mut vdp = Vdp::new();
        vdp.write_control(0xCC);
        vdp.write_control(0x8F); // code=2, reg=15
        assert_eq!(vdp.registers, [0u8; 11]);
    }

    // ---------------------------------------------------------------
    // Control port - VRAM read mode (code 0) pre-fetch
    // ---------------------------------------------------------------

    #[test]
    fn control_write_code0_prefetches_vram() {
        let mut vdp = Vdp::new();
        vdp.vram[0x0100] = 0xAA;
        // Set address 0x0100, code 0 (VRAM read)
        vdp.write_control(0x00); // low byte = 0x00
        vdp.write_control(0x01); // code=0 (bits 7-6=00), high=0x01 => address=0x0100
        // Pre-fetch should have loaded vram[0x0100] into read_buffer
        assert_eq!(vdp.read_buffer, 0xAA);
        // Address should have been incremented to 0x0101
        assert_eq!(vdp.address, 0x0101);
    }

    #[test]
    fn control_write_code0_address_wraps_at_14_bits() {
        let mut vdp = Vdp::new();
        vdp.vram[0x3FFF] = 0xBB;
        // Set address 0x3FFF, code 0
        vdp.write_control(0xFF);
        vdp.write_control(0x3F); // code=0, high=0x3F
        assert_eq!(vdp.read_buffer, 0xBB);
        // Address should wrap to 0
        assert_eq!(vdp.address, 0x0000);
    }

    // ---------------------------------------------------------------
    // Control port - CRAM write mode (code 3)
    // ---------------------------------------------------------------

    #[test]
    fn control_write_code3_sets_cram_write_mode() {
        let mut vdp = Vdp::new();
        vdp.write_control(0x00);
        vdp.write_control(0xC0); // code=3 (bits 7-6=11), high=0x00
        assert_eq!(vdp.code, 3);
    }

    // ---------------------------------------------------------------
    // Data port write - VRAM
    // ---------------------------------------------------------------

    #[test]
    fn write_data_to_vram() {
        let mut vdp = Vdp::new();
        // Set code=1 (VRAM write), address=0x1000
        vdp.write_control(0x00);
        vdp.write_control(0x50); // code=1, high=0x10 => address=0x1000
        assert_eq!(vdp.code, 1);

        vdp.write_data(0xAB);
        assert_eq!(vdp.vram[0x1000], 0xAB);
        // Address should be incremented
        assert_eq!(vdp.address, 0x1001);
    }

    #[test]
    fn write_data_to_vram_sequential() {
        let mut vdp = Vdp::new();
        // Set code=1, address=0x0000
        vdp.write_control(0x00);
        vdp.write_control(0x40); // code=1
        for i in 0..16u8 {
            vdp.write_data(i);
        }
        for i in 0..16u8 {
            assert_eq!(vdp.vram[i as usize], i);
        }
        assert_eq!(vdp.address, 16);
    }

    #[test]
    fn write_data_resets_toggle() {
        let mut vdp = Vdp::new();
        // Write first byte of control (sets toggle)
        vdp.write_control(0x00);
        assert!(vdp.toggle);
        // Now write data - should reset toggle
        vdp.write_data(0x00);
        assert!(!vdp.toggle);
    }

    #[test]
    fn write_data_vram_address_wraps() {
        let mut vdp = Vdp::new();
        // Set address to 0x3FFF, code=1
        vdp.write_control(0xFF);
        vdp.write_control(0x7F); // code=1, high=0x3F
        vdp.write_data(0xCC);
        assert_eq!(vdp.vram[0x3FFF], 0xCC);
        assert_eq!(vdp.address, 0x0000); // wrapped
    }

    // ---------------------------------------------------------------
    // Data port write - CRAM (Game Gear latch mechanism)
    // ---------------------------------------------------------------

    #[test]
    fn write_data_cram_even_stores_latch() {
        let mut vdp = Vdp::new();
        // Set code=3 (CRAM write), address=0x0000 (even)
        vdp.write_control(0x00);
        vdp.write_control(0xC0); // code=3
        vdp.write_data(0x34); // even address -> stored in latch
        assert_eq!(vdp.cram_latch, 0x34);
        // CRAM should not be written yet
        assert_eq!(vdp.cram[0], 0);
        assert_eq!(vdp.cram[1], 0);
    }

    #[test]
    fn write_data_cram_odd_writes_word() {
        let mut vdp = Vdp::new();
        // Set code=3 (CRAM write), address=0x0000
        vdp.write_control(0x00);
        vdp.write_control(0xC0); // code=3
        // Even byte: store in latch
        vdp.write_data(0x34); // address was 0, now 1
        // Odd byte: write latch + val as 16-bit word
        vdp.write_data(0x0A); // address was 1, now 2
        // CRAM[0] = latch = 0x34, CRAM[1] = val = 0x0A
        assert_eq!(vdp.cram[0], 0x34);
        assert_eq!(vdp.cram[1], 0x0A);
    }

    #[test]
    fn write_data_cram_multiple_colors() {
        let mut vdp = Vdp::new();
        // Set code=3, address=0x0000
        vdp.write_control(0x00);
        vdp.write_control(0xC0);

        // Color 0: 0x1234 (LE: low=0x34, high=0x12)
        vdp.write_data(0x34); // even -> latch
        vdp.write_data(0x12); // odd -> write word to CRAM[0..1]

        // Color 1: 0x5678 (LE: low=0x78, high=0x56)
        vdp.write_data(0x78); // even -> latch
        vdp.write_data(0x56); // odd -> write word to CRAM[2..3]

        assert_eq!(vdp.cram[0], 0x34);
        assert_eq!(vdp.cram[1], 0x12);
        assert_eq!(vdp.cram[2], 0x78);
        assert_eq!(vdp.cram[3], 0x56);
    }

    #[test]
    fn write_data_cram_wraps_within_64_bytes() {
        let mut vdp = Vdp::new();
        // Set code=3, address at 62 (the last valid even address in 64-byte CRAM)
        vdp.write_control(62);
        vdp.write_control(0xC0); // code=3, address=62
        vdp.write_data(0xAA); // even -> latch, addr becomes 63
        vdp.write_data(0xBB); // odd -> write to CRAM[62..63], addr becomes 64 (wraps in next op)
        assert_eq!(vdp.cram[62], 0xAA);
        assert_eq!(vdp.cram[63], 0xBB);
    }

    // ---------------------------------------------------------------
    // Data port read - VRAM with pre-fetch buffer
    // ---------------------------------------------------------------

    #[test]
    fn read_data_returns_buffer_then_prefetches() {
        let mut vdp = Vdp::new();
        vdp.vram[0] = 0x11;
        vdp.vram[1] = 0x22;
        vdp.vram[2] = 0x33;

        // Set code=0 (VRAM read), address=0x0000
        // This pre-fetches vram[0] into buffer and increments to 1
        vdp.write_control(0x00);
        vdp.write_control(0x00); // code=0, address=0x0000

        // First read: returns pre-fetched vram[0] = 0x11, then fetches vram[1]
        assert_eq!(vdp.read_data(), 0x11);
        assert_eq!(vdp.read_buffer, 0x22);
        assert_eq!(vdp.address, 0x0002);

        // Second read: returns pre-fetched vram[1] = 0x22, then fetches vram[2]
        assert_eq!(vdp.read_data(), 0x22);
        assert_eq!(vdp.read_buffer, 0x33);
        assert_eq!(vdp.address, 0x0003);
    }

    #[test]
    fn read_data_resets_toggle() {
        let mut vdp = Vdp::new();
        vdp.write_control(0x00); // sets toggle
        assert!(vdp.toggle);
        vdp.read_data();
        assert!(!vdp.toggle);
    }

    #[test]
    fn read_data_address_wraps() {
        let mut vdp = Vdp::new();
        vdp.vram[0x3FFF] = 0xEE;
        vdp.vram[0x0000] = 0xFF;

        // Setup: code=0, address=0x3FFF
        // Pre-fetch reads vram[0x3FFF]=0xEE, address becomes 0x0000
        vdp.write_control(0xFF);
        vdp.write_control(0x3F); // code=0
        assert_eq!(vdp.read_buffer, 0xEE);
        assert_eq!(vdp.address, 0x0000);

        // read_data returns 0xEE (buffer), fetches vram[0]=0xFF, address becomes 1
        let val = vdp.read_data();
        assert_eq!(val, 0xEE);
        assert_eq!(vdp.read_buffer, 0xFF);
        assert_eq!(vdp.address, 0x0001);
    }

    // ---------------------------------------------------------------
    // Control port read - status register
    // ---------------------------------------------------------------

    #[test]
    fn read_control_returns_status_and_clears_flags() {
        let mut vdp = Vdp::new();
        vdp.status = 0xFF;
        vdp.frame_irq_pending = true;
        vdp.line_irq_pending = true;

        let val = vdp.read_control();
        assert_eq!(val, 0xFF);
        // Bits 7, 6, 5 should be cleared; bits 4-0 remain
        assert_eq!(vdp.status, 0x1F);
        assert!(!vdp.frame_irq_pending);
        assert!(!vdp.line_irq_pending);
    }

    #[test]
    fn read_control_resets_toggle() {
        let mut vdp = Vdp::new();
        vdp.write_control(0x00); // sets toggle
        assert!(vdp.toggle);
        vdp.read_control();
        assert!(!vdp.toggle);
    }

    #[test]
    fn read_control_only_clears_upper_three_bits() {
        let mut vdp = Vdp::new();
        vdp.status = 0b1010_1010;
        let val = vdp.read_control();
        assert_eq!(val, 0b1010_1010);
        // After read: bits 7,6,5 cleared -> 0b000_01010 = 0x0A
        assert_eq!(vdp.status, 0b0000_1010);
    }

    // ---------------------------------------------------------------
    // V-counter and H-counter
    // ---------------------------------------------------------------

    #[test]
    fn read_v_counter_returns_v_counter() {
        let mut vdp = Vdp::new();
        vdp.v_counter = 0x42;
        assert_eq!(vdp.read_v_counter(), 0x42);
    }

    #[test]
    fn read_h_counter_returns_zero() {
        let vdp = Vdp::new();
        assert_eq!(vdp.read_h_counter(), 0);
    }

    // ---------------------------------------------------------------
    // run_scanline - v_counter mapping
    // ---------------------------------------------------------------

    #[test]
    fn run_scanline_v_counter_line_0() {
        let mut vdp = Vdp::new();
        // scanline starts at 0
        vdp.run_scanline();
        // After run_scanline(0): v_counter should be 0, scanline advances to 1
        // v_counter is set at the beginning of run_scanline before incrementing
        // Actually, run_scanline first sets v_counter for current scanline, then increments
        // So at scanline=0, v_counter=0, then scanline becomes 1
        assert_eq!(vdp.v_counter, 0);
        assert_eq!(vdp.scanline, 1);
    }

    #[test]
    fn run_scanline_v_counter_line_191() {
        let mut vdp = Vdp::new();
        vdp.scanline = 191;
        vdp.run_scanline();
        assert_eq!(vdp.v_counter, 191);
    }

    #[test]
    fn run_scanline_v_counter_line_218_is_0xda() {
        let mut vdp = Vdp::new();
        vdp.scanline = 218;
        vdp.run_scanline();
        assert_eq!(vdp.v_counter, 0xDA);
    }

    #[test]
    fn run_scanline_v_counter_line_219_jumps_to_0xd5() {
        let mut vdp = Vdp::new();
        vdp.scanline = 219;
        vdp.run_scanline();
        assert_eq!(vdp.v_counter, 0xD5);
    }

    #[test]
    fn run_scanline_v_counter_line_261_is_0xff() {
        let mut vdp = Vdp::new();
        vdp.scanline = 261;
        vdp.run_scanline();
        // 261 - 219 + 0xD5 = 42 + 213 = 255 = 0xFF
        assert_eq!(vdp.v_counter, 0xFF);
    }

    #[test]
    fn run_scanline_wraps_at_262() {
        let mut vdp = Vdp::new();
        vdp.scanline = 261;
        vdp.run_scanline();
        assert_eq!(vdp.scanline, 0);
    }

    #[test]
    fn run_scanline_full_frame_wraps() {
        let mut vdp = Vdp::new();
        for _ in 0..262 {
            vdp.run_scanline();
        }
        assert_eq!(vdp.scanline, 0);
    }

    // ---------------------------------------------------------------
    // run_scanline - frame IRQ
    // ---------------------------------------------------------------

    #[test]
    fn run_scanline_frame_irq_at_line_192() {
        let mut vdp = Vdp::new();
        vdp.scanline = 192;
        vdp.run_scanline();
        assert!(vdp.frame_irq_pending);
        assert_eq!(vdp.status & 0x80, 0x80);
    }

    #[test]
    fn run_scanline_no_frame_irq_before_192() {
        let mut vdp = Vdp::new();
        vdp.scanline = 191;
        vdp.run_scanline();
        assert!(!vdp.frame_irq_pending);
        assert_eq!(vdp.status & 0x80, 0);
    }

    // ---------------------------------------------------------------
    // run_scanline - line counter
    // ---------------------------------------------------------------

    #[test]
    fn run_scanline_line_counter_decrements_during_active_display() {
        let mut vdp = Vdp::new();
        vdp.registers[0x0A] = 10; // line counter reload value
        vdp.line_counter = 5;
        vdp.scanline = 50; // active display line
        vdp.run_scanline();
        assert_eq!(vdp.line_counter, 4);
    }

    #[test]
    fn run_scanline_line_counter_underflow_reloads_and_sets_irq() {
        let mut vdp = Vdp::new();
        vdp.registers[0x0A] = 10;
        vdp.line_counter = 0;
        vdp.scanline = 50; // active display line
        vdp.run_scanline();
        // Line counter was 0, so it reloads from reg 0x0A and sets line IRQ
        assert_eq!(vdp.line_counter, 10);
        assert!(vdp.line_irq_pending);
    }

    #[test]
    fn run_scanline_line_counter_reload_during_vblank() {
        let mut vdp = Vdp::new();
        vdp.registers[0x0A] = 20;
        vdp.line_counter = 5;
        vdp.scanline = 200; // vblank line
        vdp.run_scanline();
        // During vblank, line counter is reloaded each scanline
        assert_eq!(vdp.line_counter, 20);
    }

    #[test]
    fn run_scanline_line_counter_reload_at_192() {
        let mut vdp = Vdp::new();
        vdp.registers[0x0A] = 15;
        vdp.line_counter = 3;
        vdp.scanline = 192;
        vdp.run_scanline();
        // At line 192, line counter is reloaded
        assert_eq!(vdp.line_counter, 15);
    }

    // ---------------------------------------------------------------
    // irq_pending
    // ---------------------------------------------------------------

    #[test]
    fn irq_pending_false_by_default() {
        let vdp = Vdp::new();
        assert!(!vdp.irq_pending());
    }

    #[test]
    fn irq_pending_frame_irq_enabled() {
        let mut vdp = Vdp::new();
        vdp.frame_irq_pending = true;
        vdp.registers[1] = 0x20; // bit 5 set: frame IRQ enabled
        assert!(vdp.irq_pending());
    }

    #[test]
    fn irq_pending_frame_irq_disabled() {
        let mut vdp = Vdp::new();
        vdp.frame_irq_pending = true;
        vdp.registers[1] = 0x00; // bit 5 clear: frame IRQ disabled
        assert!(!vdp.irq_pending());
    }

    #[test]
    fn irq_pending_line_irq_enabled() {
        let mut vdp = Vdp::new();
        vdp.line_irq_pending = true;
        vdp.registers[0] = 0x10; // bit 4 set: line IRQ enabled
        assert!(vdp.irq_pending());
    }

    #[test]
    fn irq_pending_line_irq_disabled() {
        let mut vdp = Vdp::new();
        vdp.line_irq_pending = true;
        vdp.registers[0] = 0x00; // bit 4 clear
        assert!(!vdp.irq_pending());
    }

    #[test]
    fn irq_pending_both_irqs() {
        let mut vdp = Vdp::new();
        vdp.frame_irq_pending = true;
        vdp.line_irq_pending = true;
        vdp.registers[0] = 0x10;
        vdp.registers[1] = 0x20;
        assert!(vdp.irq_pending());
    }

    #[test]
    fn irq_pending_cleared_by_read_control() {
        let mut vdp = Vdp::new();
        vdp.frame_irq_pending = true;
        vdp.line_irq_pending = true;
        vdp.registers[0] = 0x10;
        vdp.registers[1] = 0x20;
        assert!(vdp.irq_pending());

        vdp.read_control(); // clears both pending flags
        assert!(!vdp.irq_pending());
    }

    // ---------------------------------------------------------------
    // Integration: write VRAM then read it back
    // ---------------------------------------------------------------

    #[test]
    fn integration_write_then_read_vram() {
        let mut vdp = Vdp::new();

        // Write 0xAA to VRAM address 0x0010
        vdp.write_control(0x10);
        vdp.write_control(0x40); // code=1 (VRAM write), address=0x0010
        vdp.write_data(0xAA);
        vdp.write_data(0xBB);
        vdp.write_data(0xCC);

        // Now set up for reading from 0x0010
        vdp.write_control(0x10);
        vdp.write_control(0x00); // code=0 (VRAM read), address=0x0010
        // Pre-fetch loads vram[0x0010]=0xAA into buffer

        assert_eq!(vdp.read_data(), 0xAA);
        assert_eq!(vdp.read_data(), 0xBB);
        assert_eq!(vdp.read_data(), 0xCC);
    }

    // ---------------------------------------------------------------
    // Integration: control port half-write interrupted by data port
    // ---------------------------------------------------------------

    #[test]
    fn data_write_interrupts_half_control_write() {
        let mut vdp = Vdp::new();
        // Start a control write (first byte)
        vdp.write_control(0x00); // sets toggle
        assert!(vdp.toggle);
        // Now write data - this should reset the toggle
        vdp.write_data(0x42);
        assert!(!vdp.toggle);
        // Next control write should start fresh as first byte
        vdp.write_control(0x10);
        assert!(vdp.toggle);
    }

    #[test]
    fn data_read_interrupts_half_control_write() {
        let mut vdp = Vdp::new();
        vdp.write_control(0x00); // sets toggle
        assert!(vdp.toggle);
        vdp.read_data(); // resets toggle
        assert!(!vdp.toggle);
    }

    #[test]
    fn control_read_interrupts_half_control_write() {
        let mut vdp = Vdp::new();
        vdp.write_control(0x00); // sets toggle
        assert!(vdp.toggle);
        vdp.read_control(); // resets toggle
        assert!(!vdp.toggle);
    }

    // ---------------------------------------------------------------
    // Integration: full frame v_counter sweep
    // ---------------------------------------------------------------

    #[test]
    fn v_counter_full_frame_sweep() {
        let mut vdp = Vdp::new();
        let mut v_counters = Vec::new();
        for _ in 0..262 {
            vdp.run_scanline();
            // We pushed v_counter which was set at the start of run_scanline
            // Actually v_counter is set *during* run_scanline, so we read it after
        }
        // Reset and collect properly
        vdp = Vdp::new();
        for _ in 0..262 {
            // v_counter is set at the start of run_scanline, before scanline increments
            let _before = vdp.v_counter;
            vdp.run_scanline();
            v_counters.push(vdp.v_counter);
        }
        // After running scanline 0: v_counter should be 0
        assert_eq!(v_counters[0], 0x00);
        // After running scanline 191: v_counter should be 191
        assert_eq!(v_counters[191], 191);
        // After running scanline 218: v_counter should be 0xDA
        assert_eq!(v_counters[218], 0xDA);
        // After running scanline 219: v_counter should be 0xD5
        assert_eq!(v_counters[219], 0xD5);
        // After running scanline 261: v_counter should be 0xFF
        assert_eq!(v_counters[261], 0xFF);
    }

    // ---------------------------------------------------------------
    // Edge case: write_data to VRAM even when code=0 (read mode)
    // ---------------------------------------------------------------

    #[test]
    fn write_data_writes_vram_regardless_of_code_when_not_cram() {
        let mut vdp = Vdp::new();
        // Set code=0 (VRAM read), address=0x0000
        vdp.write_control(0x00);
        vdp.write_control(0x00); // code=0

        // Even though code is "read mode", write_data still writes to VRAM
        // (address was incremented by pre-fetch, so it's now at 1)
        vdp.write_data(0x55);
        assert_eq!(vdp.vram[0x0001], 0x55);
    }

    // ---------------------------------------------------------------
    // Line counter: counts down from loaded value correctly
    // ---------------------------------------------------------------

    #[test]
    fn line_counter_countdown_sequence() {
        let mut vdp = Vdp::new();
        vdp.registers[0x0A] = 3; // reload value
        vdp.line_counter = 3;
        vdp.scanline = 0;

        // Line 0: counter=3, decrement to 2
        vdp.run_scanline();
        assert_eq!(vdp.line_counter, 2);
        assert!(!vdp.line_irq_pending);

        // Line 1: counter=2, decrement to 1
        vdp.run_scanline();
        assert_eq!(vdp.line_counter, 1);
        assert!(!vdp.line_irq_pending);

        // Line 2: counter=1, decrement to 0
        vdp.run_scanline();
        assert_eq!(vdp.line_counter, 0);
        assert!(!vdp.line_irq_pending);

        // Line 3: counter=0, underflow: reload to 3 and set IRQ
        vdp.run_scanline();
        assert_eq!(vdp.line_counter, 3);
        assert!(vdp.line_irq_pending);
    }

    // ---------------------------------------------------------------
    // Rendering: get_color conversion
    // ---------------------------------------------------------------

    #[test]
    fn test_get_color_conversion() {
        let mut vdp = Vdp::new();
        // Set up CRAM color 0 with R=0xF, G=0xA, B=0x5
        // Low byte: bits 3-0 = Red (0xF), bits 7-4 = Green (0xA) => 0xAF
        // High byte: bits 3-0 = Blue (0x5) => 0x05
        vdp.cram[0] = 0xAF; // color 0 low byte
        vdp.cram[1] = 0x05; // color 0 high byte

        let color = vdp.get_color(0);
        // R = 0xF * 17 = 255, G = 0xA * 17 = 170, B = 0x5 * 17 = 85
        let expected = (255 << 16) | (170 << 8) | 85;
        assert_eq!(color, expected);

        // Test color with all zeros
        vdp.cram[2] = 0x00;
        vdp.cram[3] = 0x00;
        assert_eq!(vdp.get_color(1), 0x000000);

        // Test color with all max
        vdp.cram[4] = 0xFF; // R=0xF, G=0xF
        vdp.cram[5] = 0x0F; // B=0xF
        let white = vdp.get_color(2);
        assert_eq!(white, 0x00FFFFFF);
    }

    // ---------------------------------------------------------------
    // Rendering: display blanked fills backdrop
    // ---------------------------------------------------------------

    #[test]
    fn test_render_display_blanked() {
        let mut vdp = Vdp::new();
        // Display blanked: register 1 bit 6 = 0 (which is default)
        // Set backdrop color: palette 1, color (reg[7] & 0x0F)
        vdp.registers[7] = 0x01; // backdrop = palette 1, color 1 => index 17
        // Set CRAM color 17 (index 17 * 2 = 34)
        vdp.cram[34] = 0x21; // R=1, G=2
        vdp.cram[35] = 0x03; // B=3

        vdp.render_scanline(0);

        let expected = ((1 * 17) << 16) | ((2 * 17) << 8) | (3 * 17);
        // All 256 pixels on line 0 should be backdrop
        for x in 0..256 {
            assert_eq!(vdp.framebuffer[x], expected,
                "pixel {} should be backdrop color", x);
        }
    }

    // ---------------------------------------------------------------
    // Rendering: solid tile renders correctly
    // ---------------------------------------------------------------

    #[test]
    fn test_render_solid_tile() {
        let mut vdp = Vdp::new();
        // Enable display
        vdp.registers[1] = 0x40; // bit 6 = 1

        // Name table at $3800 (reg[2] = 0x0E => (0x0E & 0x0E) << 10 = 0x3800)
        vdp.registers[2] = 0x0E;

        // Set up tile 1 with all pixels = color index 1 (palette 0)
        // Tile 1 starts at address 1 * 32 = 32
        // For color index 1: bitplane 0 = 0xFF (all bits set), bp1/bp2/bp3 = 0
        for row in 0..8 {
            let addr = 32 + row * 4;
            vdp.vram[addr] = 0xFF;     // bitplane 0: all 1s
            vdp.vram[addr + 1] = 0x00; // bitplane 1: all 0s
            vdp.vram[addr + 2] = 0x00; // bitplane 2: all 0s
            vdp.vram[addr + 3] = 0x00; // bitplane 3: all 0s
        }

        // Set name table entry at row 0, col 0 to tile 1, palette 0, no flip, no priority
        // Entry: tile index = 1, high byte = 0
        let nt_addr = 0x3800;
        vdp.vram[nt_addr] = 0x01;     // low byte: tile index bits 0-7 = 1
        vdp.vram[nt_addr + 1] = 0x00; // high byte: all flags 0

        // Set CRAM color 1 (palette 0, color 1)
        vdp.cram[2] = 0x0F; // R=0xF, G=0
        vdp.cram[3] = 0x00; // B=0

        // No scroll
        vdp.registers[8] = 0; // H scroll
        vdp.registers[9] = 0; // V scroll

        vdp.render_scanline(0);

        // First 8 pixels (tile 0 col 0) should be color 1 = red
        let expected_color = 0x0F * 17; // R = 255
        let expected = expected_color << 16;
        for x in 0..8 {
            assert_eq!(vdp.framebuffer[x], expected,
                "pixel {} should be color 1 (red)", x);
        }
    }

    // ---------------------------------------------------------------
    // Rendering: basic sprite
    // ---------------------------------------------------------------

    #[test]
    fn test_sprite_basic() {
        let mut vdp = Vdp::new();
        vdp.registers[1] = 0x40; // enable display
        vdp.registers[2] = 0x0E; // name table at $3800
        vdp.registers[5] = 0x7E; // SAT base: (0x7E & 0x7E) << 7 = 0x3F00
        vdp.registers[6] = 0x00; // pattern base = 0

        let sat_base = 0x3F00usize;

        // Place sprite 0 at screen Y=0, X=16, tile=2
        // Y entry: sprite appears at (entry + 1), so for Y=0 we store Y=255 (wraps to 0)
        // Actually: "sprite_y = (y_entry + 1)" so to appear at Y=0, y_entry must be 255 (wraps to 0)
        // Wait - the code uses wrapping_add: (y_entry as u16).wrapping_add(1).
        // For y_entry=255: (255 as u16).wrapping_add(1) = 256. Then line 0 >= 256 is false.
        // So y_entry = 255 won't work for line 0. Let me re-read...
        // Actually sprite_y = y_entry + 1. For the sprite to appear on line 1:
        // y_entry = 0 => sprite_y = 1 => appears on lines 1-8
        // To appear on line 10, y_entry = 9 => sprite_y = 10 => lines 10-17
        vdp.vram[sat_base] = 9; // sprite 0: Y entry = 9, sprite appears at Y=10

        // Terminate after sprite 0
        vdp.vram[sat_base + 1] = 0xD0; // terminator

        // X and tile at SAT_base + $80
        vdp.vram[sat_base + 0x80] = 16;  // X = 16
        vdp.vram[sat_base + 0x81] = 2;   // tile index = 2

        // Set up tile 2: all pixels = color index 3 (in sprite palette = palette 1 color 3 = index 19)
        // Color index 3 = bitplane 0=1, bitplane 1=1, bp2=0, bp3=0
        for row in 0..8 {
            let addr = 2 * 32 + row * 4;
            vdp.vram[addr] = 0xFF;     // bp0: all 1s
            vdp.vram[addr + 1] = 0xFF; // bp1: all 1s
            vdp.vram[addr + 2] = 0x00; // bp2: all 0s
            vdp.vram[addr + 3] = 0x00; // bp3: all 0s
        }

        // Set CRAM color 19 (palette 1, color 3): index 19 * 2 = 38
        vdp.cram[38] = 0x00; // R=0, G=0
        vdp.cram[39] = 0x0F; // B=0xF => blue

        // Render line 10 (first line of sprite)
        vdp.render_scanline(10);

        let expected_blue = 0x0F * 17; // B=255
        let offset = 10 * 256;
        // Pixels 16-23 should be the sprite color (blue)
        for x in 16..24 {
            assert_eq!(vdp.framebuffer[offset + x], expected_blue,
                "pixel {} on line 10 should be sprite blue", x);
        }
    }

    // ---------------------------------------------------------------
    // Rendering: sprite priority (BG priority hides sprite)
    // ---------------------------------------------------------------

    #[test]
    fn test_sprite_priority() {
        let mut vdp = Vdp::new();
        vdp.registers[1] = 0x40; // enable display
        vdp.registers[2] = 0x0E; // name table at $3800
        vdp.registers[5] = 0x7E; // SAT at $3F00
        vdp.registers[6] = 0x00; // pattern base = 0
        vdp.registers[8] = 0;    // no H scroll
        vdp.registers[9] = 0;    // no V scroll

        let sat_base = 0x3F00usize;
        let nt_base = 0x3800usize;

        // BG tile 1: all pixels = color index 5 (non-zero), with priority bit set
        for row in 0..8 {
            let addr = 1 * 32 + row * 4;
            // Color index 5 = 0101 => bp0=1, bp1=0, bp2=1, bp3=0
            vdp.vram[addr] = 0xFF;     // bp0
            vdp.vram[addr + 1] = 0x00; // bp1
            vdp.vram[addr + 2] = 0xFF; // bp2
            vdp.vram[addr + 3] = 0x00; // bp3
        }

        // Set name table entry at row 0, col 0 to tile 1 with priority bit
        // High byte bit 4 = priority => 0x10
        vdp.vram[nt_base] = 0x01;     // tile 1
        vdp.vram[nt_base + 1] = 0x10; // priority bit set

        // Set CRAM color 5 (palette 0) => green
        vdp.cram[10] = 0xF0; // R=0, G=0xF
        vdp.cram[11] = 0x00; // B=0

        // Sprite at X=0, Y=0 (y_entry=255 won't work, use y_entry=0 => sprite_y=1)
        // Actually, let's put sprite on line 1 and render line 1
        vdp.vram[sat_base] = 0; // y_entry=0 => sprite_y=1
        vdp.vram[sat_base + 1] = 0xD0; // terminator
        vdp.vram[sat_base + 0x80] = 0; // X=0
        vdp.vram[sat_base + 0x81] = 3; // tile 3

        // Sprite tile 3: all pixels = color index 1 (non-zero = visible)
        for row in 0..8 {
            let addr = 3 * 32 + row * 4;
            vdp.vram[addr] = 0xFF;     // bp0: color 1
            vdp.vram[addr + 1] = 0x00;
            vdp.vram[addr + 2] = 0x00;
            vdp.vram[addr + 3] = 0x00;
        }

        // Set CRAM color 17 (palette 1, color 1) => red
        vdp.cram[34] = 0x0F; // R=0xF
        vdp.cram[35] = 0x00;

        // Render line 1 (first visible line for the sprite)
        // BG tile is at row 0 (line 1 / 8 = 0), so the BG tile with priority covers it
        vdp.render_scanline(1);

        let green = (0x0F * 17) << 8; // G=255
        let offset = 1 * 256;
        // BG has priority and non-zero color, so sprite should be hidden.
        // Pixels 0-7 should show BG green, not sprite red
        for x in 0..8 {
            assert_eq!(vdp.framebuffer[offset + x], green,
                "pixel {} on line 1 should be BG green (priority), not sprite", x);
        }
    }

    // ---------------------------------------------------------------
    // Rendering: column 0 blanking
    // ---------------------------------------------------------------

    #[test]
    fn test_column_0_blanking() {
        let mut vdp = Vdp::new();
        vdp.registers[1] = 0x40; // enable display
        vdp.registers[2] = 0x0E; // name table at $3800
        vdp.registers[0] = 0x20; // bit 5: column 0 blanking
        vdp.registers[7] = 0x02; // backdrop = palette 1, color 2 => index 18

        // Set CRAM color 18 (index 18 * 2 = 36)
        vdp.cram[36] = 0x00; // R=0, G=0
        vdp.cram[37] = 0x0A; // B=0xA

        // Set up BG tile 1 with non-zero color (so we can see it gets blanked)
        for row in 0..8 {
            let addr = 1 * 32 + row * 4;
            vdp.vram[addr] = 0xFF;
            vdp.vram[addr + 1] = 0x00;
            vdp.vram[addr + 2] = 0x00;
            vdp.vram[addr + 3] = 0x00;
        }

        // Name table col 0 = tile 1
        let nt_base = 0x3800;
        vdp.vram[nt_base] = 0x01;
        vdp.vram[nt_base + 1] = 0x00;

        vdp.render_scanline(0);

        let backdrop = vdp.get_color(18);
        // First 8 pixels should be backdrop (blanked)
        for x in 0..8 {
            assert_eq!(vdp.framebuffer[x], backdrop,
                "pixel {} should be blanked to backdrop", x);
        }
        // Pixel 8 should NOT be backdrop (it's beyond column 0)
        // It should be whatever the BG tile at column 1 renders (which is tile 0 = color 0)
        // Color 0 from palette 0 = CRAM[0..1] = 0 (all black)
        assert_ne!(vdp.framebuffer[8], backdrop,
            "pixel 8 should not be blanked");
    }

    // ---------------------------------------------------------------
    // Rendering: run_scanline calls render for active lines
    // ---------------------------------------------------------------

    #[test]
    fn test_run_scanline_calls_render() {
        let mut vdp = Vdp::new();
        vdp.registers[1] = 0x40; // enable display
        vdp.registers[2] = 0x0E; // name table at $3800
        vdp.registers[7] = 0x00; // backdrop = palette 1, color 0 => index 16

        // Set CRAM color 16 to a distinctive value
        vdp.cram[32] = 0x0F; // R=0xF, G=0
        vdp.cram[33] = 0x00; // B=0

        // Set up tile 0 with all pixels color 0 (transparent/color 0 from palette 0)
        // Color 0 is in CRAM[0..1], leave as default (0)

        // Set up a tile with a distinctive color at name table position (0,0)
        // Tile 1: all color index 1
        for row in 0..8 {
            let addr = 1 * 32 + row * 4;
            vdp.vram[addr] = 0xFF;
            vdp.vram[addr + 1] = 0xFF;
            vdp.vram[addr + 2] = 0xFF;
            vdp.vram[addr + 3] = 0xFF;
        }

        // Name table entry at row 0, col 0 = tile 1
        vdp.vram[0x3800] = 0x01;
        vdp.vram[0x3801] = 0x00;

        // CRAM color 15 (palette 0, color 15 = all bits set)
        vdp.cram[30] = 0xFF; // R=0xF, G=0xF
        vdp.cram[31] = 0x0F; // B=0xF

        // Framebuffer should be all zeros initially
        assert_eq!(vdp.framebuffer[0], 0);

        // Run scanline 0 - should trigger render
        vdp.scanline = 0;
        vdp.run_scanline();

        // Pixel (0,0) should now be non-zero (it was rendered)
        let expected = 0x00FFFFFF; // color index 15 = white
        assert_eq!(vdp.framebuffer[0], expected,
            "run_scanline should have rendered line 0");

        // Now run a vblank scanline (e.g. 200) - should NOT modify framebuffer line 200
        // (which is beyond 192 visible lines, so no framebuffer write)
        vdp.scanline = 200;
        // Framebuffer line 200 doesn't exist (192 lines only), so just verify no panic
        vdp.run_scanline();
    }

    // ---------------------------------------------------------------
    // Rendering: vertical scroll
    // ---------------------------------------------------------------

    #[test]
    fn test_vertical_scroll() {
        let mut vdp = Vdp::new();
        vdp.registers[1] = 0x40; // enable display
        vdp.registers[2] = 0x0E; // name table at $3800
        vdp.registers[8] = 0;    // no H scroll
        vdp.registers[9] = 8;    // V scroll = 8 (shifts BG down by 8, so line 0 sees row 1)

        let nt_base = 0x3800usize;

        // Set up tile 10: all pixels = color index 7
        // Color index 7 = 0111 => bp0=1, bp1=1, bp2=1, bp3=0
        for row in 0..8 {
            let addr = 10 * 32 + row * 4;
            vdp.vram[addr] = 0xFF;     // bp0
            vdp.vram[addr + 1] = 0xFF; // bp1
            vdp.vram[addr + 2] = 0xFF; // bp2
            vdp.vram[addr + 3] = 0x00; // bp3
        }

        // Set up tile 20: all pixels = color index 2
        // Color index 2 = 0010 => bp0=0, bp1=1, bp2=0, bp3=0
        for row in 0..8 {
            let addr = 20 * 32 + row * 4;
            vdp.vram[addr] = 0x00;     // bp0
            vdp.vram[addr + 1] = 0xFF; // bp1
            vdp.vram[addr + 2] = 0x00; // bp2
            vdp.vram[addr + 3] = 0x00; // bp3
        }

        // Name table row 0, col 0 = tile 10
        vdp.vram[nt_base] = 10;
        vdp.vram[nt_base + 1] = 0x00;

        // Name table row 1, col 0 = tile 20
        // Row 1 starts at offset 32*2 = 64 bytes from nt_base
        vdp.vram[nt_base + 64] = 20;
        vdp.vram[nt_base + 65] = 0x00;

        // Set CRAM colors
        // Color 7 (palette 0): index 7*2 = 14
        vdp.cram[14] = 0x0F; // R=0xF, G=0
        vdp.cram[15] = 0x00; // B=0
        // Color 2 (palette 0): index 2*2 = 4
        vdp.cram[4] = 0x00; // R=0, G=0
        vdp.cram[5] = 0x0F; // B=0xF

        // With V scroll = 8, line 0 should map to effective_y = (0 + 8) % 224 = 8
        // effective_y = 8 => row = 8/8 = 1, fine_y = 0
        // So line 0 should show tile from name table row 1 = tile 20 = color 2 (blue)
        vdp.render_scanline(0);

        let blue = 0x0F * 17; // B=255
        assert_eq!(vdp.framebuffer[0], blue,
            "with V scroll=8, line 0 should show name table row 1 (blue tile)");

        // Without scroll, line 0 would show row 0 = tile 10 = color 7 (red)
        vdp.registers[9] = 0;
        vdp.render_scanline(0);

        let red = (0x0F * 17) << 16; // R=255
        assert_eq!(vdp.framebuffer[0], red,
            "without scroll, line 0 should show name table row 0 (red tile)");
    }
}
