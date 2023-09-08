use super::{PreCharger, PreState, Register, PREDATA};
use crate::{error::IndraError, pre_charger::Command};
use std::time::Duration;
use tokio::time::{sleep, timeout};
use tokio_socketcan::{CANFrame, CANSocket};

#[inline]
fn debug_frame(frame: &CANFrame) -> String {
    let reg: Register = u16::from_le_bytes([frame.data()[1], frame.data()[2]]).into();
    let cmd: Command = frame.data()[0].into();
    let val = u16::from_le_bytes([frame.data()[4], frame.data()[5]]);
    format!("{cmd:?} {reg:?} {val}")
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn debug_frame_test() {
        let frame = CANFrame::new(
            0x5d0,
            &[0x40, 0x1, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap();
        println!("{:?}", debug_frame(&frame));
        assert!(true)
    }
}

pub async fn can_send_recv(
    can_socket: &mut CANSocket,
    txframe: CANFrame,
    timeout: Duration,
) -> Result<CANFrame, IndraError> {
    use futures_util::StreamExt;
    log::trace!("Tx>>Pre {:02x?} {} ", txframe.data(), debug_frame(&txframe));
    can_socket
        .write_frame(txframe)
        .map_err(|e| IndraError::CanBusWrite(0, e))?
        .await
        .map_err(|e| IndraError::CanBusWriteIo(0, e))?;
    match tokio::select! {
        rx = can_socket.next() => rx,
        _ = tokio::time::sleep(timeout) => None
    } {
        Some(Ok(f)) => {
            log::trace!("Rx<<Pre {:02x?} {}", f.data(), debug_frame(&f));
            Ok(f)
        }
        _ => Err(IndraError::CanBusRxTimeout(0)),
    }
}

pub async fn initalise_pre(
    t100ms: Duration,
    can_socket: &mut tokio_socketcan::CANSocket,
    pre: &mut PreCharger,
) -> Result<(), IndraError> {
    if timeout(t100ms * 50, initalise(t100ms, can_socket, pre))
        .await
        .map_err(|_| IndraError::Timeout)?
        .is_ok()
    {
        timeout(t100ms * 150, enabled_wait(t100ms, can_socket, pre))
            .await
            .map_err(|_| IndraError::Timeout)?;

        pre.set_state(PreState::Online);
        *PREDATA.lock().await = *pre; // copy data
    }
    Ok(())
}

async fn initalise(
    t100ms: Duration,
    can_socket: &mut tokio_socketcan::CANSocket,
    pre: &mut PreCharger,
) -> Result<(), IndraError> {
    for (idx, frame) in init_frames().into_iter().enumerate() {
        let mut fail_count = 0u8;
        loop {
            log::debug!("Pre-init stage {}/{}", idx + 1, init_frames().len());
            sleep(t100ms * 2).await;
            let rx = match can_send_recv(can_socket, frame, t100ms).await {
                Ok(rx) => rx,
                Err(e) => {
                    log::error!("{e}");
                    fail_count += 1;
                    if fail_count > 10 {
                        return Err(IndraError::PreInitFailed);
                    }
                    continue;
                }
            };

            if pre.from_slice(rx.data()).is_ok() {
                match rx.data() {
                    [0x4b, 00, 21, 00, 00, 00, 00, 00] => continue,
                    [0x4b, 01, 21, a, b, 00, 00, 00] => {
                        if (a, b) != (&0, &0) {
                            {
                                log::error!("Invalid Pre state {:x?}", (a, b));
                                continue;
                            };
                        }
                    }
                    _ => (),
                }
                break;
            }
        }
    }

    Ok(())
}

#[inline]
pub async fn enabled_wait(
    t100ms: Duration,
    can_socket: &mut tokio_socketcan::CANSocket,
    pre: &mut PreCharger,
) {
    while !pre.enabled() {
        loop {
            sleep(t100ms).await;
            if let Ok(rx) = can_send_recv(can_socket, status_frame(), t100ms).await {
                if pre.from_slice(rx.data()).is_ok() {
                    log::info!("Status ok");
                    break;
                };
            }
        }
        for frame in enable_frames() {
            sleep(t100ms).await;
            if let Ok(rx) = can_send_recv(can_socket, frame, t100ms).await {
                if pre.from_slice(rx.data()).is_ok() {};
            }
        }
    }
    log::info!("Pre enabled");
}

fn status_frame() -> CANFrame {
    CANFrame::new(
        0x630,
        &[0x40, 0x1, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
        false,
        false,
    )
    .unwrap()
}

#[inline]
pub fn enable_frames() -> [CANFrame; 2] {
    [
        CANFrame::new(
            0x630,
            &[0x2b, 0x0, 0x21, 0x0, 0x1, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x40, 0x0, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
    ]
}

#[inline]
pub fn init_frames() -> [CANFrame; 8] {
    [
        CANFrame::new(
            0x630,
            &[0x40, 0x8, 0x10, 0x4, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x40, 0x9, 0x10, 0x4, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x40, 0xA, 0x10, 0x4, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x40, 0x1, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        // waits until ready
        CANFrame::new(
            0x630,
            &[0x2b, 0x0, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            // &[0x2b, 0x0, 0x21, 0x0, 0x1, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x2b, 0xa, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x2b, 0x9, 0x21, 0x0, 0x0, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
        CANFrame::new(
            0x630,
            &[0x2b, 0x0, 0x21, 0x0, 0x1, 0x0, 0x0, 0x0],
            false,
            false,
        )
        .unwrap(),
    ]
}
