use super::can::X102;
use crate::{
    api::OperationMode, data_io::panel::Led, error::IndraError, log_error,
    pre_charger::pre_thread::PREDATA,
};
use lazy_static::lazy_static;
use log::{error, warn};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use sysfs_gpio::Pin;
use tokio::{
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
    time::sleep,
};

lazy_static! {
    pub static ref STATE: Arc<Mutex<State>> = Arc::new(Mutex::new(State(ChargerState::Idle)));
    pub static ref CHADEMO: Arc<Mutex<Chademo>> = Arc::new(Mutex::new(Chademo::default()));
    pub static ref OPERATIONAL_MODE: Arc<Mutex<OperationMode>> =
        Arc::new(Mutex::new(OperationMode::default()));
}

const D1PIN: u64 = PinVal::GPIO_P8_27 as u64; // EV external contactor
const D2PIN: u64 = PinVal::GPIO_P8_29 as u64; // EV external contactor
const C1PIN: u64 = PinVal::GPIO_P8_30 as u64; // internal contactor
const C2PIN: u64 = PinVal::GPIO_P8_32 as u64; // internal contactor
const KPIN: u64 = PinVal::GPIO_P9_16 as u64; // input - charge signal sense
pub(crate) const ONOFFPIN: u64 = PinVal::GPIO_P9_23 as u64; // input - front panel, low = pressed
pub(crate) const BOOSTPIN: u64 = PinVal::GPIO_P9_25 as u64; // input - front panel, low = pressed
pub(crate) const RESETPCAPIN: u64 = PinVal::GPIO_P8_31 as u64; // input - front panel, low = pressed
const PLUG_LOCK: u64 = PinVal::GPIO_P8_16 as u64; // Solenoid in CHAdeMO plug
pub(crate) const MASTERCONTACTOR: u64 = PinVal::GPIO_P8_12 as u64; // lockout
pub(crate) const PREACPIN: u64 = PinVal::GPIO_P8_28 as u64; // AC contactor in charger

#[derive(Default, Clone, Copy)]
pub struct Chademo {
    target_voltage: f32,
    requested_amps: f32,
    amps: f32,
    volts: f32,
    fault: bool,
    soc: u8,
    status_vehicle_contactors: bool,
    can_charge: bool,
    state: ChargerState,
}

#[allow(dead_code)]
impl Chademo {
    pub fn soc_to_voltage(&mut self) {
        assert!(self.soc() <= 100, "soc > 100%");
        let min_input = 0;
        let max_input = 98;
        let min_output = 330.0;
        let max_output = 394.0;
        let normalized_input = f32::from(self.soc() - min_input) / f32::from(max_input - min_input);
        self.set_target_voltage(min_output + (max_output - min_output) * normalized_input);
    }
    pub fn track_ev_amps(&mut self) -> f32 {
        self.requested_amps()
    }
    pub fn contrains_amps(&self, max_amps: u8) -> f32 {
        if self.soc() < 100 {
            self.requested_amps()
                .min(max_amps as f32)
                .max(-1.0 * max_amps as f32)
        } else {
            0.0
        }
    }
    pub fn soc(&self) -> u8 {
        self.soc
    }
    pub fn amps(&self) -> f32 {
        self.amps
    }
    pub fn volts(&self) -> f32 {
        self.volts
    }
    pub fn state(&self) -> ChargerState {
        self.state
    }
    pub fn update_x102(&mut self, x102: X102) {
        self.requested_amps = x102.charging_current_request as f32;
        self.target_voltage = x102.target_battery_voltage;
        self.status_vehicle_contactors = x102.status_vehicle;
        self.can_charge = x102.can_charge();
        self.soc = x102.charging_rate;
        self.fault = x102.fault();
    }

    pub fn set_amps(&mut self, amps: f32) {
        self.amps = amps;
    }

    pub fn set_volts(&mut self, volts: f32) {
        self.volts = volts;
    }

    pub fn set_state(&mut self, state: ChargerState) {
        self.state = state;
    }

    pub fn requested_amps(&self) -> f32 {
        self.requested_amps
    }

    pub fn status_vehicle_contactors(&self) -> bool {
        self.status_vehicle_contactors
    }

    pub fn fault(&self) -> bool {
        self.fault
    }

    pub fn target_voltage(&self) -> f32 {
        self.target_voltage
    }
    pub fn set_target_voltage(&mut self, val: f32) {
        self.target_voltage = val
    }

    pub fn can_charge(&self) -> bool {
        self.can_charge
    }
}

pub async fn init_state(
    receiver: Receiver<ChargerState>,
    led_sender: Sender<Led>,
) -> Result<(), IndraError> {
    use ChargerState::*;

    log::info!("Starting GPIO thread");

    let mut receiver = receiver;
    let d1pin = pin_init_out_low(D1PIN)?;
    let d2pin = pin_init_out_low(D2PIN)?;
    let c1pin = pin_init_out_low(C1PIN)?;
    let c2pin = pin_init_out_low(C2PIN)?;
    let kpin = pin_init_input(KPIN)?;
    let pluglock = pin_init_out_low(PLUG_LOCK)?;
    // let masterpin = pin_init_out_low(MASTERCONTACTOR)?;
    // let pre_ac_contactor = pin_init_out_low(PREACPIN)?;
    // let pca9552_reset = pin_init_out_high(RESETPCAPIN)?;
    let mut exiting = false;
    let c_state = STATE.clone();

    // log_error!("Enable master pin", masterpin.set_value(1));
    // log_error!("Energise PRE charger", pre_ac_contactor.set_value(1));

    loop {
        if let Some(received_state) = receiver.recv().await {
            log::info!("New state recieved: {:?}", received_state);
            let state = match received_state {
                ChargerState::Idle => {
                    log_error!("idle d1", d1pin.set_value(0));
                    log_error!("idle d2", d2pin.set_value(0));
                    log_error!("idle c1", c1pin.set_value(0));
                    log_error!("idle c2", c2pin.set_value(0));
                    // log_error!("idle plug lock", pluglock.set_value(0));
                    let _ = led_sender
                        .send(Led::Logo(crate::data_io::panel::State::Idle))
                        .await;

                    //spin
                    received_state
                }
                ChargerState::GotoIdle => {
                    // shutdown, and if safe then unlock plug and goto idle

                    if matches!(pluglock.get_value(), Ok(0)) {
                        OPERATIONAL_MODE.lock().await.idle();
                        ChargerState::Idle
                    } else {
                        warn!("Waiting for plug unlock state before idle");
                        received_state
                    }
                }
                ChargerState::Exiting => {
                    if !exiting {
                        exiting = true;

                        received_state
                    } else {
                        warn!("CTRL-C");
                        log_error!("shutdown c2", c2pin.set_value(0));
                        log_error!("shutdown c1", c1pin.set_value(0));
                        log_error!("shutdown d2", d2pin.set_value(0));
                        log_error!("shutdown d1", d1pin.set_value(0));
                        // log_error!("shutdown Pre", pre_ac_contactor.set_value(0));
                        // log_error!("shutdown Master", pre_ac_contactor.set_value(0));
                        sleep(Duration::from_secs(1)).await;
                        log_error!("idle plug lock", pluglock.set_value(0));
                        while let Ok(1) = pluglock.get_value() {
                            sleep(Duration::from_millis(100)).await;
                            warn!("Waititng for plug lock to open");
                        }
                        let _ = led_sender
                            .send(Led::Logo(crate::data_io::panel::State::Off))
                            .await;
                        sleep(Duration::from_millis(500)).await;
                        // log_error!("shutdown LEDs", pca9552_reset.set_value(0));

                        std::process::exit(0);
                    }
                }
                ChargerState::Stage1 => {
                    // Recieveing ev can data

                    log_error!("Engage plug lock", pluglock.set_value(1));
                    log_error!("Stage1 d1pin {}", d1pin.set_value(1));

                    log::warn!("                                Stage1 d1 {}", 1);
                    // GPIO change
                    if let Ok(v) = kpin.get_value() {
                        if v == 0 {
                            ChargerState::Stage2
                        } else {
                            received_state
                        }
                    } else {
                        received_state
                    }
                }
                ChargerState::Stage2 => {
                    log::warn!("                                Stage2 k {}", 0);
                    // EV has pulled K line down
                    received_state
                }
                ChargerState::Stage3 => {
                    let pre = PREDATA.lock().await;
                    if pre.get_dc_output_volts() as u16 != pre.get_dc_setpoint_volts() as u16
                        && (100.0..420.0).contains(&pre.get_dc_output_volts())
                    {
                        received_state
                    } else {
                        log::warn!("                                Stage3 d2 {}", 1);
                        let _ = d2pin.set_value(1);
                        // D2 close
                        Stage4
                    }
                }
                ChargerState::Stage4 => {
                    // Awaiting for EV signal to charge
                    let pre_dc_volts = PREDATA.lock().await.get_dc_output_volts();
                    if !(100.0..420.0).contains(&pre_dc_volts) {
                        error!("Bad Pre DC volts {}V - no precharge", pre_dc_volts);
                        received_state
                    } else {
                        received_state
                    }
                }
                ChargerState::Stage5 => {
                    // Closing contactor is voltage equalisted across contactors
                    let pre = PREDATA.lock().await;

                    if pre.get_dc_output_volts() as u16 == pre.get_dc_setpoint_volts() as u16 {
                        warn!("                                       !!!!CONTACTORS CLOSING!!!!");
                        log_error!("Contactor 1", c1pin.set_value(1));
                        log_error!("Contactor 2", c2pin.set_value(1));
                        print!("\x07");
                        warn!("                                       !!!!CONTACTORS CLOSED!!!!");
                        print!("\x07");
                        Stage6
                    } else {
                        // contactor waiting for voltage equalisations
                        warn!(
                            "pre {}v != ev {}v",
                            pre.get_dc_output_volts(),
                            pre.get_dc_setpoint_volts()
                        );
                        received_state
                    }
                }
                ChargerState::Stage6 => {
                    let pre_dc_amps = { PREDATA.lock().await.get_dc_output_amps() };
                    let charging_mode = *OPERATIONAL_MODE.clone().lock().await;
                    log::debug!("Charging mode {charging_mode:?} in {received_state:?}");
                    if matches!(charging_mode, OperationMode::Idle) {
                        GotoIdle
                    } else if pre_dc_amps as u16 != 0 && matches!(charging_mode, OperationMode::V2h)
                    {
                        Stage7
                    } else {
                        // warn!("                                       Charging mode");
                        let _ = led_sender
                            .send(Led::Logo(crate::data_io::panel::State::Charging))
                            .await;
                        Stage6
                    }
                }
                ChargerState::Stage7 => {
                    let charging_mode = *OPERATIONAL_MODE.clone().lock().await;
                    log::debug!("Charging mode {charging_mode:?} in {received_state:?}");
                    if matches!(charging_mode, OperationMode::Idle) {
                        GotoIdle
                    } else if matches!(charging_mode, OperationMode::Charge) {
                        Stage6
                    } else {
                        // warn!("                                       V2H mode");
                        let _ = led_sender
                            .send(Led::Logo(crate::data_io::panel::State::V2h))
                            .await;
                        Stage7
                    }
                }

                ChargerState::Panic => {
                    // Open contactors, end can bus communitations
                    // Print debug info and sys-exit with error
                    let _ = led_sender
                        .send(Led::Logo(crate::data_io::panel::State::Error))
                        .await;
                    log_error!("shutdown c2", release_pin(c2pin));
                    log_error!("shutdown c1", release_pin(c1pin));
                    log_error!("shutdown d2", release_pin(d2pin));
                    log_error!("shutdown d1", release_pin(d1pin));
                    // log_error!("shutdown Pre", pre_ac_contactor.set_value(0));
                    // log_error!("shutdown Master", pre_ac_contactor.set_value(0));
                    sleep(Duration::from_secs(1)).await;

                    log_error!("shutdown plug lock", pluglock.set_value(0));
                    sleep(Duration::from_secs(1)).await;
                    std::process::exit(1)
                }
            };
            *c_state.lock().await = State(state)
        }
    }
}

#[derive(Default, PartialEq, PartialOrd, Debug, Copy, Clone, Serialize, Deserialize)]
pub enum ChargerState {
    /// All open, no can tx, pre idle
    #[default]
    Idle,
    /// Halt activity
    GotoIdle,
    /// Reset state and unexport pins
    Exiting,
    ///D1 close & Lock plug (tbc)
    Stage1,
    ///K0 detected
    Stage2,
    ///D2 closed
    Stage3,
    /// Await permit charge flag from EV
    Stage4,
    ///Voltage across contactors to be closed (final)
    Stage5,
    /// Standard charge
    Stage6,
    /// V2H
    Stage7,
    /// Leaves with error led (red)
    Panic,
}

#[derive(Debug, Clone, Copy)]
pub struct State(pub ChargerState);

pub fn pin_init_out_low(pin: u64) -> Result<Pin, IndraError> {
    let pin_out_low = Pin::new(pin);
    pin_out_low
        .export()
        .map_err(|_| IndraError::PinInitError(pin))?;
    pin_out_low
        .set_direction(sysfs_gpio::Direction::Low)
        .map_err(|_| IndraError::PinInitError(pin))?;
    Ok(pin_out_low)
}
pub fn pin_init_out_high(pin: u64) -> Result<Pin, IndraError> {
    let pin_out_low = Pin::new(pin);
    pin_out_low
        .export()
        .map_err(|_| IndraError::PinInitError(pin))?;
    pin_out_low
        .set_direction(sysfs_gpio::Direction::High)
        .map_err(|_| IndraError::PinInitError(pin))?;
    Ok(pin_out_low)
}

fn pin_init_input(pin: u64) -> Result<Pin, IndraError> {
    let pin_input = Pin::new(pin);
    pin_input
        .export()
        .map_err(|_| IndraError::PinInitError(pin))?;
    pin_input
        .set_direction(sysfs_gpio::Direction::In)
        .map_err(|_| IndraError::PinInitError(pin))?;
    Ok(pin_input)
}

pub fn release_pin(pin_o: Pin) -> Result<(), IndraError> {
    let pin = pin_o.get_pin_num();
    pin_o
        .set_value(0)
        .map_err(|_| IndraError::PinInitError(pin))?;
    // Unexport the GPIO pin when done to free resources
    pin_o
        .unexport()
        .map_err(|_| IndraError::PinReleaseError(pin))?;
    Ok(())
}

#[allow(non_camel_case_types)]
#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
pub enum PinVal {
    GPIO_P8_3 = 38,
    GPIO_P8_4 = 39,
    GPIO_P8_5 = 34,
    GPIO_P8_6 = 35,
    GPIO_P8_7 = 66,
    GPIO_P8_8 = 67,
    GPIO_P8_9 = 69,
    GPIO_P8_10 = 68,
    GPIO_P8_11 = 45,
    GPIO_P8_12 = 44,
    GPIO_P8_13 = 23,
    GPIO_P8_14 = 26,
    GPIO_P8_15 = 47,
    GPIO_P8_16 = 46,
    GPIO_P8_17 = 27,
    GPIO_P8_18 = 65,
    GPIO_P8_19 = 22,
    GPIO_P8_20 = 63,
    GPIO_P8_21 = 62,
    GPIO_P8_22 = 37,
    GPIO_P8_23 = 36,
    GPIO_P8_24 = 33,
    GPIO_P8_25 = 32,
    GPIO_P8_26 = 61,
    GPIO_P8_27 = 86,
    GPIO_P8_28 = 88,
    GPIO_P8_29 = 87,
    GPIO_P8_30 = 89,
    GPIO_P8_31 = 10,
    GPIO_P8_32 = 11,
    GPIO_P8_33 = 9,
    GPIO_P8_34 = 81,
    GPIO_P8_35 = 8,
    GPIO_P8_36 = 80,
    GPIO_P8_37 = 78,
    GPIO_P8_38 = 79,
    GPIO_P8_39 = 76,
    GPIO_P8_40 = 77,
    GPIO_P8_41 = 74,
    GPIO_P8_42 = 75,
    GPIO_P8_43 = 72,
    GPIO_P8_44 = 73,
    GPIO_P8_45 = 70,
    GPIO_P8_46 = 71,
    GPIO_P9_11 = 30,
    GPIO_P9_12 = 60,
    GPIO_P9_13 = 31,
    GPIO_P9_14 = 40,
    GPIO_P9_15 = 48,
    GPIO_P9_16 = 51, // chademo 4
    GPIO_P9_17 = 4,
    GPIO_P9_18 = 5,
    GPIO_P9_21 = 3,
    GPIO_P9_22 = 2,
    GPIO_P9_23 = 49,
    GPIO_P9_24 = 15,
    GPIO_P9_25 = 117,
    GPIO_P9_26 = 14,
    GPIO_P9_27 = 125,
    GPIO_P9_28 = 123,
    GPIO_P9_29 = 121,
    GPIO_P9_30 = 122,
    GPIO_P9_31 = 120,
    GPIO_P9_41 = 20,
    GPIO_P9_42 = 7,

    // This is the hackiest of hacks.
    // To avoid having enum variants point to the same value, we just increase all of the ADC
    // variants by 1000 (and then subtract them later in the code).
    // Yeah, I'm not too proud of this one.
    AIN_0 = 1000,
    AIN_1 = 1001,
    AIN_2 = 1002,
    AIN_3 = 1003,
    AIN_4 = 1004,
    AIN_5 = 1005,
    AIN_6 = 1006,
    AIN_7 = 1007,
    // Unfortunately it seems like the pin aliases change depending on which cape is loaded,
    // meaning we'd have to implement a way to adjust the aliases.
    // That will have to wait for now.
    // See link below for some more details.
    // https://groups.google.com/d/msg/beagleboard/1mkf_s_g0vI/55aA84qNAQAJ

    // 0  EHRPWM0A  P9.22,P9.31
    // 1  EHRPWM0B  P9.21,P9.29
    // 2  ECAPPWM0  P9.42
    // 3  EHRPWM1A  P9.14,P8.36
    // 4  EHRPWM1B  P9.16,P8.34
    // 5  EHRPWM2A  P8.19,P8.45
    // 6  EHRPWM2B  P8.13,P8.46
    // 7  ECAPPWM2  P9.28

    // PWM_P = (0,0),
    // PWM_P = (0,1),
    // PWM_P = (2,0),
    // PWM_P = (2,1),
    // PWM_P = (4,0),
    // PWM_P = (4,1),
}
