// SN76489 PSG sound chip emulation (Game Gear variant)

const VOLUME_TABLE: [i16; 16] = [
    32767, 26028, 20675, 16422, 13045, 10362,
     8231,  6568,  5193,  4125,  3277,  2603,
     2067,  1642,  1304,     0,
];

/// PSG clock frequency (NTSC) divided by 16 to get the internal tick rate.
const PSG_CLOCK: f64 = 3_579_545.0 / 16.0;

pub struct Psg {
    /// 10-bit tone frequency registers for channels 0-2
    pub tone: [u16; 3],
    /// 4-bit volume for channels 0-3 (0 = max, 15 = silent)
    pub volume: [u8; 4],
    /// 3-bit noise control register
    pub noise_register: u8,
    /// GG stereo panning register (port $06), default 0xFF (all enabled)
    pub stereo: u8,
    /// Internal countdown counters for channels 0-3
    counter: [u16; 4],
    /// Output polarity for channels 0-3 (0 or 1)
    polarity: [u8; 4],
    /// Noise linear-feedback shift register (16-bit), reset value 0x8000
    lfsr: u16,
    /// Currently latched channel (0-3)
    latched_channel: usize,
    /// Whether the latched register is volume (true) or tone/noise (false)
    latched_is_volume: bool,
    /// Fractional clock accumulator for sample generation
    clock_frac: f64,
    /// PSG ticks per output sample, derived from the audio device sample rate
    ticks_per_sample: f64,
    /// Low-pass filter state (left, right) to smooth square wave harmonics
    lpf: (f64, f64),
    /// Low-pass filter coefficient (0..1, higher = less filtering)
    lpf_alpha: f64,
}

impl Psg {
    /// Create a new PSG with all channels silent.
    pub fn new() -> Self {
        let sample_rate = 44_100.0;
        Self {
            tone: [0; 3],
            volume: [0x0F; 4],
            noise_register: 0,
            stereo: 0xFF,
            counter: [0; 4],
            polarity: [0; 4],
            lfsr: 0x8000,
            latched_channel: 0,
            latched_is_volume: false,
            clock_frac: 0.0,
            ticks_per_sample: PSG_CLOCK / sample_rate,
            lpf: (0.0, 0.0),
            lpf_alpha: Self::calc_lpf_alpha(sample_rate),
        }
    }

    /// Set the output sample rate (call before generating samples).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.ticks_per_sample = PSG_CLOCK / rate as f64;
        self.lpf_alpha = Self::calc_lpf_alpha(rate as f64);
    }

    /// Calculate low-pass filter coefficient for a ~12 kHz cutoff.
    fn calc_lpf_alpha(sample_rate: f64) -> f64 {
        let cutoff = 12_000.0;
        let rc = 1.0 / (2.0 * std::f64::consts::PI * cutoff);
        let dt = 1.0 / sample_rate;
        dt / (rc + dt)
    }

    /// Write a byte to the PSG data port.
    ///
    /// If bit 7 is set, this is a latch byte:
    ///   - Bits 6-5: channel (0-3)
    ///   - Bit 4: type (0 = tone/noise, 1 = volume)
    ///   - Bits 3-0: data (low 4 bits of tone register, or volume/noise value)
    ///
    /// If bit 7 is clear, this is a data byte:
    ///   - For tone registers: bits 5-0 go to upper 6 bits of the latched tone register
    ///   - For volume/noise: bits 3-0 replace the value
    pub fn write(&mut self, data: u8) {
        if data & 0x80 != 0 {
            // Latch byte
            let channel = ((data >> 5) & 0x03) as usize;
            let is_volume = data & 0x10 != 0;
            let low_bits = data & 0x0F;

            self.latched_channel = channel;
            self.latched_is_volume = is_volume;

            if is_volume {
                self.volume[channel] = low_bits;
            } else if channel == 3 {
                // Noise register
                self.noise_register = low_bits & 0x07;
                self.lfsr = 0x8000;
            } else {
                // Tone register: replace low 4 bits, keep upper 6 bits
                self.tone[channel] = (self.tone[channel] & 0x3F0) | low_bits as u16;
            }
        } else {
            // Data byte
            if self.latched_is_volume {
                self.volume[self.latched_channel] = data & 0x0F;
            } else if self.latched_channel == 3 {
                // Noise register
                self.noise_register = data & 0x07;
                self.lfsr = 0x8000;
            } else {
                // Tone register: replace upper 6 bits with bits 5-0 of data, keep low 4 bits
                let low = self.tone[self.latched_channel] & 0x0F;
                self.tone[self.latched_channel] = ((data as u16 & 0x3F) << 4) | low;
            }
        }
    }

    /// Write the Game Gear stereo register (port $06).
    pub fn write_stereo(&mut self, val: u8) {
        self.stereo = val;
    }

    /// Advance one internal clock tick.
    ///
    /// For tone channels: decrement counter, reload on 0, toggle polarity.
    /// For noise channel: same but shift LFSR on rising edge.
    pub fn tick(&mut self) {
        // Tone channels 0-2
        for ch in 0..3 {
            if self.counter[ch] == 0 {
                self.counter[ch] = self.tone[ch];
                self.polarity[ch] ^= 1;
            } else {
                self.counter[ch] -= 1;
            }
        }

        // Noise channel (channel 3)
        let noise_rate = match self.noise_register & 0x03 {
            0 => 0x10_u16,
            1 => 0x20,
            2 => 0x40,
            3 => self.tone[2],
            _ => unreachable!(),
        };

        if self.counter[3] == 0 {
            self.counter[3] = noise_rate;
            let old_polarity = self.polarity[3];
            self.polarity[3] ^= 1;

            // Shift LFSR on rising edge (0 -> 1)
            if old_polarity == 0 && self.polarity[3] == 1 {
                let feedback = if self.noise_register & 0x04 != 0 {
                    // White noise: feedback = parity of (lfsr & 0x0009)
                    let tapped = self.lfsr & 0x0009;
                    (tapped.count_ones() & 1) as u16
                } else {
                    // Periodic noise: feedback = bit 0
                    self.lfsr & 1
                };
                self.lfsr = (self.lfsr >> 1) | (feedback << 15);
            }
        } else {
            self.counter[3] -= 1;
        }
    }

    /// Mix all 4 channels with stereo panning, returning (left, right) as raw i32 sums.
    fn mix_channels_raw(&self) -> (i32, i32) {
        let mut left: i32 = 0;
        let mut right: i32 = 0;

        for ch in 0..4 {
            let output = if ch < 3 {
                // Tone channels: polarity selects full volume or zero
                self.polarity[ch] as i32
            } else {
                // Noise channel: output is bit 0 of LFSR
                (self.lfsr & 1) as i32
            };

            let amplitude = VOLUME_TABLE[self.volume[ch] as usize] as i32 * output;

            // Stereo register: bits 7-4 = left (noise, tone2, tone1, tone0)
            //                  bits 3-0 = right (noise, tone2, tone1, tone0)
            let left_bit = 4 + ch;
            let right_bit = ch;

            if self.stereo & (1 << left_bit) != 0 {
                left += amplitude;
            }
            if self.stereo & (1 << right_bit) != 0 {
                right += amplitude;
            }
        }

        (left, right)
    }

    /// Mix all 4 channels with stereo panning, returning (left, right) clamped to i16 range.
    pub fn mix_channels(&self) -> (i16, i16) {
        let (left, right) = self.mix_channels_raw();
        let left = left.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let right = right.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        (left, right)
    }

    /// Generate one stereo sample at the configured sample rate.
    ///
    /// Runs the appropriate number of internal ticks per sample
    /// using a fractional accumulator for accuracy, then applies a
    /// low-pass filter to tame square wave harmonics.
    pub fn generate_sample(&mut self) -> (i16, i16) {
        self.clock_frac += self.ticks_per_sample;
        let ticks = self.clock_frac as u32;
        self.clock_frac -= ticks as f64;

        for _ in 0..ticks {
            self.tick();
        }

        // Mix raw (i32) and scale down to avoid clipping when channels overlap
        let (raw_l, raw_r) = self.mix_channels_raw();
        let sl = raw_l as f64 / 4.0;
        let sr = raw_r as f64 / 4.0;

        // First-order IIR low-pass filter (simulates analog output filtering)
        let a = self.lpf_alpha;
        self.lpf.0 += a * (sl - self.lpf.0);
        self.lpf.1 += a * (sr - self.lpf.1);

        let left = (self.lpf.0 as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let right = (self.lpf.1 as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Construction / defaults
    // ---------------------------------------------------------------

    #[test]
    fn new_initializes_volumes_silent() {
        let psg = Psg::new();
        for ch in 0..4 {
            assert_eq!(psg.volume[ch], 0x0F, "channel {} should be silent", ch);
        }
    }

    #[test]
    fn new_initializes_stereo_all_enabled() {
        let psg = Psg::new();
        assert_eq!(psg.stereo, 0xFF);
    }

    #[test]
    fn new_initializes_lfsr() {
        let psg = Psg::new();
        assert_eq!(psg.lfsr, 0x8000);
    }

    #[test]
    fn new_initializes_tone_registers_zero() {
        let psg = Psg::new();
        for ch in 0..3 {
            assert_eq!(psg.tone[ch], 0);
        }
    }

    // ---------------------------------------------------------------
    // Latch / data protocol: tone registers
    // ---------------------------------------------------------------

    #[test]
    fn latch_byte_sets_tone_low_bits() {
        let mut psg = Psg::new();
        // Latch channel 0, tone, data = 0b1010 = 0x0A
        // Bit 7=1, ch=00, type=0, data=1010 => 0b1_00_0_1010 = 0x8A
        psg.write(0x8A);
        assert_eq!(psg.tone[0] & 0x0F, 0x0A);
        assert_eq!(psg.latched_channel, 0);
        assert!(!psg.latched_is_volume);
    }

    #[test]
    fn data_byte_sets_tone_high_bits() {
        let mut psg = Psg::new();
        // Latch channel 1, tone, low bits = 0x05
        // Bit 7=1, ch=01, type=0, data=0101 => 0b1_01_0_0101 = 0xA5
        psg.write(0xA5);
        assert_eq!(psg.tone[1] & 0x0F, 0x05);

        // Data byte: upper 6 bits = 0x1F (0b011111)
        // Bit 7=0, bits 5-0 = 0b011111 = 0x1F
        psg.write(0x1F);
        // tone[1] should be (0x1F << 4) | 0x05 = 0x1F5
        assert_eq!(psg.tone[1], 0x1F5);
    }

    #[test]
    fn latch_channel_2_tone() {
        let mut psg = Psg::new();
        // Latch channel 2, tone, low bits = 0x0C
        // Bit 7=1, ch=10, type=0, data=1100 => 0b1_10_0_1100 = 0xCC
        psg.write(0xCC);
        assert_eq!(psg.latched_channel, 2);
        assert!(!psg.latched_is_volume);
        assert_eq!(psg.tone[2] & 0x0F, 0x0C);
    }

    #[test]
    fn tone_register_full_10_bit_write() {
        let mut psg = Psg::new();
        // Write tone[0] = 0x3FF (max 10-bit value)
        // Latch: ch=0, type=0, low 4 bits = 0xF => 0b1_00_0_1111 = 0x8F
        psg.write(0x8F);
        // Data: upper 6 bits = 0x3F => 0b00_111111 = 0x3F
        psg.write(0x3F);
        assert_eq!(psg.tone[0], 0x3FF);
    }

    #[test]
    fn latch_preserves_upper_bits_of_tone() {
        let mut psg = Psg::new();
        // First set tone[0] to a full 10-bit value
        psg.write(0x85); // latch ch0 tone, low=0x5
        psg.write(0x1A); // data: upper 6 bits = 0x1A => tone = (0x1A << 4) | 0x5 = 0x1A5
        assert_eq!(psg.tone[0], 0x1A5);

        // Now latch again with new low bits - upper 6 should be preserved
        psg.write(0x8B); // latch ch0 tone, low=0xB
        // Upper bits from 0x1A5: 0x1A0, new low: 0xB => 0x1AB
        assert_eq!(psg.tone[0], 0x1AB);
    }

    // ---------------------------------------------------------------
    // Latch / data protocol: volume registers
    // ---------------------------------------------------------------

    #[test]
    fn latch_byte_sets_volume() {
        let mut psg = Psg::new();
        // Latch channel 0, volume, data = 0x05
        // Bit 7=1, ch=00, type=1, data=0101 => 0b1_00_1_0101 = 0x95
        psg.write(0x95);
        assert_eq!(psg.volume[0], 0x05);
        assert_eq!(psg.latched_channel, 0);
        assert!(psg.latched_is_volume);
    }

    #[test]
    fn data_byte_sets_volume() {
        let mut psg = Psg::new();
        // Latch channel 1 volume
        // Bit 7=1, ch=01, type=1, data=0011 => 0b1_01_1_0011 = 0xB3
        psg.write(0xB3);
        assert_eq!(psg.volume[1], 0x03);

        // Data byte to change volume: bits 3-0 = 0x07
        psg.write(0x07);
        assert_eq!(psg.volume[1], 0x07);
    }

    #[test]
    fn volume_channel_3() {
        let mut psg = Psg::new();
        // Latch channel 3, volume, data = 0x0A
        // Bit 7=1, ch=11, type=1, data=1010 => 0b1_11_1_1010 = 0xFA
        psg.write(0xFA);
        assert_eq!(psg.volume[3], 0x0A);
    }

    // ---------------------------------------------------------------
    // Noise register
    // ---------------------------------------------------------------

    #[test]
    fn latch_noise_register() {
        let mut psg = Psg::new();
        // Manually set LFSR to something else to verify reset
        psg.lfsr = 0x1234;

        // Latch channel 3, tone/noise, data = 0x05
        // Bit 7=1, ch=11, type=0, data=0101 => 0b1_11_0_0101 = 0xE5
        psg.write(0xE5);
        assert_eq!(psg.noise_register, 0x05);
        assert_eq!(psg.lfsr, 0x8000, "LFSR should be reset on noise register write");
    }

    #[test]
    fn data_byte_noise_register() {
        let mut psg = Psg::new();
        // Latch channel 3, tone/noise
        psg.write(0xE0); // ch=3, type=0, data=0
        psg.lfsr = 0x1234;

        // Data byte with noise value
        psg.write(0x07);
        assert_eq!(psg.noise_register, 0x07);
        assert_eq!(psg.lfsr, 0x8000, "LFSR should be reset on noise data write");
    }

    #[test]
    fn noise_register_masked_to_3_bits() {
        let mut psg = Psg::new();
        // Latch channel 3, tone/noise, data = 0x0F (only low 3 bits should be kept: 0x07)
        psg.write(0xEF);
        assert_eq!(psg.noise_register, 0x07);
    }

    // ---------------------------------------------------------------
    // Stereo register
    // ---------------------------------------------------------------

    #[test]
    fn write_stereo_register() {
        let mut psg = Psg::new();
        psg.write_stereo(0x91);
        assert_eq!(psg.stereo, 0x91);
    }

    // ---------------------------------------------------------------
    // Silence when all volumes are off
    // ---------------------------------------------------------------

    #[test]
    fn silence_when_all_volumes_off() {
        let psg = Psg::new(); // All volumes 0x0F (silent)
        let (left, right) = psg.mix_channels();
        assert_eq!(left, 0);
        assert_eq!(right, 0);
    }

    // ---------------------------------------------------------------
    // Tick / counter behavior
    // ---------------------------------------------------------------

    #[test]
    fn tick_decrements_tone_counters() {
        let mut psg = Psg::new();
        psg.tone[0] = 100;
        psg.counter[0] = 5;

        psg.tick();
        assert_eq!(psg.counter[0], 4);
    }

    #[test]
    fn tick_reloads_and_toggles_on_zero() {
        let mut psg = Psg::new();
        psg.tone[0] = 50;
        psg.counter[0] = 0;
        psg.polarity[0] = 0;

        psg.tick();
        assert_eq!(psg.counter[0], 50, "counter should reload from tone register");
        assert_eq!(psg.polarity[0], 1, "polarity should toggle");

        // Run it down to 0 again: 50 ticks to decrement from 50 to 0,
        // then one more tick triggers reload and toggle.
        for _ in 0..51 {
            psg.tick();
        }
        assert_eq!(psg.counter[0], 50, "counter should reload again");
        assert_eq!(psg.polarity[0], 0, "polarity should toggle back");
    }

    #[test]
    fn noise_channel_rate_selection() {
        let mut psg = Psg::new();

        // Rate 0 -> 0x10
        psg.noise_register = 0x00;
        psg.counter[3] = 0;
        psg.tick();
        assert_eq!(psg.counter[3], 0x10);

        // Rate 1 -> 0x20
        psg.noise_register = 0x01;
        psg.counter[3] = 0;
        psg.tick();
        assert_eq!(psg.counter[3], 0x20);

        // Rate 2 -> 0x40
        psg.noise_register = 0x02;
        psg.counter[3] = 0;
        psg.tick();
        assert_eq!(psg.counter[3], 0x40);

        // Rate 3 -> tone[2]
        psg.noise_register = 0x03;
        psg.tone[2] = 0x123;
        psg.counter[3] = 0;
        psg.tick();
        assert_eq!(psg.counter[3], 0x123);
    }

    #[test]
    fn noise_lfsr_shifts_on_rising_edge() {
        let mut psg = Psg::new();
        psg.noise_register = 0x00; // periodic noise, rate 0x10
        psg.lfsr = 0x8000;
        psg.polarity[3] = 0;
        psg.counter[3] = 0;

        // First tick: counter=0 -> reload, polarity 0->1 (rising edge) -> LFSR shifts
        psg.tick();
        assert_eq!(psg.polarity[3], 1);
        // Periodic: feedback = bit 0 of old LFSR (0x8000 & 1 = 0)
        // LFSR = (0x8000 >> 1) | (0 << 15) = 0x4000
        assert_eq!(psg.lfsr, 0x4000);
    }

    #[test]
    fn noise_lfsr_no_shift_on_falling_edge() {
        let mut psg = Psg::new();
        psg.noise_register = 0x00;
        psg.lfsr = 0x8000;
        psg.polarity[3] = 1; // Start high
        psg.counter[3] = 0;

        // Tick: polarity 1->0 (falling edge) -> no LFSR shift
        psg.tick();
        assert_eq!(psg.polarity[3], 0);
        assert_eq!(psg.lfsr, 0x8000, "LFSR should not shift on falling edge");
    }

    #[test]
    fn noise_white_noise_feedback() {
        let mut psg = Psg::new();
        psg.noise_register = 0x04; // white noise, rate 0x10
        psg.lfsr = 0x8001; // bit 0 = 1, bit 3 = 0 => lfsr & 0x0009 = 0x0001 => parity = 1
        psg.polarity[3] = 0;
        psg.counter[3] = 0;

        psg.tick();
        // feedback = parity of (0x8001 & 0x0009) = parity of 0x0001 = 1
        // LFSR = (0x8001 >> 1) | (1 << 15) = 0x4000 | 0x8000 = 0xC000
        assert_eq!(psg.lfsr, 0xC000);
    }

    #[test]
    fn noise_white_noise_feedback_parity_even() {
        let mut psg = Psg::new();
        psg.noise_register = 0x04; // white noise
        psg.lfsr = 0x0009; // lfsr & 0x0009 = 0x0009 => parity = 2 bits set => even => feedback = 0
        psg.polarity[3] = 0;
        psg.counter[3] = 0;

        psg.tick();
        // feedback = parity of (0x0009 & 0x0009) = parity of 0x0009 = 2 bits = 0
        // LFSR = (0x0009 >> 1) | (0 << 15) = 0x0004
        assert_eq!(psg.lfsr, 0x0004);
    }

    // ---------------------------------------------------------------
    // mix_channels
    // ---------------------------------------------------------------

    #[test]
    fn mix_single_tone_channel_full_volume() {
        let mut psg = Psg::new();
        psg.volume[0] = 0; // max volume
        psg.polarity[0] = 1; // output high
        // All other channels silent (default 0x0F)

        let (left, right) = psg.mix_channels();
        assert_eq!(left, VOLUME_TABLE[0]); // 32767
        assert_eq!(right, VOLUME_TABLE[0]);
    }

    #[test]
    fn mix_channel_polarity_zero_produces_silence() {
        let mut psg = Psg::new();
        psg.volume[0] = 0; // max volume
        psg.polarity[0] = 0; // output low

        let (left, right) = psg.mix_channels();
        assert_eq!(left, 0);
        assert_eq!(right, 0);
    }

    #[test]
    fn mix_stereo_panning_left_only() {
        let mut psg = Psg::new();
        psg.volume[0] = 0;
        psg.polarity[0] = 1;
        // Enable channel 0 left only: bit 4 = 1 (left ch0), bit 0 = 0 (right ch0 off)
        // All other channels disabled
        psg.stereo = 0x10; // only bit 4 set

        let (left, right) = psg.mix_channels();
        assert_eq!(left, VOLUME_TABLE[0]);
        assert_eq!(right, 0);
    }

    #[test]
    fn mix_stereo_panning_right_only() {
        let mut psg = Psg::new();
        psg.volume[0] = 0;
        psg.polarity[0] = 1;
        // Enable channel 0 right only: bit 0 = 1
        psg.stereo = 0x01;

        let (left, right) = psg.mix_channels();
        assert_eq!(left, 0);
        assert_eq!(right, VOLUME_TABLE[0]);
    }

    #[test]
    fn mix_noise_channel_uses_lfsr_bit0() {
        let mut psg = Psg::new();
        psg.volume[3] = 0; // max volume for noise
        psg.lfsr = 0x8001; // bit 0 = 1

        let (left, right) = psg.mix_channels();
        assert_eq!(left, VOLUME_TABLE[0]);
        assert_eq!(right, VOLUME_TABLE[0]);

        // Now set LFSR with bit 0 = 0
        psg.lfsr = 0x8000;
        let (left, right) = psg.mix_channels();
        assert_eq!(left, 0);
        assert_eq!(right, 0);
    }

    #[test]
    fn mix_multiple_channels_sums() {
        let mut psg = Psg::new();
        psg.volume[0] = 0; // 32767
        psg.volume[1] = 0; // 32767
        psg.polarity[0] = 1;
        psg.polarity[1] = 1;

        let (left, right) = psg.mix_channels();
        // Both channels contribute: 32767 + 32767 = 65534, clamped to 32767
        assert_eq!(left, 32767);
        assert_eq!(right, 32767);
    }

    #[test]
    fn mix_channels_clamp_to_i16_max() {
        let mut psg = Psg::new();
        // Set 3 tone channels at max volume + polarity high
        for ch in 0..3 {
            psg.volume[ch] = 0;
            psg.polarity[ch] = 1;
        }
        // Also noise at max
        psg.volume[3] = 0;
        psg.lfsr = 0x0001; // bit 0 = 1

        let (left, right) = psg.mix_channels();
        // Sum would be 4 * 32767 = 131068, but clamped to 32767
        assert_eq!(left, 32767);
        assert_eq!(right, 32767);
    }

    // ---------------------------------------------------------------
    // generate_sample
    // ---------------------------------------------------------------

    #[test]
    fn generate_sample_runs_ticks() {
        let mut psg = Psg::new();
        psg.tone[0] = 100;
        psg.volume[0] = 0; // max volume

        // Generate one sample - this should run ~5 ticks
        let (_l, _r) = psg.generate_sample();

        // We can't know the exact state, but the clock_frac should be between 0 and 1
        assert!(psg.clock_frac >= 0.0 && psg.clock_frac < 1.0);
    }

    #[test]
    fn generate_sample_silent_output() {
        let mut psg = Psg::new();
        // All volumes are 0x0F (silent) by default

        let (left, right) = psg.generate_sample();
        assert_eq!(left, 0);
        assert_eq!(right, 0);
    }

    #[test]
    fn generate_sample_ticks_approximately_5() {
        // PSG_CLOCK / 44100 = 3579545/16/44100 ~= 5.073
        // So each sample should run about 5 ticks at the default rate
        let expected = (PSG_CLOCK / 44_100.0) as u32;
        assert!(expected == 5, "Expected ~5 ticks per sample, got {}", expected);
    }

    // ---------------------------------------------------------------
    // Volume table
    // ---------------------------------------------------------------

    #[test]
    fn volume_table_max_is_32767() {
        assert_eq!(VOLUME_TABLE[0], 32767);
    }

    #[test]
    fn volume_table_silent_is_zero() {
        assert_eq!(VOLUME_TABLE[15], 0);
    }

    #[test]
    fn volume_table_is_monotonically_decreasing() {
        for i in 0..15 {
            assert!(
                VOLUME_TABLE[i] > VOLUME_TABLE[i + 1],
                "VOLUME_TABLE[{}] ({}) should be > VOLUME_TABLE[{}] ({})",
                i,
                VOLUME_TABLE[i],
                i + 1,
                VOLUME_TABLE[i + 1]
            );
        }
    }

    // ---------------------------------------------------------------
    // Integration-style tests
    // ---------------------------------------------------------------

    #[test]
    fn write_full_tone_register_via_protocol() {
        let mut psg = Psg::new();
        // Write tone[0] = 0x1A5 (426 decimal)
        // Low 4 bits = 0x5, upper 6 bits = 0x1A
        // Latch: 0b1_00_0_0101 = 0x85
        psg.write(0x85);
        // Data:  0b00_011010 = 0x1A
        psg.write(0x1A);
        assert_eq!(psg.tone[0], 0x1A5);

        // Set volume to max
        // Latch channel 0 volume to 0: 0b1_00_1_0000 = 0x90
        psg.write(0x90);
        assert_eq!(psg.volume[0], 0);
    }

    #[test]
    fn interleaved_channel_writes() {
        let mut psg = Psg::new();

        // Write tone[0] low bits
        psg.write(0x85); // ch0, tone, low=5
        // Write tone[1] low bits (changes latched channel)
        psg.write(0xA3); // ch1, tone, low=3
        // Data byte should go to channel 1 (last latched)
        psg.write(0x10); // upper bits = 0x10
        assert_eq!(psg.tone[1], (0x10 << 4) | 0x03);
        // Channel 0 should still have only low bits set
        assert_eq!(psg.tone[0], 0x05);
    }

    #[test]
    fn noise_periodic_generates_repeating_pattern() {
        let mut psg = Psg::new();
        psg.noise_register = 0x00; // periodic, rate 0x10
        psg.lfsr = 0x8000;

        // Run enough ticks to get some LFSR shifts
        // Each shift requires a rising edge which needs 2 counter reloads
        // with rate 0x10 = 16, that's 2 * 17 = 34 ticks per shift (roughly)
        let mut lfsr_values = Vec::new();
        for _ in 0..1000 {
            psg.tick();
            if !lfsr_values.contains(&psg.lfsr) || lfsr_values.last() != Some(&psg.lfsr) {
                if lfsr_values.last() != Some(&psg.lfsr) {
                    lfsr_values.push(psg.lfsr);
                }
            }
        }
        // Periodic noise should produce a repeating pattern (not stay stuck)
        assert!(lfsr_values.len() > 1, "LFSR should shift to produce different values");
    }

    #[test]
    fn stereo_register_bit_mapping() {
        let mut psg = Psg::new();

        // Set all channels to max volume with polarity high
        for ch in 0..3 {
            psg.volume[ch] = 0;
            psg.polarity[ch] = 1;
        }
        psg.volume[3] = 0;
        psg.lfsr = 0x0001; // noise output = 1

        // Disable all outputs
        psg.stereo = 0x00;
        let (l, r) = psg.mix_channels();
        assert_eq!(l, 0);
        assert_eq!(r, 0);

        // Enable only tone0 on left (bit 4) and tone1 on right (bit 1)
        psg.stereo = (1 << 4) | (1 << 1);
        let (l, r) = psg.mix_channels();
        assert_eq!(l, VOLUME_TABLE[0]); // only tone0 on left
        assert_eq!(r, VOLUME_TABLE[0]); // only tone1 on right
    }
}
