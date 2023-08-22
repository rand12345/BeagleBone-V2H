use super::state::Chademo;
use crate::MAX_AMPS;
use std::ops::ControlFlow;
use tokio_socketcan::{CANFrame, CANSocket};

const X208: &[u8; 8] = &[0xFF, 0xF4, 0x01, 0xF0, 0x00, 0x00, 0xFA, 0x00];
const X209IDLE: &[u8; 8] = &[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
const X209CHARGING: &[u8; 8] = &[0x02, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct X100 {
    pub maximum_battery_voltage: f32,
    pub constant_of_charging_rate_indication: u8,
    pub minimum_charge_current: u8,
}

impl From<&[u8]> for X100 {
    fn from(data: &[u8]) -> Self {
        X100 {
            maximum_battery_voltage: u16::from_le_bytes(data[4..=5].try_into().unwrap()) as f32,
            constant_of_charging_rate_indication: data[6],
            minimum_charge_current: data[0],
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct X101 {
    max_charging_time_10s_bit: u8,
    max_charging_time_1min_bit: u8,
    estimated_charging_time: u8,
    rated_battery_capacity: f32,
}

impl From<&[u8]> for X101 {
    fn from(data: &[u8]) -> Self {
        X101 {
            max_charging_time_10s_bit: data[1],
            max_charging_time_1min_bit: data[2],
            estimated_charging_time: data[3],
            rated_battery_capacity: u16::from_le_bytes(data[5..=6].try_into().unwrap()) as f32,
        }
    }
}

// EV decoder
#[derive(Debug, Default, Copy, Clone)]
pub struct X102 {
    pub control_protocol_number_ev: u8,
    pub target_battery_voltage: f32,
    pub charging_current_request: u8,
    pub fault_battery_voltage_deviation: bool,
    pub fault_high_battery_temperature: bool,
    pub fault_battery_current_deviation: bool,
    pub fault_battery_undervoltage: bool,
    pub fault_battery_overvoltage: bool,
    pub status_normal_stop_request: bool,
    pub status_vehicle: bool,                  // true EV contactors open
    pub status_charging_system: bool,          // false = ok / true = fault
    pub status_vehicle_shifter_position: bool, // false = ok
    pub status_vehicle_charging: bool,
    pub charging_rate: u8,
}
impl X102 {
    pub fn fault(&self) -> bool {
        self.fault_battery_voltage_deviation
            | self.fault_high_battery_temperature
            | self.fault_battery_current_deviation
            | self.fault_battery_current_deviation
            | self.fault_battery_undervoltage
            | self.fault_battery_overvoltage
    }
    pub fn can_charge(&self) -> bool {
        !(self.status_normal_stop_request
            | self.status_vehicle
            | self.status_charging_system
            | self.status_vehicle_shifter_position)
    }
}

impl From<&[u8]> for X102 {
    fn from(data: &[u8]) -> Self {
        X102 {
            control_protocol_number_ev: data[0],
            target_battery_voltage: u16::from_le_bytes(data[1..=2].try_into().unwrap()) as f32,
            charging_current_request: data[3],
            fault_battery_voltage_deviation: get_bit(data[4], 0),
            fault_high_battery_temperature: get_bit(data[4], 1),
            fault_battery_current_deviation: get_bit(data[4], 2),
            fault_battery_undervoltage: get_bit(data[4], 3),
            fault_battery_overvoltage: get_bit(data[4], 4),
            status_normal_stop_request: get_bit(data[5], 4),
            status_vehicle: get_bit(data[5], 3),
            status_charging_system: get_bit(data[5], 2),
            status_vehicle_shifter_position: get_bit(data[5], 1),
            status_vehicle_charging: get_bit(data[5], 0),
            charging_rate: data[6],
        }
    }
}

// EVSE TX struct
#[derive(Debug)]
pub struct X108 {
    pub available_output_current: u8,
    pub avaible_output_voltage: u16,
    pub welding_detection: u8,
    pub threshold_voltage: u16,
}
impl X108 {
    pub fn new() -> Self {
        Self {
            available_output_current: MAX_AMPS,
            avaible_output_voltage: 500,
            welding_detection: 1,
            threshold_voltage: 435,
        }
    }

    pub fn to_can(&self) -> CANFrame {
        let aov = self.avaible_output_voltage.to_le_bytes();
        let tv = self.threshold_voltage.to_le_bytes();
        CANFrame::new(
            0x108,
            &[
                self.welding_detection,
                aov[0],
                aov[1],
                self.available_output_current,
                tv[0],
                tv[1],
                0,
                0,
            ],
            false,
            false,
        )
        .unwrap()
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct X109 {
    control_protocol_number_qc: u8,
    pub(crate) output_voltage: f32,
    pub(crate) output_current: f32,
    status_charger_stop_control: bool,
    fault_charging_system_malfunction: bool,
    fault_battery_incompatibility: bool,
    status_vehicle_connector_lock: bool,
    fault_station_malfunction: bool,
    status_station: bool,
    remaining_charging_time_10s_bit: u8,
    remaining_charging_time_1min_bit: u8,
}

impl X109 {
    pub(crate) fn charge_start(&mut self) {
        self.status_charger_stop_control(false);
        self.status_station(true);
        self.plug_lock(true);
        self.remaining_charging_time_10s_bit = 255;
        self.remaining_charging_time_1min_bit = 60;
    }
    pub(crate) fn precharge(&mut self) {
        self.status_charger_stop_control(true);
        self.status_station(false);
        self.plug_lock(true);
    }
    pub(crate) fn charge_halt(&mut self) {
        self.status_charger_stop_control(true);
    }
    pub(crate) fn charge_stop(&mut self) {
        self.status_charger_stop_control(true);
        self.status_station(false);
        // self.plug_lock(false);
        self.output_voltage = 0.0;
        self.output_current = 0.0;
        self.remaining_charging_time_10s_bit = 0;
        self.remaining_charging_time_1min_bit = 0;
        self.fault_battery_incompatibility = false;
        self.fault_charging_system_malfunction = false;
        self.fault_station_malfunction = false;
    }
    pub(crate) fn status_charger_stop_control(&mut self, state: bool) {
        self.status_charger_stop_control = state
    }
    pub(crate) fn status_station(&mut self, state: bool) {
        self.status_station = state;
    }
    pub(crate) fn plug_lock(&mut self, state: bool) {
        self.status_vehicle_connector_lock = state; // unsure
    }
    pub fn new(control_protocol_number_qc: u8) -> Self {
        Self {
            control_protocol_number_qc,
            remaining_charging_time_10s_bit: 255,
            remaining_charging_time_1min_bit: 255,
            ..Default::default()
        }
    }
    fn to_can(self) -> CANFrame {
        let mut result = vec![0u8; 8];

        result[0] = self.control_protocol_number_qc;
        let voltage_bytes: [u8; 2] = ((self.output_voltage) as u16).to_le_bytes();
        result[1..=2].copy_from_slice(&voltage_bytes);
        result[3] = (self.output_current) as u8;
        result[4] = 0x1;
        result[5] |= (self.status_charger_stop_control as u8) << 5;
        result[5] |= (self.fault_charging_system_malfunction as u8) << 4;
        result[5] |= (self.fault_battery_incompatibility as u8) << 3;
        result[5] |= (self.status_vehicle_connector_lock as u8) << 2;
        result[5] |= (self.fault_station_malfunction as u8) << 1;
        result[5] |= (self.status_station as u8) << 0;

        result[6] = self.remaining_charging_time_10s_bit;
        result[7] = self.remaining_charging_time_1min_bit;

        CANFrame::new(0x109, &result, false, false).unwrap()
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct X200 {
    pub maximum_discharge_current: u8,
    pub minimum_discharge_voltage: f32,
    pub minimum_battery_discharge_level: u8,
    pub max_remaining_capacity_for_charging: u8,
}

impl From<&[u8]> for X200 {
    fn from(data: &[u8]) -> Self {
        X200 {
            maximum_discharge_current: data[0],
            minimum_discharge_voltage: u16::from_le_bytes(data[4..=5].try_into().unwrap()) as f32,
            minimum_battery_discharge_level: data[6],
            max_remaining_capacity_for_charging: data[7],
        }
    }
}

fn get_bit(byte: u8, position: u8) -> bool {
    (byte & (1 << position)) != 0
}

pub async fn send_can_data(can: &CANSocket, x108: &X108, x109: &X109, contactor_matched: bool) {
    let f = |(id, d)| CANFrame::new(id, d, false, false).unwrap();

    let frames: [CANFrame; 4] = if !contactor_matched {
        [
            x108.to_can(),
            x109.to_can(),
            f((0x208, X208)),
            f((0x209, X209IDLE)),
        ]
    } else {
        [
            x108.to_can(),
            x109.to_can(),
            f((0x208, X208)),
            f((0x209, X209CHARGING)),
        ]
    };
    for frame in frames {
        if let Err(e) = can.write_frame(frame).unwrap().await {
            log::error!("{}", e)
        };
    }
}

pub fn rx_can(frame: CANFrame, x102: &mut X102, chademo: &mut Chademo) -> ControlFlow<()> {
    match frame.id() {
        0x102 => {
            *x102 = X102::from(&frame.data()[..8]);
            if x102.fault() {
                log::error!("                                              x102 FAULT!");
            }
            chademo.update_x102(*x102);
            return ControlFlow::Break(());
        }
        0x200 => {
            // start transmitting frames
        }
        _ => return ControlFlow::Break(()), //log::error!("ev rx match id: {:x}", bad),
    }
    ControlFlow::Continue(())
}
