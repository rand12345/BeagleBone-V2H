use super::pre_thread::PreCharger;
use crate::error::IndraError;
use std::time::Duration;
use tokio::time::sleep;
use tokio_socketcan::{CANFrame, CANSocket};

pub async fn can_send_recv(
    can_socket: &mut CANSocket,
    txframe: CANFrame,
    timeout: Duration,
) -> Result<CANFrame, IndraError> {
    use futures_util::StreamExt;

    can_socket
        .write_frame(txframe)
        .map_err(|_| IndraError::CanTx(1))?
        .await
        .map_err(|_| IndraError::CanTx(1))?;
    match tokio::select! {
        rx = can_socket.next() => rx,
        _ = tokio::time::sleep(timeout) => None
    } {
        Some(Ok(f)) => Ok(f),
        _ => Err(IndraError::Error),
    }
}

pub async fn enabled_wait(
    t100ms: Duration,
    can_socket: &mut tokio_socketcan::CANSocket,
    pre: &mut PreCharger,
) {
    for frame in enable_frames() {
        sleep(t100ms).await;
        if let Ok(rx) = can_send_recv(can_socket, frame, t100ms).await {
            let _ = pre.from_slice(rx.data()).is_ok();
        }
    }
}

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
            &[0x2b, 0x0, 0x21, 0x0, 0x1, 0x0, 0x0, 0x0],
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
