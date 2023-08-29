use crate::error::IndraError;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_socketcan::CANFrame;
pub(crate) mod can;
pub(crate) mod fans;
pub(crate) mod pre_thread;
pub(crate) mod pwm;

lazy_static::lazy_static! {
    pub static ref PREDATA: Arc<Mutex<PreCharger>> = Arc::new(Mutex::new(PreCharger::default()));
}

pub const BB_PWM_CHIP: u32 = 0;
pub const BB_PWM_NUMBER: u32 = 0;

#[derive(Default, Clone, Copy, Debug)]
pub enum PreState {
    #[default]
    Offline,
    Init,
    Online,
}
#[allow(dead_code)]
impl PreState {
    pub fn is_offline(&self) -> bool {
        use PreState::*;
        matches!(self, Offline)
    }
    pub fn is_init(&self) -> bool {
        use PreState::*;
        matches!(self, Init)
    }
    pub fn is_online(&self) -> bool {
        use PreState::*;
        matches!(self, Online)
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct PreCharger {
    state: PreState,
    temp: f32,
    ac_volts: f32,
    ac_amps: f32,
    dc_output_volts: f32,
    dc_output_amps: f32,
    dc_output_volts_setpoint: f32,
    dc_output_amps_setpoint: f32,
    dc_bus_volts: f32,
    enabled: bool,
    fan_duty: u8,
    status: [u8; 2],
}

impl std::fmt::Display for PreCharger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sign = if self.dc_output_amps.is_sign_negative() {
            "Discharging"
        } else {
            "Charging EV"
        };
        write!(
            f,
            "PRE: {sign} {:.2}W, temp: {:.2}ªC dc_output: {:.2}V {:.2}A, dc_output_setpoint: {:.2}V {:.2}A, fan: {} enabled: {}",
            self.dc_output_amps * self.dc_output_volts,
            self.temp,
            self.dc_output_volts,
            self.dc_output_amps,
            self.dc_output_volts_setpoint,
            self.dc_output_amps_setpoint,
            self.fan_duty,
            self.enabled
        )
    }
}

#[allow(dead_code)]
impl PreCharger {
    pub fn set_state(&mut self, state: PreState) {
        self.state = state
    }
    pub fn get_state(&self) -> &PreState {
        &self.state
    }
    pub fn get_status(&self) -> &[u8; 2] {
        &self.status
    }
    pub fn get_temp(&self) -> f32 {
        self.temp
    }
    pub fn fan_duty(&mut self, duty: u8) {
        self.fan_duty = duty
    }
    pub fn ac_power(&self) -> f32 {
        self.ac_amps * self.ac_volts
    }

    pub fn from_slice(&mut self, s: &[u8]) -> Result<(), IndraError> {
        if s.len() != 8 {
            return Err(IndraError::BadSlice);
        }
        match s[0] {
            0x43 => {
                // Ack
                if let Ok(_this) = std::str::from_utf8(&s[4..8]) {
                    // info!("Decoded ASCII: {_this}");
                }
                return Ok(());
            }
            0x60 => {
                // Ack
                // info!("WRITEACK");
                return Ok(());
            }
            0x80 => {
                // Error resp
                log::error!("Can bad ack");
                return Err(IndraError::Error);
            }
            0x4b => (), // passthrough
            other => {
                log::error!("Bad request decode {other}");
                return Err(IndraError::Error); // change to error
            }
        }
        let val_u16 = u16::from_le_bytes([s[4], s[5]]);
        let val_i16 = i16::from_le_bytes([s[4], s[5]]);
        let addr = u16::from_le_bytes([s[1], s[2]]);

        match addr {
            0x2104 => self.temp = val_i16 as f32 * 0.1,
            0x2105 => self.ac_volts = val_u16 as f32 * 0.1,
            0x2106 => self.ac_amps = val_i16 as f32 * 0.1,
            0x2107 => self.dc_output_volts = val_u16 as f32 * 0.1,
            0x2108 => self.dc_output_amps = val_i16 as f32 * 0.1,
            0x2109 => self.dc_output_volts_setpoint = val_u16 as f32 * 0.1,
            0x210a => self.dc_output_amps_setpoint = val_i16 as f32 * 0.1,
            0x210d => self.dc_bus_volts = val_u16 as f32 * 0.1,
            0x2100 => self.enabled = 1 == s[4],
            0x2101 => {
                self.status = [s[4], s[5]];
            }
            u => {
                log::error!("Pre decode unknown address: {u:x}");
                return Err(IndraError::Error);
            }
        };
        Ok(())
    }
    pub fn get_dc_setpoint_volts(&self) -> f32 {
        self.dc_output_volts_setpoint
    }
    pub fn get_dc_setpoint_amps(&self) -> f32 {
        self.dc_output_amps_setpoint
    }
    pub fn get_dc_output_volts(&self) -> f32 {
        self.dc_output_volts
    }
    pub fn get_dc_output_amps(&self) -> f32 {
        self.dc_output_amps
    }
    pub fn get_fan_percentage(&self) -> u8 {
        self.fan_duty
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn status_ok(&self) -> bool {
        [0, 0] == self.status
    }
    pub fn volts_equal(&self) -> bool {
        let range = self.get_dc_setpoint_volts() - 2.0..=self.get_dc_setpoint_volts() + 2.0;
        range.contains(&self.get_dc_output_volts())
    }
}

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
    Unknown(u16),
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
            Register::Unknown(v) => v,
        }
    }
}
impl From<u16> for Register {
    fn from(reg: u16) -> Register {
        match reg {
            0x2104 => Register::Temp,
            0x2105 => Register::AcV,
            0x2106 => Register::AcA,
            0x2107 => Register::DcOutputV,
            0x2108 => Register::DcOutputA,
            0x2109 => Register::DcBusMaxVsetpoint,
            0x210a => Register::DcBusMaxAsetpoint,
            0x210d => Register::DcBusV,
            0x2100 => Register::Enabled,
            0x2101 => Register::Status,
            0x2150 => Register::Ping,
            unknown => Register::Unknown(unknown),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum PreCommand {
    DcVoltsSetpoint(f32),
    DcAmpsSetpoint(f32),
    Enable,
    Disable,
    Shutdown,
}
impl PreCommand {
    pub fn to_can(&self) -> CANFrame {
        let id = 0x630;
        let mut data = [0x2b, 0, 0x21, 0, 0, 0, 0, 0];

        match self {
            PreCommand::DcVoltsSetpoint(v) => {
                data[1] = 9;
                [data[4], data[5]] = u16::to_le_bytes((*v * 10.0) as u16)
            }
            PreCommand::DcAmpsSetpoint(a) => {
                if a.is_sign_negative() {
                    (data[6], data[7]) = (0xff, 0xff)
                }
                data[1] = 0xa;
                [data[4], data[5]] = i16::to_le_bytes((*a * 10.0) as i16)
            }
            PreCommand::Enable => data[4] = 0x1,
            _ => (),
        };
        CANFrame::new(id, &data, false, false).unwrap()
    }
}

#[derive(Debug)]
enum Command {
    Read,
    Write2b,
    String,
    TwoBytes,
    WriteAck,
    Error,
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
            WriteAck => 0x60,
            Error => 0x80,
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
            0x60 => WriteAck,
            0x80 => Error,
            _ => Unknown,
        }
    }
}
pub fn cmd_list() -> [u16; 10] {
    [
        u16::from(Register::Temp),
        u16::from(Register::AcA),
        u16::from(Register::DcOutputV),
        u16::from(Register::DcBusV),
        u16::from(Register::DcOutputA),
        u16::from(Register::DcBusMaxVsetpoint),
        u16::from(Register::DcBusMaxAsetpoint),
        u16::from(Register::Enabled),
        u16::from(Register::Status),
        u16::from(Register::Ping),
    ]
}
