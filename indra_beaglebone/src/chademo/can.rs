use std::time::Duration;

use super::state::Chademo; //, ChargerState};
use crate::{error::IndraError, global_state::OperationMode, MAX_AMPS};
use chademo_v2::chademo::*;
use tokio::time::{sleep, timeout};
use tokio_socketcan::{CANFrame, CANSocket};

/// Notes from:
/// IEEE Std 2030.1.1-2021
/// IEEE Standard for Technical Specifications of a DC Quick Charger for Use with Electric Vehicles

pub async fn recv_send(
    can: &mut CANSocket,
    chademo: &mut Chademo,
    debug: bool,
) -> Result<(), IndraError> {
    use futures_util::StreamExt;

    if chademo.fault() {
        let err = Err(IndraError::Error);
        return err;
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
                0x102 => chademo.x102 = X102::from(&frame),
                0x200 => {}
                _ => continue,
            }
        };
        sleep(Duration::from_millis(5)).await;
        for frame in chademo.tx_frames() {
            if debug {
                log::info!(">> {:02x}: {:02x?}", frame.id(), frame.data());
            }
            can.write_frame(frame)
                .unwrap()
                .await
                .map_err(|_| IndraError::CanTx(1))?
        }
        break;
    }
    Ok(())
}
