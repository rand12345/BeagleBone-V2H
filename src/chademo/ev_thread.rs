use crate::chademo::state::{OPERATIONAL_MODE, STATE};
use crate::chademo::{can::*, state::Chademo, state::ChargerState};
use crate::data_io::mqtt::CHADEMO_DATA;
use crate::data_io::panel::{ButtonTriggered, Led};
use crate::error::IndraError;
use crate::meter::METER;
use crate::pre_charger::pre_commands::PreCmd;
use crate::pre_thread::PREDATA;
use crate::{log_error, MAX_AMPS, MAX_SOC, METER_BIAS, MIN_SOC};
use futures_util::StreamExt;
use log::warn;
use std::ops::ControlFlow;
use std::time::Duration;
use tokio::time::{timeout, Instant};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    time::sleep,
};

pub async fn ev100ms(
    send_cmd: Sender<PreCmd>,
    send_state: Sender<ChargerState>,
    mut but_receiver: Receiver<ButtonTriggered>,

    led_sender: Sender<Led>,
) -> Result<(), IndraError> {
    use ChargerState::*;
    use PreCmd::*;

    log::info!("Starting EV thread");
    let mut can = tokio_socketcan::CANSocket::open(&"can1").map_err(|_| IndraError::Error)?;
    let c_state = STATE.clone();
    let mut chademo = Chademo::default();

    let mut x102 = X102::default();
    let x108 = X108::new();
    let mut x109 = X109::new(2);

    let mut feedback = 0.01; // Initial grid energy feedback value
    let t100ms = Duration::from_millis(100);

    let mut setpoint_amps_old = 0f32;
    let mut setpoint_voltage_old = 0f32;

    let mut ev_connect_timeout: Option<Instant> = None;
    loop {
        // Get current state from main state loop
        let state = { c_state.lock().await.0 };

        match but_receiver.try_recv() {
            Ok(ButtonTriggered::Boost) => {
                let mut opm = OPERATIONAL_MODE.lock().await;
                opm.boost();
                let _ = send_state.send(state).await;
            }
            Ok(ButtonTriggered::OnOff) => {
                let mut opm = OPERATIONAL_MODE.lock().await;
                opm.onoff();
                let _ = send_state.send(state).await;
            }
            Err(_) => (),
        };

        match state {
            Idle => {
                sleep(t100ms).await;
                if !OPERATIONAL_MODE.lock().await.is_idle() {
                    // Move on to active modes - V2h or Charge
                    let _ = send_state.send(Stage1).await;
                }
                continue;
            }
            Exiting => {
                warn!("Exiting called");
                x109.charge_stop();
                send_can_data(&can, &x108, &x109, false).await;
                sleep(t100ms).await;
                println!("{:#?}", x102);
                println!("{:#?}", x109);
                let _ = send_cmd.send(Disable).await;
                let _ = send_cmd.send(DcAmpsSetpoint(0.0)).await;
                let _ = send_cmd.send(DcVoltsSetpoint(0.0)).await;
                send_can_data(&can, &x108, &x109, false).await;
                sleep(t100ms).await;
                send_can_data(&can, &x108, &x109, false).await;
                sleep(t100ms).await;
            }
            Panic => todo!(),
            _ => (),
        };

        // ***************** Can RX with timeout
        let frame = if let Ok(Some(Ok(frame))) = timeout(t100ms, can.next()).await {
            frame
        } else {
            // let _ = led_sender.send(Led::SocBar(0)).await;
            continue;
        };

        // ***************** Proceed with can TX after RX finished
        if let ControlFlow::Break(_) = rx_can(frame, &mut x102, &mut chademo) {
            continue;
        }

        chademo.set_volts(x109.output_voltage);
        chademo.set_amps(x109.output_current);
        chademo.set_state(state);
        if let Ok(mut data) = CHADEMO_DATA.try_lock() {
            data.from_chademo(chademo);
        };

        // update LEDs
        update_panel_leds(&led_sender, &chademo, &state).await;

        dbg!(chademo.status_vehicle_contactors());
        dbg!(chademo.fault());
        dbg!(chademo.can_charge());
        dbg!(chademo.soc());
        if x102.fault() {
            // bail
            let _ = send_state.send(ChargerState::Exiting).await;
            continue;
        }

        log::info!("State: {:?}", state);
        match state {
            GotoIdle => {
                // shutdown chademo, pre and monitor
                // send statuschargerstopcontrol
                let predata = *PREDATA.lock().await;
                if predata.get_dc_output_amps() > 0.0 {
                    warn!("Shutdown to idle 1");
                    x109.charge_halt();
                } else if x102.status_vehicle_charging && !x102.status_vehicle {
                    //             matches!(x102.status_vehicle, false); // EV contactors closed
                    // assert_eq!(x102.status_vehicle_charging, true); // Charge commanded
                    warn!("Shutdown to idle 2");
                    x109.charge_stop();
                } else if !x102.status_vehicle_charging && !x102.status_vehicle {
                    //             matches!(x102.status_vehicle, false); // EV contactors closed
                    // assert_eq!(x102.status_vehicle_charging, false); // No charge commanded
                    warn!("Shutdown to idle 3");
                } else if !x102.status_vehicle_charging && x102.status_vehicle {
                    warn!("Shutdown to idle 4 - unlocking plug");
                    let _ = send_state.send(Idle).await;
                    x109.plug_lock(false);
                }
                send_can_data(&can, &x108, &x109, true).await;
                continue;
            }
            Stage1 => {
                if !PREDATA.lock().await.enabled() {
                    sleep(t100ms).await;
                    log_error!(
                        "Enable Charger",
                        send_cmd.send(Enable).await //
                    );
                    warn!("Waiting for Pre charger enable signal");
                    continue;
                };
                x109.precharge();
                send_can_data(&can, &x108, &x109, false).await;
                let _ = send_state.send(ChargerState::Stage1).await;
                if ev_connect_timeout.is_none() {
                    ev_connect_timeout = Some(Instant::now())
                } else if ev_connect_timeout.unwrap().elapsed().as_secs() > 10 {
                    ev_connect_timeout = None;
                    log::error!("EV connect timeout");
                    let _ = send_state.send(ChargerState::Idle).await;
                }
                continue;
            }
            Stage2 => {
                // Kline low
                // send charger parameters and move to stage 3
                let pre = PREDATA.lock().await;

                x109.output_voltage = pre.get_dc_output_volts();
                x109.output_current = 0.0;
                chademo.soc_to_voltage();

                send_profile_to_pre(
                    &mut setpoint_amps_old,
                    &x109.output_current,
                    &send_cmd,
                    &mut setpoint_voltage_old,
                    chademo,
                )
                .await;
                send_can_data(&can, &x108, &x109, false).await;
                let _ = send_state.send(ChargerState::Stage3).await;
                continue;
            }
            _ => {
                let predata = *crate::pre_thread::PREDATA.clone().lock().await;
                x109.output_voltage = predata.get_dc_output_volts();
                x109.output_current = predata.get_dc_output_amps();
            }
        }
        // ***************** Awaiting charging enable status change from vehicle
        match state {
            Stage3 | Stage4 => {
                send_can_data(&can, &x108, &x109, false).await;

                if matches!(state, Stage3) {
                    let _ = send_state.send(ChargerState::Stage3).await;
                } else {
                    if chademo.can_charge() {
                        let _ = send_state.send(ChargerState::Stage5).await;
                    } else {
                        let _ = send_state.send(ChargerState::Stage4).await;
                    }
                }
            }
            // ***************** Contactors closing/closed charging states
            _ => {
                if !chademo.can_charge() {
                    let _ = send_state.send(ChargerState::Idle).await; // TEMP!
                    log::error!("Stage demotion: x102 can_charge == false ");
                    continue;
                }

                x109.charge_start();

                let setpoint_amps = if matches!(state, Stage5) {
                    // Stage5
                    chademo.soc_to_voltage();
                    if chademo.requested_amps() == 0.0 {
                        1.0
                    } else {
                        chademo.requested_amps()
                    }
                } else if matches!(state, Stage7) {
                    // Stage 7
                    let meter = { *METER.lock().await } + METER_BIAS;
                    v2h_throttle(&mut feedback, meter, setpoint_amps_old, x109, chademo)
                } else {
                    // Stage 6
                    chademo.track_ev_amps()
                };

                send_profile_to_pre(
                    &mut setpoint_amps_old,
                    &setpoint_amps,
                    &send_cmd,
                    &mut setpoint_voltage_old,
                    chademo,
                )
                .await;

                send_can_data(&can, &x108, &x109, true).await;
                let _ = send_state.send(state).await;
            }
        };
    }
}

async fn update_panel_leds(led_sender: &Sender<Led>, chademo: &Chademo, state: &ChargerState) {
    let (soc, amps, neg) = if matches!(state, ChargerState::Idle) {
        (0, 0.0, false) // Remove status bars from led panel
    } else {
        (
            chademo.soc(),
            chademo.amps(),
            chademo.amps().is_sign_negative(),
        )
    };
    let _ = led_sender.send(Led::SocBar(soc)).await;

    // convert from float amps to percentage vs max amps const
    let mut amps: u32 = (amps.abs() as u8).min(MAX_AMPS) as u32 * 100;
    if amps != 0 {
        amps /= MAX_AMPS as u32;
    };
    let _ = led_sender.send(Led::EnergyBar(amps as u8, neg)).await;
}

fn v2h_throttle(
    feedback: &mut f32,
    meter: f32,
    setpoint_amps_old: f32,
    x109: X109,
    chademo: Chademo,
) -> f32 {
    if *feedback != meter && meter.is_normal() {
        *feedback = meter;

        let setpoint_amps = setpoint_amps_old - (meter / x109.output_voltage) * 0.45;

        let setpoint_amps = setpoint_amps
            .min(MAX_AMPS as f32)
            .max(-1.0 * MAX_AMPS as f32);

        // Check against SoC limits and EV throttling
        if MIN_SOC >= chademo.soc() && setpoint_amps.is_sign_negative() {
            warn!("SoC: {} too low, discharge disabled", chademo.soc());
            0.0
        } else if MAX_SOC <= chademo.soc() && setpoint_amps.is_sign_positive() {
            warn!("SoC: {} too high, charge disabled", chademo.soc());
            0.0
        // Restrict charging
        } else if setpoint_amps.is_sign_positive() && setpoint_amps > chademo.requested_amps() {
            warn!(
                "Charge taper: {setpoint_amps}A too high, charge restricted to {}A",
                chademo.requested_amps()
            );
            chademo.requested_amps()
        } else {
            setpoint_amps
        }
    } else {
        setpoint_amps_old
    }
}

async fn send_profile_to_pre(
    setpoint_amps_old: &mut f32,
    setpoint_amps: &f32,
    send_cmd: &Sender<PreCmd>,
    setpoint_voltage_old: &mut f32,
    chademo: Chademo,
) {
    // reduce Pre commands
    if *setpoint_amps_old != *setpoint_amps {
        *setpoint_amps_old = *setpoint_amps;
        log_error!(
            format!("DcAmpsSetpoint send {}A", setpoint_amps),
            send_cmd.send(PreCmd::DcAmpsSetpoint(*setpoint_amps)).await // DcAmpsSetpoint!!!! x102.
        );
    }
    if *setpoint_voltage_old != chademo.target_voltage() {
        *setpoint_voltage_old = chademo.target_voltage();
        log_error!(
            format!("DcVoltsSetpoint send {}", chademo.target_voltage()),
            send_cmd
                .send(PreCmd::DcVoltsSetpoint(chademo.target_voltage()))
                .await
        );
    }
}
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test1() {
        let x102: X102 = X102::from([0x2, 0x9A, 0x01, 0x00, 0x0, 0xC8, 0x56, 0x00].as_slice());
        //         02 9A 01 00 00 C8 56 00    <x102>
        // 100ms
        // ControlProtocolNumberEV: 2-
        // TargetBatteryVoltage: 410V
        // ChargingCurrentRequest: 0A
        // FaultBatteryVoltageDeviation: Normal
        // FaultHighBatteryTemperature: Normal
        // FaultBatteryCurrentDeviation: Normal
        // FaultBatteryUndervoltage: Normal
        // FaultBatteryOvervoltage: Normal
        // StatusNormalStopRequest: No request
        // StatusVehicle: EV contactor open or welding detection finished
        // StatusChargingSystem: Normal
        // StatusVehicleShifterPosition: Parked
        // StatusVehicleCharging: Disabled
        // ChargingRate: 86%
        assert_eq!(x102.control_protocol_number_ev, 2);
        assert_eq!(x102.target_battery_voltage, 410.0);
        assert_eq!(x102.charging_current_request, 0);
        assert_eq!(x102.fault(), false);
        assert_eq!(x102.status_vehicle, true); // EV contactors open
        assert_eq!(x102.status_vehicle_charging, false); // No commanded charge
    }

    #[test]
    fn test2() {
        let x102: X102 = X102::from([0x2, 0x9A, 0x01, 0x00, 0x0, 0xC0, 0x56, 0x00].as_slice());

        //         02 9A 01 00 00 C0 56 00    <x102>
        // 100ms
        // ControlProtocolNumberEV: 2-
        // TargetBatteryVoltage: 410V
        // ChargingCurrentRequest: 0A
        // FaultBatteryVoltageDeviation: Normal
        // FaultHighBatteryTemperature: Normal
        // FaultBatteryCurrentDeviation: Normal
        // FaultBatteryUndervoltage: Normal
        // FaultBatteryOvervoltage: Normal
        // StatusNormalStopRequest: No request
        // StatusVehicle: EV contactor closed or during welding detection
        // StatusChargingSystem: Normal
        // StatusVehicleShifterPosition: Parked
        // StatusVehicleCharging: Disabled
        // ChargingRate: 86%
        // Charging_close_unknown1: Enabled
        // Charging_close_unknown2: Enabled

        assert_eq!(x102.control_protocol_number_ev, 2);
        assert_eq!(x102.target_battery_voltage, 410.0);
        assert_eq!(x102.charging_current_request, 0);
        assert_eq!(x102.fault(), false);
        assert_eq!(x102.status_vehicle, false); // EV contactors closed
        assert_eq!(x102.status_vehicle_charging, false); // No commanded charge
    }

    #[test]
    fn test3() {
        let x102: X102 = X102::from([0x2, 0x9A, 0x01, 0x00, 0x0, 0xC1, 0x56, 0x00].as_slice());

        //  02 9A 01 0E 00 C1 56 00    <x102>
        // 100ms
        // ControlProtocolNumberEV: 2-
        // TargetBatteryVoltage: 410V
        // ChargingCurrentRequest: 14A
        // FaultBatteryVoltageDeviation: Normal
        // FaultHighBatteryTemperature: Normal
        // FaultBatteryCurrentDeviation: Normal
        // FaultBatteryUndervoltage: Normal
        // FaultBatteryOvervoltage: Normal
        // StatusNormalStopRequest: No request
        // StatusVehicle: EV contactor closed or during welding detection
        // StatusChargingSystem: Normal
        // StatusVehicleShifterPosition: Parked
        // StatusVehicleCharging: Enabled
        // ChargingRate: 86%
        // Charging_close_unknown1: Enabled
        // Charging_close_unknown2: Enabled

        assert_eq!(x102.control_protocol_number_ev, 2);
        assert_eq!(x102.target_battery_voltage, 410.0);
        assert_eq!(x102.charging_current_request, 0);
        assert_eq!(x102.fault(), false);
        assert_eq!(x102.status_vehicle, false); // EV contactors closed
        assert_eq!(x102.status_vehicle_charging, true); // Charge commanded
    }
}
