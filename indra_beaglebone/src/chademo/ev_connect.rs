use crate::{
    async_timeout_loop, async_timeout_result,
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
    timeout_condition, MAX_AMPS, MAX_SOC, METER_BIAS, MIN_SOC,
};
use chademo_v2::{X109Status, X108};
use log::warn;
use std::{sync::Arc, time::Duration};
use sysfs_gpio::Pin;
use tokio::{
    sync::Mutex,
    time::{sleep, timeout, Instant},
};
use tokio_socketcan::{CANFrame, CANSocket};

const DUMMYMODE: bool = false;

pub async fn ev100ms(led_tx: LedTx, mode_rx: ChademoRx) -> Result<(), IndraError> {
    log::info!("Starting EV thread");

    // let operational_mode = OPERATIONAL_MODE.clone();
    let mut chademo = Chademo::new();
    let t100ms = Duration::from_millis(100);
    let predata = PREDATA.clone();
    let (pre_tx, pre_rx) = statics::pre_channel();
    let pre_rx = mutex(pre_rx);
    let mode_rx = mutex(mode_rx);
    use tokio::task::JoinHandle;
    let mut handles: Vec<JoinHandle<Result<(), IndraError>>> = Vec::new(); // Store spawned task handles
    loop {
        for handle in handles.drain(..) {
            log::info!("Aborting Pre thread {}", handle.id());
            handle.abort(); // Abort the previous tasks
        }
        reset_gpio_state(&mut chademo);
        chademo.set_state(OperationMode::Idle);
        update_panel_leds(&led_tx, &chademo).await;
        update_chademo_mutex(&chademo).await;
        let mut can = tokio_socketcan::CANSocket::open(&"can1").map_err(|_| IndraError::Error)?;
        {
            if let Some(state) = mode_rx.clone().lock().await.recv().await {
                chademo.set_state(state);
                update_panel_leds(&led_tx, &chademo).await;
                update_chademo_mutex(&chademo).await;
                if !(state.is_v2h() || state.is_charge()) {
                    continue;
                }
                if matches!(state, OperationMode::Quit) {
                    return Ok(());
                }
            }
        }

        if DUMMYMODE {
            log::info!("            Entering charge loop!");
            let _ = match charge_mode(&mut chademo, &mut can, &pre_tx, &led_tx, mode_rx.clone())
                .await
            {
                Ok(reason) => reason,
                Err(e) => {
                    log::error!("Bailed out of main charge {e:?}");
                    OperationMode::Idle
                }
            };
            continue;
        }
        // let mut ev_connect_timeout: Option<Instant> = Some(Instant::now());

        //
        // Spawn a watchdog timer which changed OP to Idle after timeout
        //
        log::info!("{:?} active", chademo.state());

        // Spawn new task
        let handle = tokio::spawn(pre_thread::init(pre_rx.clone()));
        log::info!("Spawned new Pre thread {}", handle.id());
        handles.push(handle);

        chademo.pins().pre_ac.set_value(1).unwrap();
        if let Err(e) = init_pre(&predata, t100ms, &pre_tx).await {
            log::error!("Pre init failed - {e:?}");

            chademo.set_state(OperationMode::Idle);
            reset_gpio_state(&mut chademo);
            update_chademo_mutex(&chademo).await;
            continue;
        };
        chademo.x109.status = X109Status::from(0x20);
        assert!(!chademo.x109.status.status_vehicle_connector_lock);
        assert!(!chademo.x109.status.status_station);
        log::info!("Raise D1");
        log_error!("Setting D1 high", chademo.pins().d1.set_value(1));
        update_chademo_mutex(&chademo).await;

        chademo.plug_lock(true).expect("Plug lock failed");
        chademo.x109.status = X109Status::from(0x24);
        assert!(chademo.x109.status.status_vehicle_connector_lock);
        log::info!("Check can frames & Wait for K line");
        if let Err(e) = k_line(&mut can, &mut chademo).await {
            log::error!("K line init failed - is car connected? {e:?}");

            chademo.set_state(OperationMode::Idle);
            reset_gpio_state(&mut chademo);
            update_chademo_mutex(&chademo).await;
            continue;
        };

        // chademo.precharge();
        log::info!("insulation tests skipped !!!");
        chademo.pins().d2.set_value(1).unwrap();

        update_chademo_mutex(&chademo).await;
        log::info!("when voltage match - raise D2");
        if let Err(e) = precharge(&mut can, &mut chademo, &pre_tx, &predata).await {
            log::error!("precharge & contactor init failed - should be catastropic and hang {e:?}");

            chademo.set_state(OperationMode::Idle);
            reset_gpio_state(&mut chademo);
            update_chademo_mutex(&chademo).await;
            continue;
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

        log::warn!("End of init fn 'end charge'");
        update_chademo_mutex(&chademo).await;
        chademo.x109.status = X109Status::from(0x24);
        chademo.charging_stop_control_release();
        log_error!("Shutdown pre", pre_tx.send(PreCommand::Shutdown).await);
        let mut contactors = true;
        loop {
            match timeout(
                Duration::from_millis(200),
                recv_send(&mut can, &mut chademo, false),
            )
            .await
            {
                Ok(Ok(_)) => (),
                Ok(Err(e)) => {
                    log::error!("CAN error on closure {e?}");
                    if !contactors && !chademo.x102.status.status_vehicle {
                        break;
                    }
                }
                Err(e) => {
                    log::warn!("CAN timed out on closure {e?}");
                    if !contactors && !chademo.x102.status.status_vehicle {
                        break;
                    }
                }
            };

            if matches!(chademo.pins().k.get_value(), Ok(0)) {
                continue;
            };
            if contactors {
                log::info!("Contactors opening");
                if chademo.pins().c1.set_value(0).is_ok() {
                    print!("\x07");
                    if chademo.pins().c2.set_value(0).is_ok() {
                        print!("\x07");
                        warn!("                                       !!!!CONTACTORS OPEN!!!!");

                        contactors = false;
                    }
                }
            } else {
                log::warn!("Conditions not yet met for contactor open");
                continue;
            }

            chademo.x109.status = X109Status::from(0x20); // make this an enum
                                                          // if !chademo.x102.status.status_vehicle {
                                                          //     break;
                                                          // }
        }
        log::warn!("Charge/discharge mode ended");

        if matches!(exit_reason, crate::global_state::OperationMode::Quit) {
            return Ok(());
        }
        drop(can);
        //loops back to idleß
    }
}
fn reset_gpio_state(chademo: &mut Chademo) {
    log_error!("Exit charge: c2", chademo.pins().c2.set_value(0));
    log_error!("Exit charge: c1", chademo.pins().c1.set_value(0));
    log_error!("Exit charge: d2", chademo.pins().d2.set_value(0));
    log_error!("Exit charge: d1", chademo.pins().d1.set_value(0));
    log_error!("Exit charge: Pre AC", chademo.pins().pre_ac.set_value(0));
    log_error!(
        "Exit charge: pluglock",
        chademo.pins().pluglock.set_value(0)
    );
    chademo.x109.status = X109Status::from(0x24);
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
    use crate::global_state::OperationMode::*;

    let exit_reason = loop {
        if DUMMYMODE {
            sleep(Duration::from_millis(100)).await
        } else {
            recv_send(can, chademo, false).await?;
            if !chademo.status_vehicle_charging() {
                log::warn!("EV stopped charge");
                break Idle;
            }
        };

        {
            // listen for incomming mode changes
            if let Ok(op) = mode_rx.try_recv() {
                let op = match (chademo.state(), op) {
                    (V2h, V2h) => Idle,
                    (V2h, Charge(p)) => Charge(p),
                    (_, Idle) => Idle,
                    (Charge(_), V2h) => V2h,
                    (_, Discharge(p)) => Discharge(p),
                    (_, Quit) => Quit,
                    _ => *chademo.state(),
                };
                update_panel_leds(&led_tx, &chademo).await;
                log::info!("New CHAdeMO mode received {op:?}");
                chademo.set_state(op)
            }
            // update_panel_leds(&led_tx, &chademo).await
        }
        update_chademo_mutex(chademo).await;

        let op = chademo.state();

        let charging_current_request = match *op {
            V2h => amps_meter_profiler(&mut feedback, &last_amps, &*chademo).await,
            Discharge(d) => match handle_discharge_mode(&d, &chademo).await {
                Some(amps) => amps,
                None => break Idle,
            },
            Charge(c) => match c.get_eco() {
                false => match handle_charge_mode(&c, &chademo).await {
                    Some(amps) => amps,
                    None => break Idle,
                },
                true => amps_meter_profiler(&mut feedback, &last_amps, &*chademo)
                    .await
                    .clamp(0.0, MAX_AMPS as f32), //
            },
            Quit | Idle => break *op,
            _ => continue,
        };
        if &last_volts != chademo.target_voltage() {
            last_volts = *chademo.target_voltage();
            log_error!(
                "",
                pre_tx.send(PreCommand::DcVoltsSetpoint(last_volts)).await
            );
        }

        // testing!!!!!!!
        // chademo.update_dynamic_charge_limits(charging_current_request);

        if last_amps != charging_current_request as f32 {
            last_amps = charging_current_request as f32;
            log_error!(
                "",
                pre_tx
                    .send(PreCommand::DcAmpsSetpoint(charging_current_request))
                    .await
            );

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

async fn handle_charge_mode(cp: &ChargeParameters, chademo: &Chademo) -> Option<f32> {
    let mut amps = Some((cp.get_amps() as f32).min(chademo.requested_charging_amps()));
    if let Some(soc_limit) = cp.get_soc_limit() {
        if &soc_limit <= chademo.soc() {
            amps = None;
            log::info!("Charge to SoC limit hit, charging disabled")
        }
    }
    amps
}
async fn handle_discharge_mode(cp: &ChargeParameters, chademo: &Chademo) -> Option<f32> {
    // note negative Some()
    let mut amps = Some(-(cp.get_amps() as f32).min(chademo.requested_discharging_amps()));
    if let Some(soc_limit) = cp.get_soc_limit() {
        if &soc_limit <= chademo.soc() {
            amps = None;
            log::info!("Charge to SoC limit hit, charging disabled")
        }
    }
    amps
}

async fn init_pre(
    predata: &std::sync::Arc<tokio::sync::Mutex<crate::pre_charger::PreCharger>>,
    t100ms: Duration,
    pre_tx: &tokio::sync::mpsc::Sender<PreCommand>,
) -> Result<(), IndraError> {
    log::info!("Initalise PRE");
    let mut c = false;
    let mut counter = 0;
    while !c {
        if counter > 10 {
            return Err(IndraError::Timeout);
        }
        counter += 1;
        sleep(Duration::from_millis(1000)).await;
        let pre = predata.lock().await;
        c = pre.get_state().is_online()
    }
    log::info!("Pre stage 1");
    log_error!("", pre_tx.send(PreCommand::DcAmpsSetpoint(1.0)).await);
    sleep(t100ms).await;
    log_error!("", pre_tx.send(PreCommand::DcVoltsSetpoint(370.0)).await);
    sleep(t100ms).await;

    c = false;
    counter = 0;
    while !c {
        if counter > 50 {
            return Err(IndraError::Timeout);
        }
        sleep(Duration::from_millis(100)).await;
        let pre = predata.lock().await;
        if pre.get_dc_setpoint_volts() as u16 == 370 && pre.get_dc_setpoint_amps() as u16 == 1 {
            c = pre.get_dc_output_volts() as u16 == pre.get_dc_setpoint_volts() as u16;
        };
    }

    log::info!("Pre stage 2");
    Ok(())
}

async fn k_line(can: &mut CANSocket, chademo: &mut Chademo) -> Result<(), IndraError> {
    let mut counter = 100u8; //10 seconds
    sleep(Duration::from_millis(100)).await;
    while counter != 0 {
        //100ms loop
        recv_send(can, chademo, false).await?;
        if matches!(chademo.pins().k.get_value(), Ok(0)) {
            log::info!("K line ok");
            if chademo.x102_status().status_vehicle_charging {
                log::info!("102.5.0 ok");
                return Ok(());
            };
        };
        counter -= 1
    }
    Err(IndraError::Timeout)
}

async fn precharge(
    can: &mut CANSocket,
    chademo: &mut Chademo,
    pre_tx: &tokio::sync::mpsc::Sender<PreCommand>,
    predata: &Arc<Mutex<PreCharger>>,
) -> Result<(), IndraError> {
    let mut old_soc = chademo.x102.state_of_charge;
    let mut counter = 50u8;
    while counter != 0 {
        counter -= 1;
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
                        return Ok(());
                    }
                }
            } else {
                log::warn!("Pre volts not equal")
            };
        } else {
            log::warn!("x102.5.3 high")
        }
    }
    Err(IndraError::Timeout)
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

/*
ndra_beaglebone::chademo::ev_connect] Update logo state Ok()
2023-08-31T14:30:21.600Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED SoC Ok()
2023-08-31T14:30:21.601Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED Energy Ok()
2023-08-31T14:30:21.608Z DEBUG [indra_beaglebone::chademo::ev_connect] Update logo state Ok()
2023-08-31T14:30:21.610Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED SoC Ok()
2023-08-31T14:30:21.611Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED Energy Ok()
PRE: Charging EV 3254.16W, temp: 25.90ªC dc_output: 387.40V 8.40A, dc_output_setpoint: 410.00V 8.50A, fan: 0 enabled: true
2023-08-31T14:30:21.695Z DEBUG [indra_beaglebone::chademo::ev_connect] Update logo state Ok()
2023-08-31T14:30:21.697Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED SoC Ok()
2023-08-31T14:30:21.698Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED Energy Ok()
2023-08-31T14:30:21.709Z DEBUG [indra_beaglebone::chademo::ev_connect] Update logo state Ok()
2023-08-31T14:30:21.710Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED SoC Ok()
2023-08-31T14:30:21.711Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED Energy Ok()
PRE: Charging EV 3294.60W, temp: 25.90ªC dc_output: 387.60V 8.50A, dc_output_setpoint: 410.00V 8.50A, fan: 0 enabled: true
2023-08-31T14:30:21.795Z DEBUG [indra_beaglebone::chademo::ev_connect] Update logo state Ok()
2023-08-31T14:30:21.797Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED SoC Ok()
2023-08-31T14:30:21.798Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED Energy Ok()
2023-08-31T14:30:21.808Z DEBUG [indra_beaglebone::chademo::ev_connect] Update logo state Ok()
2023-08-31T14:30:21.810Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED SoC Ok()
2023-08-31T14:30:21.811Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED Energy Ok()
PRE: Charging EV 3292.90W, temp: 25.90ªC dc_output: 387.40V 8.50A, dc_output_setpoint: 410.00V 8.50A, fan: 0 enabled: true
2023-08-31T14:30:21.897Z DEBUG [indra_beaglebone::chademo::ev_connect] Update logo state Ok()
2023-08-31T14:30:21.898Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED SoC Ok()
2023-08-31T14:30:21.899Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED Energy Ok()
PRE: Charging EV 3294.60W, temp: 25.90ªC dc_output: 387.60V 8.50A, dc_output_setpoint: 410.00V 8.50A, fan: 0 enabled: true
2023-08-31T14:30:21.907Z DEBUG [indra_beaglebone::chademo::ev_connect] Update logo state Ok()
2023-08-31T14:30:21.908Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED SoC Ok()
2023-08-31T14:30:21.909Z DEBUG [indra_beaglebone::chademo::ev_connect] Update LED Energy Ok()
PRE: Charging EV 3291.20W, temp: 25.90ªC dc_output: 387.20V 8.50A, dc_output_setpoint: 410.00V 8.50A, fa
*/
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
    use chademo_v2::{X102, X109};

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
