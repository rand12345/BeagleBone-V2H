use tokio_socketcan::CANFrame;

#[derive(Debug, Clone, Copy)]
pub enum Register {
    Temp,
    AcV,
    AcA,
    DcOutputV,
    DcOutputA,
    DcBusMaxVsetpoint,
    DcBusMaxAsetpoint,
    DcBusV,
    Enabled,
    Status,
    Ping,
}

impl From<Register> for u16 {
    fn from(reg: Register) -> u16 {
        match reg {
            Register::Temp => 0x2104,
            Register::AcV => 0x2105,
            Register::AcA => 0x2106,
            Register::DcOutputV => 0x2107,
            Register::DcOutputA => 0x2108,
            Register::DcBusMaxVsetpoint => 0x2109,
            Register::DcBusMaxAsetpoint => 0x210a,
            Register::DcBusV => 0x210d,
            Register::Enabled => 0x2100,
            Register::Status => 0x2101,
            Register::Ping => 0x2150,
        }
    }
}

// pub enum ChCommand {
//     Set(PreCmd),
//     Get(PreCmd),
// }

#[derive(Debug)]
pub enum PreCmd {
    DcVoltsSetpoint(f32),
    DcAmpsSetpoint(f32),
    Enable,
    Disable,
}
impl PreCmd {
    pub fn to_can(&self) -> CANFrame {
        let id = 0x630;
        let mut data = [0x2b, 0, 0x21, 0, 0, 0, 0, 0]; // disable

        // ??[0x2b, 9, 0x21, 0, 0x88, 0x13, 0, 0]
        match self {
            PreCmd::DcVoltsSetpoint(v) => {
                data[1] = 9;
                [data[4], data[5]] = u16::to_le_bytes((*v * 10.0) as u16)
            }
            PreCmd::DcAmpsSetpoint(a) => {
                if a.is_sign_negative() {
                    (data[6], data[7]) = (0xff, 0xff)
                }
                data[1] = 0xa;
                [data[4], data[5]] = i16::to_le_bytes((*a * 10.0) as i16)
            }
            PreCmd::Enable => data[4] = 0x1,
            PreCmd::Disable => (),
        };
        CANFrame::new(id, &data, false, false).unwrap()
    }
}

enum Command {
    Read,
    Write2b,
    String,
    TwoBytes,
    Unknown,
}
impl Into<u8> for Command {
    fn into(self) -> u8 {
        use Command::*;
        match self {
            Read => 0x40,
            Write2b => 0x2b,
            String => 0x43,
            TwoBytes => 0x4b,
            Unknown => panic!("Bad data enum decode 1"),
        }
    }
}

impl From<u8> for Command {
    fn from(value: u8) -> Self {
        use Command::*;
        match value {
            0x40 => Read,
            0x2b => Write2b,
            0x43 => String,
            0x4b => TwoBytes,
            _ => Unknown,
        }
    }
}
