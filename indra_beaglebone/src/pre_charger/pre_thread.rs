use super::can::*;
use super::fans::*;
use super::pwm::Pwm;
use super::{cmd_list, PreCharger, PreCommand, BB_PWM_CHIP, BB_PWM_NUMBER, PREDATA};
// use crate::chademo::state;
use crate::chademo::state::pin_init_out_high;
use crate::chademo::state::PREACPIN;
use crate::data_io::mqtt::CHADEMO_DATA;
use crate::error::IndraError;
// use crate::eventbus::{Event, EvtBus};
use crate::log_error;
use crate::statics::PreRxMutex;
use std::time::Duration;
use sysfs_gpio::Pin;
use tokio::time::sleep;
use tokio::time::{timeout, Instant};
use tokio_socketcan::CANFrame;

pub async fn init(pre_rx_m: PreRxMutex) -> Result<(), IndraError> {
    log::info!("Starting Pre thread {}", tokio::task::id());
    let t100ms = Duration::from_millis(100);
    let mut pre = PreCharger::default();
    let mut can_socket =
        tokio_socketcan::CANSocket::open("can0").map_err(|e| IndraError::CanOpen(e))?;
    let predata = PREDATA.clone();
    let pwm = Pwm::new(BB_PWM_CHIP, BB_PWM_NUMBER, 1000).unwrap(); // number depends on chip, etc.
    let mut fan = Fan::new(pwm);
    fan.update(10.0); // turn fans off
    let pre_ac_contactor: Pin = pin_init_out_high(PREACPIN)?;

    sleep(t100ms * 10).await;

    let result = initalise_pre(t100ms, &mut can_socket, &mut pre).await;
    log::warn!("Init {result:?}");

    pre.set_state(crate::pre_charger::PreState::Init);

    {
        *predata.lock().await = pre; // copy data
    }

    enabled_wait(t100ms, &mut can_socket, &mut pre).await;

    pre.set_state(crate::pre_charger::PreState::Online);

    let cmd_list = cmd_list();
    let mut pre_rx = pre_rx_m.lock().await;

    loop {
        let instant = Instant::now();
        read_pre(&cmd_list, &mut can_socket, t100ms, &mut pre).await;
        update_fan(&mut pre, &mut fan);

        while instant.elapsed().as_millis().le(&70) {
            while let Ok(Some(cmd)) = timeout(Duration::from_millis(10), pre_rx.recv()).await {
                log::debug!("Received {cmd:?}");
                if matches!(cmd, PreCommand::Shutdown) {
                    use crate::pre_charger::PreState::Offline;
                    predata.lock().await.set_state(Offline);
                    pre_ac_contactor
                        .set_value(0)
                        .map_err(|e| IndraError::PinAccess(e))?;
                    return Ok(());
                } else {
                    write_pre(cmd, &mut can_socket, t100ms / 10, &mut pre).await;
                }
            }
        }

        // update MQTT struct
        {
            *predata.lock().await = pre;
        }
        if let Ok(mut data) = CHADEMO_DATA.try_lock() {
            data.from_pre(pre);
        };
        println!("{}", pre);
        if instant.elapsed().as_millis().gt(&100) {
            log::warn!("loop time > 100ms");
            dbg!(instant.elapsed().as_millis());
        }
    }
}

async fn write_pre(
    cmd: PreCommand,
    can_socket: &mut tokio_socketcan::CANSocket,
    t100ms: Duration,
    pre: &mut PreCharger,
) {
    log::debug!("New pre_cmd {:?}", cmd);
    let frame = cmd.to_can();
    if let Ok(rx) = can_send_recv(can_socket, frame, t100ms).await {
        log_error!("Send pre cmd", pre.from_slice(rx.data()));
    };
}

fn update_fan(pre: &mut PreCharger, fan: &mut Fan) {
    if pre.enabled() || pre.get_temp() > 55.0 {
        pre.fan_duty(fan.update(pre.get_temp()));
    } else {
        fan.update(10.0);
        pre.fan_duty(fan.update(10.0));
    }
}

async fn read_pre(
    cmd_list: &[u16],
    can_socket: &mut tokio_socketcan::CANSocket,
    t100ms: Duration,
    pre: &mut PreCharger,
) {
    for address in cmd_list.iter() {
        let addr = address.to_le_bytes();
        let p: [u8; 8] = [0x40, addr[0], addr[1], 0, 0, 0, 0, 0];
        // p[0] = 0x40;

        let frame = CANFrame::new(0x630, &p, false, false).unwrap();

        if let Ok(rx) = can_send_recv(can_socket, frame, t100ms).await {
            if pre.from_slice(rx.data()).is_ok() {
                // break;
            }
        }
    }
}
