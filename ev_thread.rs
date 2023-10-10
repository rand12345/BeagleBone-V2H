use crate::chademo::state::STATE;
use crate::chademo::{can::*, state::Chademo, state::ChargerState};
use crate::data_io::mqtt::CHADEMO_DATA;
use crate::data_io::panel::LedCommand;
use crate::error::IndraError;
use crate::meter::METER;
// use crate::pre_charger::pre_thread::pre_thread;
use crate::pre_charger::{PreCommand, PREDATA};
use crate::statics::*;
use crate::{log_error, MAX_AMPS, MAX_SOC, METER_BIAS, MIN_SOC};
use futures_util::StreamExt;
use log::warn;
use std::ops::ControlFlow;
use std::time::Duration;
use tokio::time::sleep;
use tokio::time::{timeout, Instant};

#[allow(unused_assignments)]
pub async fn ev100ms(
    chademo_tx: ChademoTx,
    pre_tx: PreTx,
    led_tx: LedTx,
) -> Result<(), IndraError> {
    use ChargerState::*;

    log::info!("Starting EV thread");
    let mut can = tokio_socketcan::CANSocket::open(&"can1").map_err(|_| IndraError::Error)?;
    let c_state = STATE.clone();
    let operational_mode = OPERATIONAL_MODE.clone();
    let mut chademo = Chademo::default();

    let mut x102 = X102::default();
    let x108 = X108::new();
    let mut x109 = X109::new(2);

    let mut feedback = 0.01; // Initial grid energy feedback value
    let t100ms = Duration::from_millis(100);

    let mut setpoint_amps_old = 0f32;
    let mut setpoint_voltage_old = 0f32;
    let predata = PREDATA.clone();
    let mut ev_connect_timeout: Option<Instant> = None;
    let mut ev_can_timeout: Option<Instant> = None;

    loop {
        // Get current state from main state loop
        let state = { c_state.lock().await.0 };

        match state {
            Idle => {
                let s = if !operational_mode.lock().await.is_idle() {
                    if ev_connect_timeout.is_none() {
                        ev_connect_timeout = Some(Instant::now())
                    }
                    Stage1
                } else {
                    Idle
                };
                log_error!("", chademo_tx.send(s).await);
                sleep(t100ms).await;
                continue;
            }
            GotoIdle => {
                // shutdown chademo, pre and monitor
                let predata = *predata.lock().await;
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
                    log_error!("", chademo_tx.send(Idle).await);
                    x109.plug_lock(false);
                }
                send_can_data(&can, &x108, &x109, true).await;
                log_error!("", chademo_tx.send(GotoIdle).await);
                sleep(t100ms).await;
                continue;
            }
            Exiting => {
                warn!("Exiting called");
                x109.charge_stop();
                send_can_data(&can, &x108, &x109, false).await;
                sleep(t100ms).await;
                println!("{:#?}", x102);
                println!("{:#?}", x109);
                let _ = pre_tx.send(PreCommand::Disable).await;
                let _ = pre_tx.send(PreCommand::DcAmpsSetpoint(0.0)).await;
                let _ = pre_tx.send(PreCommand::DcVoltsSetpoint(0.0)).await;
                send_can_data(&can, &x108, &x109, false).await;
                sleep(t100ms).await;
                send_can_data(&can, &x108, &x109, false).await;
                sleep(t100ms).await;
                log_error!("Final exiting", chademo_tx.send(state).await);
                continue;
            }
            Panic => todo!(),
            _ => (),
        };

        // ***************** Can RX with timeout
        if let ControlFlow::Break(_) = read_can_frames(
            t100ms,
            &mut can,
            &mut ev_can_timeout,
            &mut x102,
            &mut chademo,
            state,
        )
        .await
        {
            continue;
        }

        // ***************** Proceed with can TX after RX finished
        log::info!("State: {:?}", state);

        update_chademo_data(&mut chademo, x109, state);

        // update LEDs
        update_panel_leds(&led_tx, &chademo, &state).await;

        dbg!(chademo.status_vehicle_contactors());
        dbg!(chademo.fault());
        dbg!(chademo.can_charge());
        dbg!(chademo.soc());

        if let ControlFlow::Break(_) = error_guards(
            x102,
            &chademo_tx,
            state,
            &mut ev_connect_timeout,
            &mut ev_can_timeout,
            &operational_mode,
        )
        .await
        {
            continue;
        }
        match state {
            Stage1 => {
                let pre = predata.lock().await;
                if matches!(pre.get_state(), crate::pre_charger::PreState::Online) {
                    if !pre.enabled() {
                        log::warn!("{state:?} Pre not enabled");
                        log_error!(
                            "Re-enable Charger",
                            pre_tx.send(PreCommand::Enable).await //
                        );
                    } else {
                        x109.precharge();
                        send_can_data(&can, &x108, &x109, false).await;
                    }
                };

                log_error!("", chademo_tx.send(Stage1).await);

                continue;
            }
            Stage2 => {
                // K line low
                // send charger parameters and move to stage 3
                let pre = PREDATA.lock().await;

                x109.output_voltage = pre.get_dc_output_volts();
                x109.output_current = 0.0;
                chademo.soc_to_voltage();

                send_profile_to_pre(
                    &mut setpoint_amps_old,
                    &x109.output_current,
                    &pre_tx,
                    &mut setpoint_voltage_old,
                    chademo,
                )
                .await;
                send_can_data(&can, &x108, &x109, false).await;
                log_error!("", chademo_tx.send(Stage3).await);
                continue;
            }
            _ => {
                let predata = *predata.lock().await;
                x109.output_voltage = predata.get_dc_output_volts();
                x109.output_current = predata.get_dc_output_amps();
            }
        }
        // ***************** Awaiting charging enable status change from vehicle
        match state {
            Stage3 | Stage4 => {
                send_can_data(&can, &x108, &x109, false).await;
                if matches!(state, Stage3) {
                    // log::warn!("{state:?} Pre not enabled");
                    log_error!(
                        "Enable Charger",
                        pre_tx.send(PreCommand::Enable).await //
                    );
                    log_error!("", chademo_tx.send(Stage3).await);
                } else {
                    // Stage4
                    if chademo.can_charge() {
                        log_error!("", chademo_tx.send(Stage5).await);
                    } else {
                        log_error!("", chademo_tx.send(Stage4).await);
                    }
                };
            }
            // ***************** Contactors closing/closed charging states
            _ => {
                if !chademo.can_charge() {
                    log_error!("", chademo_tx.send(GotoIdle).await); // TEMP!
                    send_can_data(&can, &x108, &x109, true).await;
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
                    if chademo.soc() >= MAX_SOC {
                        *operational_mode.lock().await = crate::global_state::OperationMode::Idle;
                    };
                    chademo.track_ev_amps()
                };

                send_profile_to_pre(
                    &mut setpoint_amps_old,
                    &setpoint_amps,
                    &pre_tx,
                    &mut setpoint_voltage_old,
                    chademo,
                )
                .await;
                ev_connect_timeout = Some(Instant::now());
                send_can_data(&can, &x108, &x109, true).await;
                log_error!("", chademo_tx.send(state).await);
            }
        };
    }
}

#[inline]
async fn error_guards(
    x102: X102,
    chademo_tx: &tokio::sync::mpsc::Sender<ChargerState>,
    state: ChargerState,
    ev_connect_timeout: &mut Option<Instant>,
    ev_can_timeout: &mut Option<Instant>,
    operational_mode: &std::sync::Arc<tokio::sync::Mutex<crate::global_state::OperationMode>>,
) -> ControlFlow<()> {
    use crate::ChargerState::*;
    if x102.fault() {
        // bail
        log_error!("x102.fault bail", chademo_tx.send(Exiting).await);
        return ControlFlow::Break(());
    }
    if state > GotoIdle && ev_connect_timeout.is_some() {
        if ev_connect_timeout.unwrap().elapsed().as_secs() > 15 {
            *ev_connect_timeout = None;
            log::error!("EV connect timeout (15s)");
            log_error!("", chademo_tx.send(GotoIdle).await);
            return ControlFlow::Break(());
        };
    };
    if state >= Stage1 && ev_can_timeout.is_some() {
        if ev_can_timeout.unwrap().elapsed().as_secs() > 1 {
            *ev_can_timeout = None;
            log::error!("EV ev can timeout (1s)");
            log_error!("", chademo_tx.send(GotoIdle).await);
            return ControlFlow::Break(());
        };
    };

    if operational_mode.lock().await.is_idle() {
        log::warn!("Operational mode = idle -> call GotoIdle");
        log_error!("", chademo_tx.send(GotoIdle).await);
        return ControlFlow::Break(());
    }
    ControlFlow::Continue(())
}

#[inline]
fn update_chademo_data(chademo: &mut Chademo, x109: X109, state: ChargerState) {
    chademo.set_volts(x109.output_voltage);
    chademo.set_amps(x109.output_current);
    chademo.set_state(state);

    if let Ok(mut data) = CHADEMO_DATA.try_lock() {
        data.from_chademo(*chademo);
    };
}

#[inline]
async fn read_can_frames(
    t100ms: Duration,
    can: &mut tokio_socketcan::CANSocket,
    ev_can_timeout: &mut Option<Instant>,
    x102: &mut X102,
    chademo: &mut Chademo,
    state: ChargerState,
) -> ControlFlow<()> {
    use crate::ChargerState::Stage1;
    if let Ok(Some(Ok(frame))) = timeout(t100ms, can.next()).await {
        *ev_can_timeout = Some(Instant::now());
        if let ControlFlow::Break(_) = rx_can(frame, x102, chademo) {
            return ControlFlow::Break(());
        }
    } else {
        if state > Stage1 {
            log::warn!("Can 100ms timeout! {state:?}")
        }
    };
    ControlFlow::Continue(())
}

#[inline]
async fn update_panel_leds(led_tx: &LedTx, chademo: &Chademo, state: &ChargerState) {
    // use crate::eventbus::Event::LedCommand;
    let (soc, amps, neg) = if matches!(state, ChargerState::Idle) {
        (0, 0.0, false) // Remove status bars from led panel
    } else {
        (
            chademo.soc(),
            chademo.amps(),
            chademo.amps().is_sign_negative(),
        )
    };
    log_error!("", led_tx.send(LedCommand::SocBar(soc)).await);

    // convert from float amps to percentage vs max amps const
    let mut amps: u32 = (amps.abs() as u8).min(MAX_AMPS) as u32 * 100;
    if amps != 0 {
        amps /= MAX_AMPS as u32;
    };
    log_error!(
        "",
        led_tx.send(LedCommand::EnergyBar(amps as u8, neg)).await
    );
}

#[inline]
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

#[inline]
async fn send_profile_to_pre(
    setpoint_amps_old: &mut f32,
    setpoint_amps: &f32,
    pre_tx: &PreTx,
    setpoint_voltage_old: &mut f32,
    chademo: Chademo,
) {
    // reduce Pre commands through dedup
    if *setpoint_amps_old != *setpoint_amps {
        *setpoint_amps_old = *setpoint_amps;
        log_error!(
            format!("DcAmpsSetpoint send {}A", setpoint_amps),
            pre_tx
                .send(PreCommand::DcAmpsSetpoint(*setpoint_amps))
                .await // DcAmpsSetpoint!!!! x102.
        );
    }
    if *setpoint_voltage_old != chademo.target_voltage() {
        *setpoint_voltage_old = chademo.target_voltage();
        log_error!(
            format!("DcVoltsSetpoint send {}", chademo.target_voltage()),
            pre_tx
                .send(PreCommand::DcVoltsSetpoint(chademo.target_voltage()))
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
