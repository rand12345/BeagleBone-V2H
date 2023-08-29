use crate::{
    chademo::{
        can::*,
        state::{Chademo, *}, //ChargerState
    },
    data_io::{mqtt::CHADEMO_DATA, panel::LedCommand},
    error::IndraError,
    global_state::{ChargeParameters, OperationMode},
    log_error,
    meter::METER,
    pre_charger::{
        pre_thread::{self},
        PreCharger, PreCommand, PREDATA,
    },
    statics::{self, *},
    MAX_AMPS, MAX_SOC, METER_BIAS, MIN_SOC,
};
use chademo_v2::chademo::{X109Status, X108};
use log::warn;
use std::{sync::Arc, time::Duration};
use sysfs_gpio::Pin;
use tokio::time::Instant;
use tokio::{sync::Mutex, time::sleep};
use tokio_socketcan::{CANFrame, CANSocket};

pub async fn ev100ms(led_tx: LedTx, mode_rx: ChademoRx) -> Result<(), IndraError> {
    log::info!("Starting EV thread");
    let mut can = tokio_socketcan::CANSocket::open(&"can1").map_err(|_| IndraError::Error)?;
    // let operational_mode = OPERATIONAL_MODE.clone();
    let mut chademo = Chademo::new();
    let t100ms = Duration::from_millis(100);
    let predata = PREDATA.clone();
    let (pre_tx, pre_rx) = statics::pre_channel();
    let pre_rx = mutex(pre_rx);
    let mode_rx = mutex(mode_rx);
    update_panel_leds(&led_tx, &chademo).await;

    loop {
        if let Some(state) = mode_rx.clone().lock().await.recv().await {
            chademo.set_state(state);
            if !(state.is_v2h() || state.is_charge()) {
                continue;
            }
        }
        // let mut ev_connect_timeout: Option<Instant> = Some(Instant::now());

        //
        // Spawn a watchdog timer which changed OP to Idle after timeout
        //
        log::info!("{:?} active", chademo.state());
        chademo.pins().pre_ac.set_value(1).unwrap();
        if let Err(e) = init_pre(&pre_rx, &predata, t100ms, &pre_tx).await {
            log::error!("Pre init failed - should be catastropic and hang {e:?}")
        };
        chademo.x109.status = X109Status::from(0x20);
        assert!(!chademo.x109.status.status_vehicle_connector_lock);
        assert!(!chademo.x109.status.status_station);
        log::info!("Raise D1");
        log_error!("Setting D1 high", chademo.pins().d1.set_value(1));
        update_chademo_mutex(&chademo).await;

        chademo.plug_lock(true);
        chademo.x109.status = X109Status::from(0x24);
        assert!(chademo.x109.status.status_vehicle_connector_lock);
        log::info!("Check can frames & Wait for K line");
        if let Err(e) = k_line(&mut can, &mut chademo, &pre_tx).await {
            log::error!("K line init failed - should be catastropic and hang {e:?}")
        };

        // chademo.precharge();
        log::info!("insulation tests skipped !!!");
        chademo.pins().d2.set_value(1).unwrap();

        update_chademo_mutex(&chademo).await;
        log::info!("when voltage match - raise D2");
        if let Err(e) = precharge(&mut can, &mut chademo, &pre_tx, &predata).await {
            log::error!("precharge & contactor init failed - should be catastropic and hang {e:?}")
        }
        chademo.charge_start();
        chademo.x109.status = X109Status::from(0x05);
        assert!(chademo.x109.status.status_vehicle_connector_lock);
        assert!(chademo.x109.status.status_station);
        update_panel_leds(&led_tx, &chademo).await;
        update_chademo_mutex(&chademo).await;

        log::info!("            Entering charge loop!");
        let exit_reason =
            match charge_mode(&mut chademo, &mut can, &pre_tx, &led_tx, mode_rx.clone()).await {
                Ok(reason) => reason,
                Err(e) => {
                    log::error!("Bailed out of main charge {e:?}");
                    OperationMode::Idle
                }
            };

        // end charge ========================================================

        update_chademo_mutex(&chademo).await;
        chademo.x109.status = X109Status::from(0x24);
        chademo.charging_stop_control_release();
        let _ = pre_tx.send(PreCommand::DcAmpsSetpoint(0.0)).await;
        let mut exit = false;
        loop {
            recv_send(&mut can, &mut chademo, false).await?;
            let predata = predata.lock().await;
            chademo.x109.output_voltage = predata.get_dc_output_volts();
            chademo.update_amps(predata.get_dc_output_amps() as i16);

            if matches!(chademo.pins().k.get_value(), Ok(1)) {
                continue;
            };
            chademo.status_station_enabled(false);

            if !chademo.status_vehicle_ok()
                && predata.volts_equal()
                && predata.get_dc_output_volts() != 0.0
            {
                log::info!("Contactors opening");
                if chademo.pins().c1.set_value(0).is_ok() {
                    print!("\x07");
                    if chademo.pins().c2.set_value(0).is_ok() {
                        print!("\x07");
                        warn!("                                       !!!!CONTACTORS OPEN!!!!");

                        chademo.charge_stop();
                        exit = true;
                    }
                }
            } else {
                log::warn!("Conditions not yet met for contactor open");
                log::warn!("Perform welding detection etc etc");
                continue;
            }

            chademo.x109.status = X109Status::from(0x20);
            if exit {
                break;
            }
        }
        let _ = pre_tx.send(PreCommand::Shutdown).await;
        log_error!("Exit charge: d2", chademo.pins().d2.set_value(0));
        log_error!("Exit charge: d1", chademo.pins().d1.set_value(0));
        sleep(t100ms * 2).await;
        log_error!("Exit charge: Pre AC", chademo.pins().pre_ac.set_value(0));
        log_error!(
            "Exit charge: pluglock",
            chademo.pins().pluglock.set_value(0)
        );
        log::warn!("Charge/discharge mode ended");
        update_chademo_mutex(&chademo).await;

        if matches!(exit_reason, crate::global_state::OperationMode::Quit) {
            return Ok(());
        }
        //loops back to idle
    }
}

async fn charge_mode(
    chademo: &mut Chademo,
    can: &mut CANSocket,
    pre_tx: &tokio::sync::mpsc::Sender<PreCommand>,
    led_tx: &tokio::sync::mpsc::Sender<LedCommand>,
    mode_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<OperationMode>>>,
) -> Result<OperationMode, IndraError> {
    let mut mode_rx = mode_rx.lock().await;
    let mut last_soc = *chademo.soc();
    let mut last_volts = 0.0;
    let mut last_amps = 0.0;
    let mut feedback = 0.01;

    let exit_reason = loop {
        recv_send(can, chademo, false).await?;
        use crate::global_state::OperationMode::*;
        if !chademo.status_vehicle_charging() {
            log::warn!("EV stopped charge");
            break Idle;
        }
        {
            // listen for incomming mode changes
            if let Ok(op) = mode_rx.try_recv() {
                let op = match (chademo.state(), op) {
                    (V2h, V2h) => Idle,
                    (V2h, Charge(p)) => Charge(p),
                    (_, Idle) => Idle,
                    (Charge(_), V2h) => V2h,
                    (_, Quit) => Quit,
                    _ => *chademo.state(),
                };
                update_panel_leds(&led_tx, &chademo).await;
                chademo.set_state(op)
            }
            update_panel_leds(&led_tx, &chademo).await
        }

        let op = chademo.state();

        async fn handle_charge_mode(cp: &ChargeParameters, chademo: &Chademo) -> f32 {
            let mut amps = (cp.get_amps() as f32).min(chademo.requested_amps());
            if let Some(soc_limit) = cp.get_soc_limit() {
                if &soc_limit <= chademo.soc() {
                    amps = 0.0
                }
            }

            amps
        }

        let charging_current_request = match *op {
            V2h => amps_meter_profiler(&mut feedback, &last_amps, &*chademo).await,

            // This needs feature adding for eco mode ("v2h" charge only)
            Charge(c) => match c.get_eco() {
                false => handle_charge_mode(&c, &chademo).await,
                true => amps_meter_profiler(&mut feedback, &last_amps, &*chademo)
                    .await
                    .clamp(0.0, MAX_AMPS as f32), //
            },
            Quit | Idle => break *op,
            _ => continue,
        };
        if &last_volts != chademo.target_voltage() {
            last_volts = *chademo.target_voltage();
            let _ = pre_tx.send(PreCommand::DcVoltsSetpoint(last_volts)).await;
        }

        if last_amps != charging_current_request as f32 {
            last_amps = charging_current_request as f32;
            let _ = pre_tx.send(PreCommand::DcAmpsSetpoint(last_amps)).await;
            update_chademo_mutex(&*chademo).await;
            update_panel_leds(&led_tx, &chademo).await
        }
        if &last_soc != chademo.soc() {
            last_soc = *chademo.soc();
            update_chademo_mutex(&*chademo).await;
            update_panel_leds(&led_tx, &chademo).await
        }
    };
    Ok(exit_reason)
}

async fn init_pre(
    pre_rx: &std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<PreCommand>>>,
    predata: &std::sync::Arc<tokio::sync::Mutex<crate::pre_charger::PreCharger>>,
    t100ms: Duration,
    pre_tx: &tokio::sync::mpsc::Sender<PreCommand>,
) -> Result<(), IndraError> {
    log::info!("Initalise PRE");
    tokio::spawn(pre_thread::init(pre_rx.clone()));
    loop {
        let pre = predata.lock().await;
        if pre.get_state().is_online() {
            let _ = pre_tx.send(PreCommand::DcAmpsSetpoint(1.0)).await;
            sleep(t100ms).await;
            let _ = pre_tx.send(PreCommand::DcVoltsSetpoint(370.0)).await;
            sleep(t100ms).await;
            break;
        };
        sleep(t100ms * 10).await;
        log::info!("wait for pre enabled - test pre command");
    }

    loop {
        let pre = predata.lock().await;
        if pre.get_dc_setpoint_volts() as u16 == 370 && pre.get_dc_setpoint_amps() as u16 == 1 {
            // if pre.volts_equal() {
            if pre.get_dc_output_volts() as u16 == pre.get_dc_setpoint_volts() as u16 {
                break Ok(());
            }
        }
        log::info!(
            "Waiting for Pre ({}V {}A) = Output ({}V {}A) (pre.status_ok() {})",
            pre.get_dc_setpoint_volts() as u16,
            pre.get_dc_setpoint_amps() as u16,
            pre.get_dc_output_volts(),
            1,
            pre.status_ok()
        );
        if !pre.status_ok() {
            log::warn!("{:x?}", pre.get_status())
        }
        sleep(t100ms * 1).await;
    }
}

async fn k_line(
    can: &mut CANSocket,
    chademo: &mut Chademo,
    pre_tx: &tokio::sync::mpsc::Sender<PreCommand>,
) -> Result<(), IndraError> {
    loop {
        recv_send(can, chademo, false).await?;
        if matches!(chademo.pins().k.get_value(), Ok(0)) {
            //&& chademo.status_vehicle_charging() {
            log::info!("K line ok");
            if chademo.x102_status().status_vehicle_charging {
                // await 102.5.0 high
                break Ok(());
            } else {
                log::warn!("await 102.5.0 high")
            }
        } else {
            log::warn!("await k high")
        }
    }
}

async fn precharge(
    can: &mut CANSocket,
    chademo: &mut Chademo,
    pre_tx: &tokio::sync::mpsc::Sender<PreCommand>,
    // mode_rx: &mut tokio::sync::mpsc::Receiver<OperationMode>,
    predata: &Arc<Mutex<PreCharger>>,
) -> Result<Option<OperationMode>, IndraError> {
    use OperationMode::*;
    let mut old_soc = chademo.x102.state_of_charge;
    loop {
        recv_send(can, chademo, false).await?;

        let predata = predata.lock().await;
        chademo.x109.output_voltage = predata.get_dc_output_volts();
        chademo.update_amps(predata.get_dc_output_amps() as i16);

        // if x102.5.3
        if chademo.status_vehicle_ok() {
            if (10..=100).contains(chademo.soc()) && chademo.soc() != &old_soc {
                old_soc = *chademo.soc();

                log_error!(
                    format!("SoC at {}", chademo.soc()),
                    pre_tx
                        .send(PreCommand::DcVoltsSetpoint(chademo.soc_to_voltage()))
                        .await
                );
                continue; // allow pre to change voltage before proceeding
            }
            if predata.volts_equal() {
                log::info!("Contactors closing");
                if chademo.pins().c1.set_value(1).is_ok() {
                    print!("\x07");
                    if chademo.pins().c2.set_value(1).is_ok() {
                        print!("\x07");
                        //109.5.5
                        chademo.charging_stop_control_set();
                        break Ok(None);
                    }
                }
            } else {
                log::warn!("Volts not equal")
            };
        } else {
            log::warn!("x102.5.3 high")
        }
    }
}

async fn amps_meter_profiler(
    feedback: &mut f32,
    setpoint_amps_old: &f32,
    chademo: &Chademo,
) -> f32 {
    let meter = *METER.lock().await + METER_BIAS;
    let soc = *chademo.soc(); // Change this
    if *feedback == meter && meter.is_normal() {
        *setpoint_amps_old
    } else {
        *feedback = meter;

        let setpoint_amps = setpoint_amps_old - (meter / chademo.x109.output_voltage) * 0.45;

        let setpoint_amps = setpoint_amps.clamp(-1.0 * MAX_AMPS as f32, MAX_AMPS as f32);

        // Check against SoC limits and EV throttling
        if MIN_SOC >= soc && setpoint_amps.is_sign_negative() {
            warn!("SoC: {} too low, discharge disabled", soc);
            0.0
        } else if MAX_SOC <= soc && setpoint_amps.is_sign_positive() {
            warn!("SoC: {} too high, charge disabled", soc);
            0.0
        // Restrict charging
        } else if setpoint_amps.is_sign_positive()
            && setpoint_amps > chademo.x102.charging_current_request as f32
        {
            warn!(
                "Charge taper: {setpoint_amps}A too high, charge restricted to {}A",
                chademo.x102.charging_current_request
            );
            chademo.x102.charging_current_request as f32
        } else {
            setpoint_amps
        }
    }
}

#[inline]
async fn update_chademo_mutex(chademo: &Chademo) {
    CHADEMO_DATA.clone().lock().await.from_chademo(*chademo);
}

#[inline]
async fn update_panel_leds(led_tx: &LedTx, chademo: &Chademo) {
    // use crate::eventbus::Event::LedCommand;

    log_error!(
        "Update logo state",
        led_tx.send(LedCommand::Logo(chademo.state().into())).await
    );
    let (soc, amps, neg) = if matches!(chademo.state(), &OperationMode::Idle) {
        (0, 0, false) // Remove status bars from led panel
    } else {
        (
            *chademo.soc(),
            *chademo.output_amps(),
            chademo.output_amps().is_negative(),
        )
    };
    log_error!("Update LED SoC", led_tx.send(LedCommand::SocBar(soc)).await);

    // convert from float amps to percentage vs max amps const
    let mut amps: u32 = (amps.abs() as u8).min(MAX_AMPS) as u32 * 100;
    if amps != 0 {
        amps /= MAX_AMPS as u32;
    };
    log_error!(
        "Update LED Energy",
        led_tx.send(LedCommand::EnergyBar(amps as u8, neg)).await
    );
}

#[cfg(test)]
mod test {
    use chademo_v2::chademo::{X102, X109};

    use super::*;

    #[test]
    fn test_x109() {
        // let mut chademo = Chademo::new();
        let mut x109 = X109::new(3, true);
        println!("{:02x}", Into::<u8>::into(x109.status));
        assert!(!x109.status.status_vehicle_connector_lock);
        assert!(!x109.status.status_station);
        x109.status = 0x24.into();
        println!("{:02x}", Into::<u8>::into(x109.status));
        assert!(x109.status.status_vehicle_connector_lock);
        x109.status = 0x05.into();
        println!("{:02x}", Into::<u8>::into(x109.status));
        assert!(x109.status.status_vehicle_connector_lock);
        assert!(x109.status.status_station);
    }

    #[test]
    fn test1() {
        let frame = CANFrame::new(
            0x102,
            [0x2, 0x9A, 0x01, 0x00, 0x0, 0xC8, 0x56, 0x00].as_slice(),
            false,
            false,
        )
        .unwrap();
        let x102: X102 = X102::from(&frame);
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
        assert_eq!(x102.status.status_vehicle, true); // EV contactors open
        assert_eq!(x102.status.status_vehicle_charging, false); // No commanded charge
    }

    #[test]
    fn test2() {
        let frame = CANFrame::new(
            0x109,
            [0x2, 0x9A, 0x01, 0x00, 0x0, 0xC0, 0x56, 0x00].as_slice(),
            false,
            false,
        )
        .unwrap();
        let x102: X102 = X102::from(&frame);

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
        assert_eq!(x102.status.status_vehicle, false); // EV contactors closed
        assert_eq!(x102.status.status_vehicle_charging, false); // No commanded charge
    }

    #[test]
    fn test3() {
        let frame = CANFrame::new(
            0x109,
            [0x2, 0x9A, 0x01, 0x00, 0x0, 0xC1, 0x56, 0x00].as_slice(),
            false,
            false,
        )
        .unwrap();
        let x102: X102 = X102::from(&frame);

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
        assert_eq!(x102.status.status_vehicle, false); // EV contactors closed
        assert_eq!(x102.status.status_vehicle_charging, true); // Charge commanded
    }
}
