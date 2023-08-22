use crate::error::IndraError;

const L1BS: u8 = 2 << 6;
const L2BS: u8 = 2 << 4;
const L3BS: u8 = 2 << 2;
const L4BS: u8 = 2;
const L1BF: u8 = 3 << 6;
const L2BF: u8 = 3 << 4;
const L3BF: u8 = 3 << 2;
const L4BF: u8 = 3;
const L1: u8 = 1 << 6;
const L2: u8 = 1 << 4;
const L3: u8 = 1 << 2;
const L4: u8 = 1;
const ONOFFLED: u8 = 1 << 6;
const BOOSTLED: u8 = 1 << 4;
const RED: u8 = 1 << 6;
const GREEN: u8 = 1 << 4;
const BLUE: u8 = 1 << 2;
const WHITE: u8 = 1;

const LOGO: u8 = 9;
const BUTTONS: u8 = 9;
const UPPERBAR: u8 = 6;
const LOWERBAR: u8 = 8;

/**
 *
 * Implement I2C
 * addr 0x60
 * let led_cmd = Led::Logg(State::Error.into)
 * write()
 */

fn test() {
    // use linux_embedded_hal::I2cdev;
    let led_cmd = Led::Logo(State::Error.into());
    // let i2c = { ... }
    // https://github.com/rust-embedded/linux-embedded-hal/blob/master/examples/transactional-i2c.rs
}

pub enum PanelLed {
    Logo,
    Buttons,
    Upper,
    Lower,
}
impl Into<u8> for PanelLed {
    fn into(self) -> u8 {
        match self {
            PanelLed::Logo => LOGO,
            PanelLed::Buttons => BUTTONS,
            PanelLed::Upper => UPPERBAR,
            PanelLed::Lower => LOWERBAR,
        }
    }
}

pub enum State {
    Error,
    Idle,
    Charging,
    V2h,
}

impl Into<LogoLed> for State {
    fn into(self) -> LogoLed {
        match self {
            State::Error => LogoLed::Red,
            State::Idle => LogoLed::White,
            State::Charging => LogoLed::Blue,
            State::V2h => LogoLed::Green,
        }
    }
}
pub struct LedStrip {
    buttons: u8,
    upper: u8,
    lower: u8,
}

impl LedStrip {
    pub fn on_led(&mut self, state: bool) -> Result<Led, IndraError> {
        self.buttons = if state {
            self.buttons | ONOFFLED
        } else {
            self.buttons & !ONOFFLED
        };
        Ok(Led::Buttons(self.buttons))
    }
    pub fn boost(&mut self, state: bool) -> Result<Led, IndraError> {
        self.buttons = if state {
            self.buttons | BOOSTLED
        } else {
            self.buttons & !BOOSTLED
        };
        Ok(Led::Buttons(self.buttons))
    }
    pub fn lower_from_percentage_animated(&mut self, val: u8) -> Result<Led, IndraError> {
        self.lower = animated_bars(val);
        Ok(Led::LowerBar(self.lower))
    }
    pub fn upper_from_percentage_animated(&mut self, val: u8) -> Result<Led, IndraError> {
        let led_val = animated_bars(val);
        self.upper = mirror_bit_pairs(led_val);
        Ok(Led::UpperBar(self.upper))
    }
    pub fn lower_from_percentage(&mut self, val: u8) -> Result<Led, IndraError> {
        self.lower = standard_bars(val);
        Ok(Led::LowerBar(self.lower))
    }
    pub fn upper_from_percentage(&mut self, val: u8) -> Result<Led, IndraError> {
        let led_val = standard_bars(val);
        self.upper = mirror_bit_pairs(led_val);
        Ok(Led::UpperBar(self.upper))
    }
}

fn animated_bars(val: u8) -> u8 {
    match val {
        0..=25 => L1BF,
        26..=50 => L1 | L2BF,
        51..=75 => L1 | L2 | L3BF,
        76..=99 => L1 | L2 | L3 | L4BF,
        _ => L1 | L2 | L3 | L4,
    }
}
fn standard_bars(val: u8) -> u8 {
    match val {
        0..=25 => L1BS,
        26..=50 => L1,
        51..=75 => L1 | L2,
        76..=99 => L1 | L2 | L3,
        _ => L1 | L2 | L3 | L4,
    }
}
pub enum Led {
    Logo(LogoLed),
    Buttons(u8),
    UpperBar(u8),
    LowerBar(u8),
}
impl Into<[u8; 2]> for Led {
    fn into(self) -> [u8; 2] {
        match self {
            Led::Logo(led) => [LOGO, led.into()],
            Led::Buttons(v) => [BUTTONS, v],
            Led::UpperBar(v) => [UPPERBAR, v],
            Led::LowerBar(v) => [LOWERBAR, v],
        }
    }
}

impl From<State> for Led {
    fn from(value: State) -> Self {
        match value {
            State::Error => Led::Logo(LogoLed::Red),
            State::Idle => Led::Logo(LogoLed::White),
            State::Charging => Led::Logo(LogoLed::Blue),
            State::V2h => Led::Logo(LogoLed::Green),
        }
    }
}
impl Led {
    fn from_panel_led(led: PanelLed, val: u8) -> Self {
        match led {
            PanelLed::Logo => Led::Logo(LogoLed::from(val)),
            PanelLed::Buttons => Led::Buttons(val),
            PanelLed::Upper => Led::UpperBar(val),
            PanelLed::Lower => Led::LowerBar(val),
        }
    }
}

pub enum LogoLed {
    Red,
    Green,
    Blue,
    White,
}
impl Into<u8> for LogoLed {
    fn into(self) -> u8 {
        match self {
            LogoLed::Red => RED,
            LogoLed::Green => GREEN,
            LogoLed::Blue => BLUE,
            LogoLed::White => WHITE,
        }
    }
}
impl From<u8> for LogoLed {
    fn from(value: u8) -> Self {
        match value {
            RED => LogoLed::Red,
            GREEN => LogoLed::Green,
            BLUE => LogoLed::Blue,
            WHITE => LogoLed::White,
            _ => panic!("Unrecognized colour value!"),
        }
    }
}

fn mirror_bit_pairs(input: u8) -> u8 {
    let mut output = 0;
    for i in 0..4 {
        let bit_pair = ((input >> (2 * i)) & 0x03) << (6 - 2 * i);
        output |= bit_pair;
    }
    output
}
