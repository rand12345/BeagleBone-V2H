use super::can::*;
use super::fans::*;
use super::pre_commands::PreCmd;
use super::pwm::Pwm;
use crate::data_io::mqtt::CHADEMO_DATA;
use crate::error::IndraError;
use crate::pre_charger::pre_commands::cmd_list;
use lazy_static::lazy_static;
use log::error;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc::Receiver, Barrier, Mutex};
use tokio::time::{sleep, timeout, Instant};
use tokio_socketcan::CANFrame;

lazy_static! {
    pub static ref PREDATA: Arc<Mutex<PreCharger>> = Arc::new(Mutex::new(PreCharger::default()));
}

const BB_PWM_CHIP: u32 = 0;
const BB_PWM_NUMBER: u32 = 0;

#[derive(Default, Clone, Copy, Debug)]
pub struct PreCharger {
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
    pub fn ac_power(&self) -> f32 {
        self.ac_amps * self.ac_volts
    }
    pub fn temp(&self) -> f32 {
        self.temp
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
                error!("Can bad ack");
                return Err(IndraError::Error);
            }
            0x4b => (), // passthrough
            other => {
                error!("Bad request decode {other}");
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
                error!("Pre decode unknown address: {u:x}");
                return Err(IndraError::Error);
            }
        };
        Ok(())
    }
    pub fn get_dc_setpoint_volts(&self) -> f32 {
        self.dc_output_volts_setpoint
    }
    pub fn get_dc_setpoint_amps(&self) -> f32 {
        self.dc_output_volts_setpoint
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
}

pub async fn pre_thread(
    _name: &str,
    barrier: Arc<Barrier>,
    mut pre_cmd: Receiver<PreCmd>,
) -> Result<(), IndraError> {
    log::info!("Starting Pre thread");
    let t100ms = Duration::from_millis(100);
    let mut pre = PreCharger::default();
    let mut can_socket =
        tokio_socketcan::CANSocket::open("can0").map_err(|e| IndraError::CanOpen(e))?;

    let pwm = Pwm::new(BB_PWM_CHIP, BB_PWM_NUMBER, 1000).unwrap(); // number depends on chip, etc.
    let mut fan = Fan::new(pwm);

    initalise_pre(t100ms, &mut can_socket, &mut pre).await;

    while !pre.enabled() {
        enabled_wait(t100ms, &mut can_socket, &mut pre).await;
    }

    *PREDATA.clone().lock().await = pre; // copy data
    barrier.wait().await; // start EV thread

    let cmd_list = cmd_list();
    loop {
        let instant = Instant::now();

        for address in cmd_list.iter() {
            let mut p: [u8; 8] = [0u8; 8];
            p[0] = 0x40;
            [p[1], p[2]] = address.to_le_bytes();

            let frame = CANFrame::new(0x630, &p, false, false).unwrap();
            loop {
                if let Ok(rx) = can_send_recv(&mut can_socket, frame, t100ms).await {
                    if pre.from_slice(rx.data()).is_ok() {
                        break;
                    }
                }
            }
        }

        // set heatsink fan PWM
        if pre.enabled() || pre.temp() > 55.0 {
            pre.fan_duty = fan.update(pre.temp);
        } else {
            fan.update(10.0);
            pre.fan_duty = fan.update(10.0);
        }

        while instant.elapsed().as_millis().le(&89) {
            if let Ok(Some(cmd)) = timeout(Duration::from_millis(10), pre_cmd.recv()).await {
                log::debug!("New pre_cmd {:?}", cmd);
                let frame = cmd.to_can();
                if let Ok(rx) = can_send_recv(&mut can_socket, frame, t100ms).await {
                    if pre.from_slice(rx.data()).is_ok() {
                        *PREDATA.clone().lock().await = pre; //copy data
                    }
                };
            };
        }
        {
            // update MQTT struct
            *PREDATA.clone().lock().await = pre;
            if let Ok(mut data) = CHADEMO_DATA.try_lock() {
                data.from_pre(pre);
            };
        }
        println!("{}", pre);
    }
}

async fn initalise_pre(
    t100ms: Duration,
    can_socket: &mut tokio_socketcan::CANSocket,
    pre: &mut PreCharger,
) {
    for (idx, frame) in init_frames().into_iter().enumerate() {
        loop {
            log::debug!("Pre-init stage {}/{}", idx + 1, init_frames().len());
            sleep(t100ms * 2).await;
            let rx = match can_send_recv(can_socket, frame, t100ms).await {
                Ok(rx) => rx,
                Err(e) => {
                    log::error!("{e}");
                    continue;
                }
            };

            if pre.from_slice(rx.data()).is_ok() {
                match rx.data() {
                    [0x4b, 00, 21, 00, 00, 00, 00, 00] => continue,
                    [0x4b, 01, 21, a, b, 00, 00, 00] => {
                        if (a, b) != (&0, &0) {
                            continue;
                        }
                    }
                    _ => (),
                }
                break;
            }
        }
    }
}
