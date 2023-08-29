use tokio_socketcan::CANFrame;

/// Vehicle CAN frame
#[derive(Debug, Default, Copy, Clone)]
pub struct X100 {
    /// Set “minimum current” defined by vehicle
    pub minimum_charge_current: u8,
    /// Lower limit voltage for backup to stop by a charger
    pub minimum_battery_voltage: f32,
    /// Upper limit voltage for backup to stop by a charger
    pub maximum_battery_voltage: f32,
    /// Set fixed value (0x64: 100 %) related to charged rate
    pub constant_of_charging_rate_indication: u8,
}

impl From<&CANFrame> for X100 {
    fn from(frame: &CANFrame) -> Self {
        let data = data_sanity(&frame, 0x100, 8);
        X100 {
            minimum_battery_voltage: u16::from_le_bytes(data[2..=3].try_into().unwrap()) as f32,
            maximum_battery_voltage: u16::from_le_bytes(data[4..=5].try_into().unwrap()) as f32,
            constant_of_charging_rate_indication: data[6],
            minimum_charge_current: data[0],
        }
    }
}

/// Vehicle CAN frame
#[allow(dead_code)]
#[derive(Debug, Default, Copy, Clone)]
pub struct X101 {
    /// Maximum charging time that vehicle permits charger
    max_charging_time_10s_bit: u8,
    /// Maximum charging time that vehicle permits charger
    max_charging_time_1min_bit: u8,
    /// Estimated time until stop of charging
    estimated_charging_time: u8,
    /// Set total capacity of battery
    rated_battery_capacity: f32,
}

impl From<&CANFrame> for X101 {
    fn from(frame: &CANFrame) -> Self {
        let data = data_sanity(&frame, 0x101, 8);
        X101 {
            max_charging_time_10s_bit: data[1],
            max_charging_time_1min_bit: data[2],
            estimated_charging_time: data[3],
            rated_battery_capacity: u16::from_le_bytes(data[5..=6].try_into().unwrap()) as f32,
        }
    }
}

/// Vehicle CAN frame
#[derive(Debug, Default, Copy, Clone)]
pub struct X102 {
    /// CHAdeMO protocol number
    pub control_protocol_number_ev: u8,
    /// Target value of charging voltage
    pub target_battery_voltage: f32,
    /// Charging current request
    pub charging_current_request: u8,
    pub faults: X102Faults,
    pub status: X102Status,
    /// state of charge of battery
    pub state_of_charge: u8,
}
impl X102 {
    pub fn fault(&self) -> bool {
        self.faults.into()
    }
    pub fn can_charge(&self) -> bool {
        !(self.status.status_normal_stop_request
            | self.status.status_vehicle
            | self.status.status_charging_system
            | self.status.status_vehicle_shifter_position)
    }
}

impl From<&CANFrame> for X102 {
    fn from(frame: &CANFrame) -> X102 {
        let data = data_sanity(&frame, 0x102, 8);
        X102 {
            control_protocol_number_ev: data[0],
            target_battery_voltage: u16::from_le_bytes(data[1..=2].try_into().unwrap()) as f32,
            charging_current_request: data[3],
            faults: From::from(data[4]),
            status: From::from(data[5]),
            state_of_charge: data[6],
        }
    }
}

#[derive(Debug, Default, Copy, Clone)]
pub struct X102Faults {
    pub fault_battery_voltage_deviation: bool,
    pub fault_high_battery_temperature: bool,
    pub fault_battery_current_deviation: bool,
    pub fault_battery_undervoltage: bool,
    pub fault_battery_overvoltage: bool,
}
impl Into<bool> for X102Faults {
    fn into(self) -> bool {
        self.fault_battery_voltage_deviation
            | self.fault_high_battery_temperature
            | self.fault_battery_current_deviation
            | self.fault_battery_undervoltage
            | self.fault_battery_overvoltage
    }
}

impl From<u8> for X102Faults {
    fn from(value: u8) -> Self {
        Self {
            fault_battery_overvoltage: get_bit(value, 4),
            fault_battery_undervoltage: get_bit(value, 3),
            fault_battery_current_deviation: get_bit(value, 2),
            fault_high_battery_temperature: get_bit(value, 1),
            fault_battery_voltage_deviation: get_bit(value, 0),
        }
    }
}

#[derive(Debug, Default, Copy, Clone)]
pub struct X102Status {
    /// 102.5.4
    pub status_normal_stop_request: bool,
    /// 102.5.3
    pub status_vehicle: bool, // true EV contactors open
    /// 102.5.2
    pub status_charging_system: bool, // false = ok / true = fault
    /// 102.5.1
    pub status_vehicle_shifter_position: bool, // false = ok
    /// 102.5.0
    pub status_vehicle_charging: bool,
}
impl std::fmt::Display for X102Status {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "102.5.0:{} 1:{} 2:{} 3:{} 4:{}",
            self.status_vehicle_charging as u8,
            self.status_vehicle_shifter_position as u8,
            self.status_charging_system as u8,
            self.status_vehicle as u8,
            self.status_normal_stop_request as u8
        )
    }
}
impl From<u8> for X102Status {
    fn from(val: u8) -> Self {
        Self {
            status_normal_stop_request: get_bit(val, 4),
            status_vehicle: get_bit(val, 3),
            status_charging_system: get_bit(val, 2),
            status_vehicle_shifter_position: get_bit(val, 1),
            status_vehicle_charging: get_bit(val, 0),
        }
    }
}

/// EVSE CAN frame
#[derive(Debug, Copy, Clone)]
pub struct X108 {
    /// 108.3 - Current that the EVSE can output at present. This value shall be set from the initial CAN communication. The initial value shall be the maximum current that can be output by the EVSE, and during the charging/discharging, the value shall be updated from time to time as the current which can be output by the EVSE.
    /// The smaller value between this value and the “maximum charge current” shall be set as the target charge current.
    pub available_output_current: u8,
    /// 108.1-2 - Maximum output voltage value of the EVSE. Set the number from initial CAN data transmission and do not update it.
    /// If the EVSE receives “target battery voltage” exceeding this value from the vehicle, regard this situation as “Battery incompatible” and shift to charge termination process.
    pub avaible_output_voltage: u16,
    /// 108.0 - Identifier indicating characteristic of output circuit of EVSE which corresponds to welding detection of EV contactor.
    pub welding_detection: u8,
    /// 108.4-5 - Judgmental voltage value to stop charging process for on-board battery protection. This flag may be updated until the initial value of charging current request is sent from the vehicle.
    /// — The EVSE shall compare vehicle CAN “maximum battery voltage” with charger CAN “available output voltage,” set the lower value to this value. — When circuit voltage reaches to this value, the EVSE stops charging output.
    pub threshold_voltage: u16,
}

impl X108 {
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
    pub fn new(
        available_output_current: u8,
        avaible_output_voltage: u16,
        welding_detection: bool,
        threshold_voltage: u16,
    ) -> Self {
        Self {
            available_output_current,
            avaible_output_voltage,
            welding_detection: welding_detection.into(),
            threshold_voltage,
        }
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct X109Status {
    /// 109.5.5 - Set this flag to 1 before charging (e.g., initial value and during insulation test). Change this flag to 0 from 1 after shifting to the start of charging control. Also, both the timing that the “charging stop control (H’109.5.5)” is changed to 0 from 1 and the timing that the “charger status (H’109.5.0)” is changed to 1 from 0 shall be in an exclusive relation. Set 1 from 0 to this flag in case the charging sequence shifts to stop process (including a state of stop process).
    pub status_charger_stop_control: bool,
    /// 109.5.4 - Error flag indicating vehicle error or charger error. Charger shall detect error and shall shift to error stop process in case this flag is set to 1.
    pub fault_charging_system_malfunction: bool,
    /// 109.5.3 -Error flag indicating “available output voltage” of charger which is not suitable for charging to traction battery. - Set 1 to this flag in case “target battery voltage (H’102.1, H’102.2)” of vehicle exceeds “available output voltage (H’108.1, H’108.2)” or “Minimum battery voltage (H’100.2, H’ 100.3)” of vehicle is below “output voltage lower limit." Charger shall detect error and shall shift to error stop process in case this flag is set to 1.
    pub fault_battery_incompatibility: bool,
    /// 109.5.2 - Status flag indicating a state in which voltage can be applied from charger or a state in which output charging is permitted. - Set 1 to this flag when vehicle permits charger to charge and/or voltage in output circuit exceeds 10 V. Set 0 to this flag when vehicle prohibits charger to charge and/or voltage in output circuit is less than or equal to 10 V.
    pub status_vehicle_connector_lock: bool,
    /// 109.5.1 - Error flag indicating charger’s error detected by charger - Charger shall detect error and shall shift to error stop process in case this flag is set to 1.
    pub fault_station_malfunction: bool,
    /// 109.5.0 - Status flag indicating charging - Set 0 to this flag before charging (e.g., initial value, during insulation test) and at the end of the charging (shifting to stop process and charging current decreases less than or equal to 5 A). Set 1 to this flag during charging
    pub status_station: bool,
}
impl std::fmt::Display for X109Status {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "109.5.0:{} 1:{} 2:{} 3:{} 4:{} 5:{}",
            self.status_station as u8,
            self.fault_station_malfunction as u8,
            self.status_vehicle_connector_lock as u8,
            self.fault_battery_incompatibility as u8,
            self.fault_charging_system_malfunction as u8,
            self.status_charger_stop_control as u8
        )
    }
}
impl Into<u8> for X109Status {
    fn into(self) -> u8 {
        let mut result = 0u8;
        result |= (self.status_charger_stop_control as u8) << 5;
        result |= (self.fault_charging_system_malfunction as u8) << 4;
        result |= (self.fault_battery_incompatibility as u8) << 3;
        result |= (self.status_vehicle_connector_lock as u8) << 2;
        result |= (self.fault_station_malfunction as u8) << 1;
        result |= (self.status_station as u8) << 0;
        result
    }
}
impl From<u8> for X109Status {
    fn from(value: u8) -> Self {
        let mut x109status = X109Status::default();
        x109status.status_charger_stop_control = get_bit(value, 5);
        x109status.fault_charging_system_malfunction = get_bit(value, 4);
        x109status.fault_battery_incompatibility = get_bit(value, 3);
        x109status.status_vehicle_connector_lock = get_bit(value, 2);
        x109status.fault_station_malfunction = get_bit(value, 1);
        x109status.status_station = get_bit(value, 0);
        x109status
    }
}
/// EVSE CAN frame
#[derive(Default, Debug, Clone, Copy)]
pub struct X109 {
    pub status: X109Status,
    control_protocol_number_qc: u8,
    pub output_voltage: f32,
    pub output_current: u8,
    discharge_compatitiblity: bool,
    pub remaining_charging_time_10s_bit: u8,
    pub remaining_charging_time_1min_bit: u8,
}

impl X109 {
    pub fn to_can(&self) -> CANFrame {
        let mut result = [0u8; 8];

        result[0] = self.control_protocol_number_qc;
        let voltage_bytes: [u8; 2] = ((self.output_voltage) as u16).to_le_bytes();
        result[1..=2].copy_from_slice(&voltage_bytes);
        result[3] = (self.output_current) as u8;
        result[4] = self.discharge_compatitiblity.into(); // EVSE discharge compatitbility flag
        result[5] = self.status.into();
        result[6] = self.remaining_charging_time_10s_bit;
        result[7] = self.remaining_charging_time_1min_bit;

        CANFrame::new(0x109, &result, false, false).unwrap()
    }
    pub fn new(control_protocol_number_qc: u8, discharge_compatitiblity: bool) -> Self {
        let mut status = X109Status::default();
        status.status_charger_stop_control = true;
        Self {
            control_protocol_number_qc,
            discharge_compatitiblity,
            remaining_charging_time_10s_bit: 255,
            remaining_charging_time_1min_bit: 255,
            status,
            ..Default::default()
        }
    }
}

impl From<&CANFrame> for X109 {
    fn from(frame: &CANFrame) -> X109 {
        let data = data_sanity(&frame, 0x109, 8);
        let mut x109 = Self {
            ..Default::default()
        };
        x109.control_protocol_number_qc = data[0];
        x109.output_voltage = u16::from_le_bytes([data[1], data[2]]) as f32;
        x109.output_current = data[3];
        x109.status = data[5].into();
        x109.remaining_charging_time_10s_bit = data[6];
        x109.remaining_charging_time_1min_bit = data[7];
        x109
    }
}

// Vehicle can frame
#[derive(Debug)]
pub struct X200 {
    pub maximum_discharge_current: i16,
    pub minimum_discharge_voltage: f32,
    pub minimum_battery_discharge_level: i16,
    pub max_remaining_capacity_for_charging: u8,
}

impl From<&CANFrame> for X200 {
    fn from(frame: &CANFrame) -> Self {
        let data = data_sanity(&frame, 0x200, 8);
        X200 {
            maximum_discharge_current: data[0] as i16 - 255,
            minimum_discharge_voltage: u16::from_le_bytes(data[4..=5].try_into().unwrap()) as f32,
            minimum_battery_discharge_level: data[6] as i16 - 255,
            max_remaining_capacity_for_charging: data[7],
        }
    }
}

/// EVSE V2x

#[derive(Debug, Clone, Copy)]
pub struct X208 {
    /// The circuit current measured by the EVSE.
    pub discharge_current: i16,
    /// The minimum voltage with which the EVSE can operate.
    input_voltage: u16,
    /// The current with which the EVSE stops discharging in order to protect the circuit
    input_current: i16,
    /// The voltage with which the EVSE shall stop when the vehicle cannot stop at the minimum discharge voltage of the vehicle system due to a fault.
    lower_threshold_voltage: u16,
}
impl X208 {
    pub fn to_can(&self) -> CANFrame {
        let mut data = [0u8; 8];

        data[0] = (0xff + (self.discharge_current).clamp(-254, 0)) as u8;
        [data[1], data[2]] = self.input_voltage.to_le_bytes();
        data[3] = (0xff + (self.input_current).clamp(-254, 0)) as u8;
        [data[6], data[7]] = self.lower_threshold_voltage.to_le_bytes();
        CANFrame::new(0x208, &data, false, false).unwrap()
    }
    /// negative is discharge
    pub fn new(
        discharge_current: i16,
        input_voltage: u16,
        input_current: i16,
        lower_threshold_voltage: u16,
    ) -> Self {
        Self {
            discharge_current,
            input_voltage,
            input_current,
            lower_threshold_voltage,
        }
    }

    pub fn get_input_voltage(&self) -> u16 {
        self.input_voltage
    }
    pub fn get_input_current(&self) -> i16 {
        self.input_current
    }
    pub fn get_lower_threshold_voltage(&self) -> u16 {
        self.lower_threshold_voltage
    }
}

impl From<&CANFrame> for X208 {
    fn from(frame: &CANFrame) -> Self {
        let data = data_sanity(&frame, 0x208, 8);
        X208 {
            discharge_current: data[0] as i16 - 255,
            input_voltage: u16::from_le_bytes(data[1..=2].try_into().unwrap()),
            input_current: data[3] as i16 - 255,
            lower_threshold_voltage: u16::from_le_bytes(data[6..=7].try_into().unwrap()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct X209 {
    /// Charge/dis charge sequence control number
    sequence: u8,
    /// Remaining discharging time
    pub remaing_discharge_time: u16,
}

impl X209 {
    pub fn to_can(&self) -> CANFrame {
        let mut data = [0u8; 8];

        data[0] = self.sequence;
        [data[1], data[2]] = self.remaing_discharge_time.to_le_bytes();
        CANFrame::new(0x209, &data, false, false).unwrap()
    }
    pub fn new(sequence: u8, remaing_discharge_time: u16) -> Self {
        Self {
            sequence,
            remaing_discharge_time,
        }
    }
}

impl From<&CANFrame> for X209 {
    fn from(frame: &CANFrame) -> Self {
        let data = data_sanity(&frame, 0x209, 8);
        X209 {
            sequence: data[0],
            remaing_discharge_time: u16::from_le_bytes(data[1..=2].try_into().unwrap()),
        }
    }
}

#[inline]
fn get_bit(byte: u8, position: u8) -> bool {
    (byte & (1 << position)) != 0
}

#[inline]
fn data_sanity(frame: &CANFrame, id: u32, dlc: usize) -> &[u8] {
    assert!(
        frame.id() == id,
        "CANFrame decoder error: Incorrect ID can frame"
    );
    assert!(
        frame.data().len() == dlc,
        "CANFrame decoder error: DLC for can frame is not 8"
    );
    &frame.data()
}
#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn x109_test() {
        let frame = CANFrame::new(
            0x109,
            [0x02, 0x00, 0x00, 0x00, 0x01, 0x20, 0x00, 0x00].as_slice(),
            false,
            false,
        )
        .unwrap();
        let x109: X109 = X109::from(&frame);
        assert!(!x109.status.status_vehicle_connector_lock);
        assert!(x109.status.status_charger_stop_control);

        let frame = CANFrame::new(
            0x109,
            [0x02, 0x80, 0x01, 0x00, 0x01, 0x24, 0x00, 0x00].as_slice(),
            false,
            false,
        )
        .unwrap();
        let x109: X109 = X109::from(&frame);
        assert!(x109.status.status_charger_stop_control);
        let frame = CANFrame::new(
            0x109,
            [0x02, 0x80, 0x01, 0x00, 0x01, 0x05, 0x00, 0x00].as_slice(),
            false,
            false,
        )
        .unwrap();
        let x109: X109 = X109::from(&frame);
        assert!(!x109.status.status_charger_stop_control);
        assert!(x109.status.status_station);
    }
    #[test]
    fn x102_test() {
        let frame = CANFrame::new(
            0x102,
            [0x02, 0x9A, 0x01, 0x00, 0x00, 0xC9, 0x56, 0x00].as_slice(),
            false,
            false,
        )
        .unwrap();
        let x102: X102 = X102::from(&frame);
        assert!(x102.status.status_vehicle);
        assert!(!x102.can_charge());
        let frame = CANFrame::new(
            0x102,
            [0x02, 0x9A, 0x01, 0x00, 0x00, 0xC1, 0x56, 0x00].as_slice(),
            false,
            false,
        )
        .unwrap();
        let x102 = X102::from(&frame);
        assert!(!x102.status.status_vehicle); // start matching voltage
        assert!(x102.can_charge());
    }
    /*
            [0x02, 0x9A, 0x01, 0x00, 0x00, 0xC9, 0x56, 0x00]    <x102>
        100ms
        ControlProtocolNumberEV: 2-
        TargetBatteryVoltage: 410V
        ChargingCurrentRequest: 0A
        FaultBatteryVoltageDeviation: Normal
        FaultHighBatteryTemperature: Normal
        FaultBatteryCurrentDeviation: Normal
        FaultBatteryUndervoltage: Normal
        FaultBatteryOvervoltage: Normal
        StatusNormalStopRequest: No request
        StatusVehicle: EV contactor open or welding detection finished
        StatusChargingSystem: Normal
        StatusVehicleShifterPosition: Parked
        StatusVehicleCharging: Enabled
        ChargingRate: 86%
        Charging_close_unknown1: Enabled
        Charging_close_unknown2: Enabled

        0x02, 0x9A, 0x01, 0x00, 0x00, 0xC1, 0x56, 0x00    <x102>
    100ms
    ControlProtocolNumberEV: 2-
    TargetBatteryVoltage: 410V
    ChargingCurrentRequest: 0A
    FaultBatteryVoltageDeviation: Normal
    FaultHighBatteryTemperature: Normal
    FaultBatteryCurrentDeviation: Normal
    FaultBatteryUndervoltage: Normal
    FaultBatteryOvervoltage: Normal
    StatusNormalStopRequest: No request
    StatusVehicle: EV contactor closed or during welding detection
    StatusChargingSystem: Normal
    StatusVehicleShifterPosition: Parked
    StatusVehicleCharging: Enabled
    ChargingRate: 86%
    Charging_close_unknown1: Enabled
    Charging_close_unknown2: Enabled

         */
}
