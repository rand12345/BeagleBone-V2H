use super::{
    can::*, fans::*, pwm::Pwm, PreCharger, PreCommand, BB_PWM_CHIP, BB_PWM_NUMBER, PREDATA,
};
use crate::{
    chademo::state::{pin_init_out_high, PREACPIN},
    data_io::mqtt::CHADEMO_DATA,
    error::IndraError,
    global_state::OperationMode,
    log_error,
    pre_charger::{cmd_list_outputs, cmd_list_setpoints, PreState},
    statics::PreRxMutex,
};
use std::time::Duration;
use sysfs_gpio::Pin;
use tokio::time::{sleep, timeout, Instant};
use tokio_socketcan::CANFrame;

pub async fn init(pre_rx_m: PreRxMutex) -> Result<(), IndraError> {
    log::info!("Starting Pre thread {}", tokio::task::id());
    let t100ms = Duration::from_millis(100);
    let mut pre = PreCharger::default();
    let mut can_socket =
        tokio_socketcan::CANSocket::open("can0").map_err(|e| IndraError::CanOpen(e))?;
    let predata = PREDATA.clone();
    predata.lock().await.set_state(PreState::Init);
    let pwm = Pwm::new(BB_PWM_CHIP, BB_PWM_NUMBER, 1000).unwrap(); // number depends on chip, etc.
    let mut fan = Fan::new(pwm);
    fan.update(10.0); // turn fans off
    let pre_ac_contactor: Pin = pin_init_out_high(PREACPIN)?;
    sleep(t100ms * 10).await;

    initalise_pre(t100ms, &mut can_socket, &mut pre).await?;

    let setpoints = cmd_list_setpoints();
    let outputs = cmd_list_outputs();
    let mut pre_rx = pre_rx_m.lock().await;
    let mut counter = 0;
    loop {
        counter += 1;
        let instant = Instant::now();
        let cmd_list = if counter % 2 == 0 { setpoints } else { outputs };
        read_pre(&cmd_list, &mut can_socket, t100ms, &mut pre).await;
        update_fan(&mut pre, &mut fan);

        if let Ok(cmd) = pre_rx.try_recv() {
            log::debug!("Received {cmd:?}");
            if !matches!(cmd, PreCommand::Shutdown) {
                write_pre(cmd, &mut can_socket, t100ms / 10, &mut pre).await;
            } else {
                fan.stop();
                pre.set_state(PreState::Offline);
                pre.fan_duty(1);
            }
            // let mut data = predata.lock().await;
        }

        // update MQTT struct
        {
            *predata.lock().await = pre;
        }
        if let Ok(mut data) = CHADEMO_DATA.try_write() {
            data.from_pre(pre);
            if matches!(pre.state, PreState::Offline) {
                pre_ac_contactor
                    .set_value(0)
                    .map_err(|e| IndraError::PinAccess(e))?;
                log::warn!("Pre AC contactor opened");
                return Ok(());
            };
        };

        // 1 sec Pre stats
        if counter > 10 {
            println!("{}", pre);
            counter = 0;
        }
        if instant.elapsed().as_millis().gt(&99) {
            log::warn!("loop time > 100ms");
            dbg!(instant.elapsed().as_millis());
        } else {
            sleep(t100ms - instant.elapsed()).await
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
