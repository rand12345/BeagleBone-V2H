use std::time::Duration;

use super::state::Chademo; //, ChargerState};
use crate::{error::IndraError, global_state::OperationMode, MAX_AMPS};
use chademo_v2::*;
use tokio::time::{sleep, timeout};
use tokio_socketcan::{CANFrame, CANSocket};

pub async fn recv_send(
    can: &mut CANSocket,
    chademo: &mut Chademo,
    debug: bool,
) -> Result<(), IndraError> {
    use futures_util::StreamExt;

    if chademo.fault() {
        return Err(IndraError::Error);
    }
    loop {
        if let Some(Ok(frame)) = timeout(Duration::from_millis(100), can.next())
            .await
            .map_err(|_| IndraError::CanBusRxTimeout(1))?
        {
            if debug {
                log::info!("<< {:02x}: {:02x?}", frame.id(), frame.data());
            }
            match frame.id() {
                0x100 => chademo.x100 = X100::from(&frame),
                0x101 => chademo.x101 = X101::from(&frame),
                0x102 => chademo.x102 = X102::from(&frame),
                0x200 => {
                    chademo.x200 = X200::from(&frame);
                    break;
                }
                _ => continue,
            }
        };
    }

    sleep(Duration::from_millis(10)).await;
    for frame in chademo.tx_frames() {
        if debug {
            log::info!(">> {:02x}: {:02x?}", frame.id(), frame.data());
        }
        can.write_frame(frame)
            .map_err(|_| IndraError::CanTx(1))?
            .await
            .map_err(|e| IndraError::CanTxError((e, 1)))?
    }
    Ok(())
}
