use futures_util::SinkExt;
use log::*;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_socketcan::CANFrame;

use crate::error::PreError;

pub async fn can_start(
    socket_name: &str,
    send: Sender<CANFrame>,
    mut recv: Receiver<CANFrame>,
) -> Result<(), PreError> {
    use futures_util::StreamExt;
    use tokio_socketcan::CANSocket;
    let mut can_socket = CANSocket::open(&socket_name).map_err(|e| PreError::CanOpen(e))?;

    log::info!("Starting {socket_name} thread");
    if let Err(e) = can_socket.flush().await {
        log::error!("{e:?}")
    };

    loop {
        tokio::select! {
                rx = can_socket.next() => {
                    match rx {
            Some(Ok(f)) => {
                // info!("{socket_name} Rx: {:x} {:x?}", f.id(),f.data());
                            if let Err(e) = send.send(f).await {error!("{socket_name} MPSC send error: {e:?}")}},
            Some(Err(e)) => error!("{socket_name} rx error {e:?}"),
            _ => ()
        }
                    },
                tx = recv.recv() => {
                    match tx {
                        Some(f) => {let r = match can_socket.write_frame(f){
                            // Ok(r)=> {info!("{socket_name} Tx: {:x} {:x?}", f.id(),f.data());r},
                            Ok(r)=> {r},
                            Err(e) => { {error!("{socket_name} tx error: {e:?}")}; continue}
                        };
                    if let Err(e) = r.await  {error!("{socket_name} MPSC recv error: {e:?}")}},
                        None => (),
                        }
                    }
            }
    }
}
