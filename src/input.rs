// Joypad input state

/// Holds the current state of the Game Gear joypad buttons.
///
/// Each field is `true` when the corresponding button is pressed.
/// The Game Gear has a single d-pad (up/down/left/right), two action
/// buttons (button1 and button2), and a Start button.
pub struct Input {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub button1: bool,
    pub button2: bool,
    pub start: bool,
}

impl Input {
    /// Creates a new `Input` with all buttons released (not pressed).
    pub fn new() -> Self {
        Self {
            up: false,
            down: false,
            left: false,
            right: false,
            button1: false,
            button2: false,
            start: false,
        }
    }

    /// Returns the joypad byte for I/O port $DC.
    ///
    /// The Game Gear maps player 1 controls to port $DC with active-low
    /// logic: a bit is 0 when the corresponding button is pressed and 1
    /// when it is released.
    ///
    /// Bit mapping:
    /// - bit 0: Up
    /// - bit 1: Down
    /// - bit 2: Left
    /// - bit 3: Right
    /// - bit 4: Button 1 (TL)
    /// - bit 5: Button 2 (TR)
    /// - bits 6-7: always 1 (accent bits / accent unused lines)
    pub fn read_port_dc(&self) -> u8 {
        let mut value: u8 = 0xFF;

        if self.up {
            value &= !(1 << 0);
        }
        if self.down {
            value &= !(1 << 1);
        }
        if self.left {
            value &= !(1 << 2);
        }
        if self.right {
            value &= !(1 << 3);
        }
        if self.button1 {
            value &= !(1 << 4);
        }
        if self.button2 {
            value &= !(1 << 5);
        }

        value
    }

    /// Returns the byte for I/O port $DD.
    ///
    /// On the Game Gear, port $DD (player 2 / miscellaneous) is unused
    /// and always reads as 0xFF.
    pub fn read_port_dd(&self) -> u8 {
        0xFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_all_buttons_released() {
        let input = Input::new();
        assert!(!input.up);
        assert!(!input.down);
        assert!(!input.left);
        assert!(!input.right);
        assert!(!input.button1);
        assert!(!input.button2);
        assert!(!input.start);
    }

    #[test]
    fn port_dc_no_buttons_pressed() {
        let input = Input::new();
        assert_eq!(input.read_port_dc(), 0xFF);
    }

    #[test]
    fn port_dc_up_pressed() {
        let mut input = Input::new();
        input.up = true;
        // Bit 0 should be cleared: 0xFF & !0x01 = 0xFE
        assert_eq!(input.read_port_dc(), 0xFE);
    }

    #[test]
    fn port_dc_down_pressed() {
        let mut input = Input::new();
        input.down = true;
        // Bit 1 cleared: 0xFF & !0x02 = 0xFD
        assert_eq!(input.read_port_dc(), 0xFD);
    }

    #[test]
    fn port_dc_left_pressed() {
        let mut input = Input::new();
        input.left = true;
        // Bit 2 cleared: 0xFF & !0x04 = 0xFB
        assert_eq!(input.read_port_dc(), 0xFB);
    }

    #[test]
    fn port_dc_right_pressed() {
        let mut input = Input::new();
        input.right = true;
        // Bit 3 cleared: 0xFF & !0x08 = 0xF7
        assert_eq!(input.read_port_dc(), 0xF7);
    }

    #[test]
    fn port_dc_button1_pressed() {
        let mut input = Input::new();
        input.button1 = true;
        // Bit 4 cleared: 0xFF & !0x10 = 0xEF
        assert_eq!(input.read_port_dc(), 0xEF);
    }

    #[test]
    fn port_dc_button2_pressed() {
        let mut input = Input::new();
        input.button2 = true;
        // Bit 5 cleared: 0xFF & !0x20 = 0xDF
        assert_eq!(input.read_port_dc(), 0xDF);
    }

    #[test]
    fn port_dc_bits_6_7_always_high() {
        // Even with all buttons pressed, bits 6 and 7 must remain set
        let mut input = Input::new();
        input.up = true;
        input.down = true;
        input.left = true;
        input.right = true;
        input.button1 = true;
        input.button2 = true;
        let val = input.read_port_dc();
        assert_eq!(val & 0xC0, 0xC0, "bits 6-7 must always be 1");
        // Lower 6 bits should all be cleared: val = 0xC0
        assert_eq!(val, 0xC0);
    }

    #[test]
    fn port_dc_start_does_not_affect_dc() {
        // The Start button on Game Gear is read via a different mechanism
        // (port $00 bit 7), not through port $DC.
        let mut input = Input::new();
        input.start = true;
        assert_eq!(input.read_port_dc(), 0xFF);
    }

    #[test]
    fn port_dc_multiple_buttons() {
        let mut input = Input::new();
        input.up = true;
        input.button1 = true;
        // Bits 0 and 4 cleared: 0xFF & !0x01 & !0x10 = 0xEE
        assert_eq!(input.read_port_dc(), 0xEE);
    }

    #[test]
    fn port_dc_dpad_and_buttons() {
        let mut input = Input::new();
        input.left = true;
        input.right = true;
        input.button2 = true;
        // Bits 2, 3, 5 cleared: 0xFF & !0x04 & !0x08 & !0x20 = 0xD3
        assert_eq!(input.read_port_dc(), 0xD3);
    }

    #[test]
    fn port_dd_always_0xff() {
        let input = Input::new();
        assert_eq!(input.read_port_dd(), 0xFF);
    }

    #[test]
    fn port_dd_unaffected_by_buttons() {
        let mut input = Input::new();
        input.up = true;
        input.down = true;
        input.left = true;
        input.right = true;
        input.button1 = true;
        input.button2 = true;
        input.start = true;
        assert_eq!(input.read_port_dd(), 0xFF);
    }

    #[test]
    fn each_button_clears_exactly_one_bit() {
        let buttons: [(fn(&mut Input), u8); 6] = [
            (|i: &mut Input| i.up = true, 0),
            (|i: &mut Input| i.down = true, 1),
            (|i: &mut Input| i.left = true, 2),
            (|i: &mut Input| i.right = true, 3),
            (|i: &mut Input| i.button1 = true, 4),
            (|i: &mut Input| i.button2 = true, 5),
        ];

        for (set_button, expected_bit) in &buttons {
            let mut input = Input::new();
            set_button(&mut input);
            let val = input.read_port_dc();
            // Exactly one bit should differ from 0xFF
            let diff = 0xFF ^ val;
            assert_eq!(
                diff.count_ones(),
                1,
                "pressing one button should clear exactly one bit"
            );
            assert_eq!(
                diff.trailing_zeros(),
                *expected_bit as u32,
                "button should map to bit {}",
                expected_bit
            );
        }
    }
}
