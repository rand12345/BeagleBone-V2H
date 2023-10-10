use crate::{error::IndraError, global_state::OperationMode, MAX_AMPS};
use chademo_v2::*;
use lazy_static::lazy_static;
use log::warn;
use serde::Serialize;
use std::{sync::Arc, time::Duration};
use sysfs_gpio::Pin;
use tokio::{sync::Mutex, time::sleep};
use tokio_socketcan::CANFrame;

lazy_static! {
    pub static ref CHADEMO: Arc<Mutex<Chademo>> = Arc::new(Mutex::new(Chademo::new()));
}

pub const D1PIN: u64 = PinVal::GPIO_P8_27 as u64; // EV external contactor
pub const D2PIN: u64 = PinVal::GPIO_P8_29 as u64; // EV external contactor
pub const C1PIN: u64 = PinVal::GPIO_P8_30 as u64; // internal contactor
pub const C2PIN: u64 = PinVal::GPIO_P8_32 as u64; // internal contactor
pub const KPIN: u64 = PinVal::GPIO_P9_16 as u64; // input - charge signal sense
pub(crate) const ONOFFPIN: u64 = PinVal::GPIO_P9_23 as u64; // input - front panel, low = pressed
pub(crate) const BOOSTPIN: u64 = PinVal::GPIO_P9_25 as u64; // input - front panel, low = pressed
pub(crate) const RESETPCAPIN: u64 = PinVal::GPIO_P8_31 as u64; // input - front panel, low = pressed
pub const PLUG_LOCK: u64 = PinVal::GPIO_P8_16 as u64; // Solenoid in CHAdeMO plug
pub(crate) const MASTERCONTACTOR: u64 = PinVal::GPIO_P8_12 as u64; // lockout
pub(crate) const PREACPIN: u64 = PinVal::GPIO_P8_28 as u64; // AC contactor in charger

#[derive(Copy, Clone, Debug)]
pub struct Pins {
    pub d1: Pin,
    pub d2: Pin,
    pub c1: Pin,
    pub c2: Pin,
    pub k: Pin,
    pub pluglock: Pin,
    pub pre_ac: Pin,
}

impl Pins {
    fn new() -> Self {
        let d1 = pin_init_out_low(D1PIN).unwrap();
        let d2 = pin_init_out_low(D2PIN).unwrap();
        let c1 = pin_init_out_low(C1PIN).unwrap();
        let c2 = pin_init_out_low(C2PIN).unwrap();
        let k = pin_init_input(KPIN).unwrap();
        let pluglock = pin_init_out_low(PLUG_LOCK).unwrap();
        let pre_ac = pin_init_out_low(PREACPIN).unwrap();
        Self {
            d1,
            d2,
            c1,
            c2,
            k,
            pluglock,
            pre_ac,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Chademo {
    pins: Pins,
    pub x100: X100,
    pub x101: X101,
    pub x102: X102,
    pub x108: X108,
    pub x109: X109,
    pub x200: X200,
    pub x208: X208,
    pub x209: X209,
    state: OperationMode,
    amps: i16,
}

impl std::fmt::Display for Chademo {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let x102 = format!("{}", self.x102_status());
        let x109 = format!("{}", self.x109_status());
        write!(
            f,
            "x102: status {}\nx109: status {}\nd1:{:?} d2:{:?} k:{:?}, c1:{:?}, c2:{:?}, plug:{:?}, pre:{:?}\nV2x: Max Dis: (-{}A {}%) Chg: {}A (Currently {}A)",
            x102,
            x109,
            self.pins.d1.get_value(),
            self.pins.d2.get_value(),
            self.pins.k.get_value(),
            self.pins.c1.get_value(),
            self.pins.c2.get_value(),
            self.pins.pluglock.get_value(),
            self.pins.pre_ac.get_value(),
            self.requested_discharging_amps(),
            self.max_remaining_capacity_for_charging(),
            self.requested_charging_amps(),
            self.amps
        )
    }
}

impl Chademo {
    pub fn new() -> Self {
        Self {
            //EV decode
            pins: Pins::new(),
            x100: X100::default(),
            x101: X101::default(),
            x102: X102::default(),
            x200: X200::default(),
            //EVSE encode
            x109: X109::new(2, true),
            x108: X108::new(MAX_AMPS, 500, true, 435).into(),
            x208: X208::new(0, 500, MAX_AMPS, 250),
            x209: X209::new(2, 0),
            state: OperationMode::Uninitalised,
            amps: 0,
        }
    }
    /// Flag to EV that charge has been cancelled
    /// Sets 109.5.5 high
    pub fn request_stop_charge(&mut self) {
        // change status of x109
        self.x109.status.status_charger_stop_control = true;
    }
    /// Reports Pre current to EV, both charge and discharge
    pub fn update_amps(&mut self, amps: impl Into<i16>) {
        self.amps = amps.into();
        (self.x208.discharge_current, self.x109.output_current) = match self.amps.is_negative() {
            true => (self.amps.abs() as u8, 0),
            false => (0, self.amps as u8),
        };
    }

    pub fn x102_status(&self) -> X102Status {
        self.x102.status
    }
    pub fn x109_status(&self) -> X109Status {
        self.x109.status
    }

    pub fn pins(&self) -> &Pins {
        &self.pins
    }

    pub fn tx_frames(&self) -> [CANFrame; 4] {
        [
            self.x108.to_can(),
            self.x109.to_can(),
            self.x208.to_can(),
            self.x209.to_can(),
        ]
    }

    pub fn soc_to_voltage(&mut self) -> f32 {
        // assert!(self.soc <= 100, "soc > 100%");
        let min_input = 0;
        let max_input = 98;
        let min_output = 330.0;
        let max_output = 394.0;
        let normalized_input = f32::from(self.soc() - min_input) / f32::from(max_input - min_input);
        min_output + (max_output - min_output) * normalized_input
    }

    pub fn update_dynamic_charge_limits(&mut self, amps: impl Into<f32>) {
        let amps: f32 = amps.into();
        match amps.is_sign_negative() {
            true => self.set_max_discharge_amps((-1.0 * amps) as u8),
            false => self.set_max_charge_amps(amps as u8),
        }
    }
    pub fn disable_dynamic_charge_limits(&mut self) {
        self.set_max_discharge_amps(MAX_AMPS);
        self.set_max_charge_amps(MAX_AMPS);
    }

    pub fn output_volts(&self) -> &f32 {
        &self.x109.output_voltage
    }
    fn set_max_charge_amps(&mut self, amps: impl Into<u8>) {
        self.x109.output_current = amps.into();
    }
    fn set_max_discharge_amps(&mut self, amps: impl Into<u8>) {
        self.x208.set_input_current(amps.into());
    }

    /// Monitoring only
    pub fn output_amps(&self) -> &i16 {
        &self.amps
    }
    pub fn soc(&self) -> &u8 {
        &self.x102.state_of_charge
    }
    pub fn state(&self) -> &OperationMode {
        &self.state
    }

    pub fn set_state(&mut self, state: OperationMode) {
        self.state = state;
    }

    pub fn requested_charging_amps(&self) -> f32 {
        self.x102.charging_current_request as f32
    }
    pub fn requested_discharging_amps(&self) -> f32 {
        self.x200.maximum_discharge_current as f32
    }
    pub fn max_remaining_capacity_for_charging(&self) -> f32 {
        self.x200.max_remaining_capacity_for_charging as f32
    }

    pub fn status_vehicle_contactors(&self) -> bool {
        self.x102.status.status_vehicle
    }

    pub fn fault(&self) -> bool {
        self.x102.fault().into()
    }

    pub fn target_voltage(&self) -> &f32 {
        &self.x102.target_battery_voltage
    }

    // all below this needs verifying
    pub fn can_charge(&self) -> bool {
        self.x102.can_close_contactors()
    }

    // if kline and the other line bool true

    pub fn k_line_check(&mut self) -> Result<bool, IndraError> {
        Ok(self
            .pins
            .k
            .get_value()
            .map_err(|e| IndraError::PinAccess(e))?
            == 0
            && self.x102.can_close_contactors())
    }
    pub fn charge_start(&mut self) {
        self.x109.status.status_charger_stop_control = false;
        self.x109.status.status_station = true;
        // self.status_station_enabled(true);
        // self.plug_lock(true);
        self.x109.remaining_charging_time_10s_bit = 255;
        self.x109.remaining_charging_time_1min_bit = 60;
    }
    pub fn charge_stop(&mut self) {
        // self.status_charger_stop_control(true);
        // self.status_station_enabled(false);
        // self.plug_lock(false);
        self.x109.output_voltage = 0.0;
        self.x109.output_current = 0;
        self.x109.remaining_charging_time_10s_bit = 0;
        self.x109.remaining_charging_time_1min_bit = 0;
        self.x109.status.fault_battery_incompatibility = false;
        self.x109.status.fault_charging_system_malfunction = false;
        self.x109.status.fault_station_malfunction = false;
    }
    // pub fn status_charger_stop_control(&mut self, state: bool) {
    //     self.x109.status.status_charger_stop_control = state
    // }

    // pub fn status_station_enabled(&mut self, state: bool) {
    //     self.x109.status.status_station = state;
    // }
    pub fn plug_lock(&mut self, state: bool) -> Result<(), IndraError> {
        self.pins
            .pluglock
            .set_value(state.into())
            .map_err(|e| IndraError::PinAccess(e))?;
        self.x109.status.status_vehicle_connector_lock = state; // unsure
        Ok(())
    }
    pub fn status_vehicle_charging(&self) -> bool {
        self.x102.status.status_vehicle_charging
    }
    pub fn status_vehicle_ok(&self) -> bool {
        !self.x102.status.status_vehicle
    }
    pub fn charging_stop_control_set(&mut self) {
        self.x109.status.status_charger_stop_control = false
    }
    pub fn charging_stop_control_release(&mut self) {
        self.x109.status.status_charger_stop_control = true
    }
    pub fn close_contactors(&mut self) -> Result<(), IndraError> {
        log::info!("Contactors closing");
        self.pins()
            .c1
            .set_value(1)
            .map_err(|e| IndraError::PinAccess(e))?;
        self.pins()
            .c2
            .set_value(1)
            .map_err(|e| IndraError::PinAccess(e))?;

        print!("\x07");
        //109.5.5
        self.charging_stop_control_set();
        Ok(())
    }
}

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

pub fn pin_init_input(pin: u64) -> Result<Pin, IndraError> {
    let pin_input = Pin::new(pin);
    pin_input
        .export()
        .map_err(|_| IndraError::PinInitError(pin))?;
    pin_input
        .set_direction(sysfs_gpio::Direction::In)
        .map_err(|_| IndraError::PinInitError(pin))?;
    Ok(pin_input)
}

pub fn _release_pin(pin_o: Pin) -> Result<(), IndraError> {
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
pub(crate) enum PinVal {
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
#[cfg(test)]
mod test {
    use chademo_v2::X109;
    use tokio_socketcan::CANFrame;

    use super::*;
    #[test]
    fn soc_test() {
        let frame = CANFrame::new(
            0x102,
            [0x2, 0x9A, 0x1, 0x0E, 0x0, 0xC1, 0x56, 0x0].as_slice(),
            false,
            false,
        )
        .unwrap();

        let x109 = X109::new(2, true);
        let mut chademo = Chademo::new();
        chademo.x109 = x109;
        chademo.x102 = X102::from(&frame);
        assert_eq!(chademo.soc(), &79)
    }
    #[test]
    fn x208_test() {
        let y = X208::new(1, 500, 16, 250);
        println!(
            "{} {} {} {}",
            y.get_discharge_current(),
            y.get_input_voltage(),
            y.get_input_current(),
            y.get_lower_threshold_voltage()
        );
        assert!(y.get_discharge_current() == 1);
        assert!(y.get_input_voltage() == 500);
        assert!(y.get_input_current() == 16);
        assert!(y.get_lower_threshold_voltage() == 250);
        let cf: CANFrame = y.to_can();
        assert!(cf.data()[0] == 0xff - 1);
        assert!(cf.data()[3] == 0xff - 16);

        let y: X208 = X208::from(&cf);
        println!(
            "{} {} {} {}",
            y.get_discharge_current(),
            y.get_input_voltage(),
            y.get_input_current(),
            y.get_lower_threshold_voltage()
        );
        assert!(y.get_discharge_current() == 1);
        assert!(y.get_input_voltage() == 500);
        assert!(y.get_input_current() == 16);
        assert!(y.get_lower_threshold_voltage() == 250);
    }
}
