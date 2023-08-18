use super::pre_commands::{PreCmd, Register};
use super::pwm::Pwm;
use crate::data_io::mqtt::CHADEMO_DATA;
use crate::error::PreError;
use crate::log_error;

use lazy_static::lazy_static;
use log::error;
use std::{sync::Arc, time::Duration};

use tokio::sync::{mpsc::Receiver, Barrier, Mutex};
use tokio::time::{sleep, timeout, Instant};
use tokio_socketcan::{CANFrame, CANSocket};

lazy_static! {
    pub static ref PREDATA: Arc<Mutex<PreCharger>> = Arc::new(Mutex::new(PreCharger::default()));
}

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
            "PRE: {sign} {:.2}W, temp: {:.2}ªC dc_output: {:.2}V {:.2}A, dc_output_setpoint: {:.2}V {:.2}A, enabled: {}",
            self.dc_output_amps * self.dc_output_volts,
            self.temp,
            self.dc_output_volts,
            self.dc_output_amps,
            self.dc_output_volts_setpoint,
            self.dc_output_amps_setpoint,
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
    fn from_slice(&mut self, s: &[u8]) -> Result<(), PreError> {
        if s.len() != 8 {
            return Err(PreError::BadSlice);
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
                // Ack
                error!("Can bad ack");
                return Err(PreError::Error);
            }
            0x4b => (), // passthrough
            other => {
                error!("Bad request decode {other}");
                return Err(PreError::Error); // change to error
            }
        }
        let val_u16 = u16::from_le_bytes([s[4], s[5]]);
        let val_i16 = i16::from_le_bytes([s[4], s[5]]);
        let addr = u16::from_le_bytes([s[1], s[2]]);
        // info!("Decoding addr {addr:x} u16 {val_u16} i16 {val_i16}");

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
                return Err(PreError::Error);
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
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

//  SWITCHOFFREASDON: u16 = 0x2150; // le
// async fn send_recv(
//     send: &Sender<CANFrame>,
//     recv: &mut Receiver<CANFrame>,
//     txframe: CANFrame,
//     timeout: Duration,
// ) -> Result<CANFrame, PreError> {
//     send.send(txframe)
//         .await
//         .map_err(|e| PreError::ChannelSend(e))?;
//     match tokio::select! {
//         rx = recv.recv() => rx,
//         _ = tokio::time::sleep(timeout) => None
//     } {
//         Some(f) => Ok(f),
//         None => Err(PreError::Error),
//     }
// }
async fn can_send_recv(
    can_socket: &mut CANSocket,
    txframe: CANFrame,
    timeout: Duration,
) -> Result<CANFrame, PreError> {
    use futures_util::StreamExt;

    can_socket
        .write_frame(txframe)
        .map_err(|_| PreError::CanTx(1))?
        .await
        .map_err(|_| PreError::CanTx(1))?;
    match tokio::select! {
        rx = can_socket.next() => rx,
        _ = tokio::time::sleep(timeout) => None
    } {
        Some(Ok(f)) => Ok(f),
        _ => Err(PreError::Error),
    }
}

#[derive(Default, Copy, Clone)]
struct Duty {
    val: u8,
    duration: Option<Instant>,
}

impl Duty {
    pub fn new() -> Duty {
        Duty {
            val: 0,
            duration: Some(Instant::now()),
        }
    }
    /// Returns true if time > duration
    fn elapsed(&self, time: Duration) -> bool {
        match self.duration {
            None => false,
            Some(t) => t.elapsed().cmp(&time).is_gt(),
        }
    }
}
impl Into<u8> for Duty {
    fn into(self) -> u8 {
        self.val
    }
}

struct Fan {
    duty: Duty,
    pwm: Pwm,
}

impl Fan {
    pub fn new(pwm: Pwm) -> Self {
        if pwm.export().is_err() {
            panic!("PWM EXPORT FAIL")
        }
        if pwm.enable(true).is_err() {
            if pwm.enable(false).is_err() {
                log::warn!("PWM ENABLE RETRY")
            } else if pwm.enable(true).is_err() {
                panic!("PWM ENABLE FAILED")
            }
        }
        Self {
            duty: Duty::default(),
            pwm,
        }
    }
    pub fn disable(&mut self) {
        log_error!("PWM disable", self.pwm.enable(false));
        log_error!("PWM unexport", self.pwm.unexport());
    }
    pub fn update(&mut self, temp: f32) {
        let elapsed = self.duty.elapsed(Duration::from_secs(20));
        let new_duty = Duty {
            val: self.temp_to_duty(temp),
            duration: None,
        };

        if self.duty.val != new_duty.val {
            if self.duty.val > new_duty.val && !elapsed {
                // falling -> overrun fan for 20 seconds
                return;
            }
            self.duty = if new_duty.val < 20 {
                Duty::new()
            } else {
                new_duty
            }; // pwm noise below 20%
            log_error!("Set pwm", self.pwm.set_duty(self.duty.into()));
        }
    }

    fn temp_to_duty(&self, value: impl Into<f32>) -> u8 {
        // specify voltage range against fsd soc
        const CELL100: f32 = 60.0;
        const CELL0: f32 = 35.0;

        let old_range = CELL100 - CELL0;
        let new_range = 100.0 - 0.1;
        let value: f32 = value.into();
        (((((value - CELL0) * new_range) / old_range) + 0.1) as u8).min(100)
    }
}
pub async fn init_pre(
    _name: &str,
    barrier: Arc<Barrier>,
    // send: Sender<CANFrame>,
    // recv: Receiver<CANFrame>,
    mut pre_cmd: Receiver<PreCmd>,
) -> Result<(), PreError> {
    log::info!("Starting Pre thread");
    let t100ms = Duration::from_millis(100);
    let mut pre = PreCharger::default();
    let mut can_socket =
        tokio_socketcan::CANSocket::open("can0").map_err(|e| PreError::CanOpen(e))?;

    const BB_PWM_CHIP: u32 = 7;
    const BB_PWM_NUMBER: u32 = 0;
    let pwm = Pwm::new(BB_PWM_CHIP, BB_PWM_NUMBER, 1000).unwrap(); // number depends on chip, etc.
    let mut fan = Fan::new(pwm);

    // let mut recv = recv;
    sleep(t100ms * 10).await;
    for (idx, frame) in init_frames().into_iter().enumerate() {
        loop {
            log::debug!("Pre-init stage {}/{}", idx + 1, init_frames().len());
            sleep(t100ms * 2).await;
            // match send_recv(&send, &mut recv, frame, t100ms).await {
            match can_send_recv(&mut can_socket, frame, t100ms).await {
                Ok(rx) => {
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
                Err(e) => log::error!("{e}"),
            }
        }
    }

    let frames = [
        CANFrame::new(
            0x630,
            &[0x2b, 0x0, 0x21, 0x0, 0x1, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x40, 0x0, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
    ];

    while !pre.enabled() {
        for frame in frames {
            sleep(t100ms).await;
            // if let Ok(rx) = send_recv(&send, &mut recv, frame, t100ms).await {
            if let Ok(rx) = can_send_recv(&mut can_socket, frame, t100ms).await {
                let _ = pre.from_slice(rx.data()).is_ok();
            }
        }
    }
    *PREDATA.clone().lock().await = pre; //copy data
    barrier.wait().await;

    let cmd_list = cmd_list();
    loop {
        let instant = Instant::now();

        for address in cmd_list.iter() {
            let mut p: [u8; 8] = [0u8; 8];
            p[0] = 0x40;
            [p[1], p[2]] = address.to_le_bytes();

            let frame = CANFrame::new(0x630, &p, false, false).unwrap();
            loop {
                // if let Ok(rx) = send_recv(&send, &mut recv, frame, t100ms).await {
                if let Ok(rx) = can_send_recv(&mut can_socket, frame, t100ms).await {
                    if pre.from_slice(rx.data()).is_ok() {
                        break;
                    }
                }
            }
        }

        // set heatsink fan PWM
        fan.update(pre.temp);

        while instant.elapsed().as_millis().le(&89) {
            if let Ok(Some(cmd)) = timeout(Duration::from_millis(10), pre_cmd.recv()).await {
                log::debug!("New pre_cmd {:?}", cmd);
                let frame = cmd.to_can();
                // if let Ok(rx) = send_recv(&send, &mut recv, frame, t100ms).await {
                if let Ok(rx) = can_send_recv(&mut can_socket, frame, t100ms).await {
                    if pre.from_slice(rx.data()).is_ok() {
                        *PREDATA.clone().lock().await = pre; //copy data
                    }
                };
                if matches!(cmd, PreCmd::Disable) {
                    fan.disable()
                }
                // }
            };
        }
        {
            *PREDATA.clone().lock().await = pre; //copy data
            if let Ok(mut data) = CHADEMO_DATA.try_lock() {
                data.from_pre(pre);
            };
        }
        println!("{}", pre);
    }
}
fn cmd_list() -> Vec<u16> {
    vec![
        u16::from(Register::Temp),
        u16::from(Register::AcA),
        u16::from(Register::DcOutputV),
        u16::from(Register::DcBusV),
        u16::from(Register::DcOutputA),
        u16::from(Register::DcBusMaxVsetpoint),
        u16::from(Register::DcBusMaxAsetpoint),
        u16::from(Register::Enabled),
        // u16::from(Register::Status),
        u16::from(Register::Ping),
    ]
}
fn init_frames() -> Vec<CANFrame> {
    vec![
        CANFrame::new(
            0x630,
            &[0x40, 0x8, 0x10, 0x4, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x40, 0x9, 0x10, 0x4, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x40, 0xA, 0x10, 0x4, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x40, 0x1, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        // waits until ready
        CANFrame::new(
            0x630,
            &[0x2b, 0x0, 0x21, 0x0, 0x1, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x2b, 0xa, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x2b, 0x9, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x2b, 0x0, 0x21, 0x0, 0x1, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
    ]
}
