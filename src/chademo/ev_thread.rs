#![allow(unused_assignments)]

use crate::chademo::state::STATE;
use crate::chademo::{can::*, state::Chademo, state::ChargerState};
use crate::error::PreError;
use crate::meter::METER;
use crate::mqtt::CHADEMO_DATA;
use crate::pre_charger::pre_commands::PreCmd;
use crate::pre_thread::PREDATA;
use crate::{log_error, MAX_AMPS, MAX_SOC, METER_BIAS, MIN_SOC};
use futures_util::StreamExt;
use log::warn;
use std::ops::ControlFlow;
use std::time::Duration;
use tokio::time::timeout;
use tokio::{sync::mpsc::Sender, time::sleep};

pub async fn ev100ms(
    send_cmd: Sender<PreCmd>,
    send_state: Sender<ChargerState>,
) -> Result<(), PreError> {
    use ChargerState::*;
    use PreCmd::*;

    log::info!("Starting EV thread");
    let mut can = tokio_socketcan::CANSocket::open(&"can1").map_err(|_| PreError::Error)?;
    let c_state = STATE.clone();
    let mut chademo = Chademo::default();

    let mut x102 = X102::default();
    let x108 = X108::new();
    let mut x109 = X109::new(2);

    let mut feedback = 0.01; // Initial grid energy feedback value
    let t100ms = Duration::from_millis(100);

    let mut setpoint_amps_old = 0f32;
    let mut setpoint_voltage_old = 0f32;
    loop {
        // Get current state from main state loop
        let state = { c_state.lock().await.0 };

        match state {
            Idle => {
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

        dbg!(chademo.status_vehicle_contactors());
        dbg!(chademo.fault());
        dbg!(chademo.can_charge());
        dbg!(chademo.soc());
        if x102.fault() {
            // bail
            let _ = send_state.send(ChargerState::Exiting).await;
            continue;
        }
        warn!("State: {:?}", state);
        match state {
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
