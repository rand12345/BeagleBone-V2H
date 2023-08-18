use crate::error::PreError;
use std::{net::SocketAddr, sync::Arc};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::time::timeout;
use tokio::{
    net::TcpStream,
    sync::Mutex,
    time::{sleep, Duration},
};

use super::config::MeterConfig;

lazy_static::lazy_static! {
    pub static ref METER: Arc<Mutex<f32>> = Arc::new(Mutex::new(0f32));
}

pub async fn meter(config: MeterConfig) -> Result<(), PreError> {
    log::info!("Starting Meter thread");
    // let config = &APP_CONFIG.clone();s
    let address = config.address.clone();
    let socket_addr: SocketAddr = address
        .parse::<SocketAddr>()
        .map_err(|e| PreError::SocketError(e))?;
    log::info!(
        "Connecting to RTU meter: IP:{:?} port:{}",
        socket_addr.ip(),
        socket_addr.port()
    );
    let mut stream = TcpStream::connect(socket_addr)
        .await
        .map_err(|e| PreError::SocketConnectError(e))?;
    let (mut rx, mut tx) = stream.split();

    // Raw modbus params for SDM230 @ 1hz
    let device_id = 1;
    let function_code = 0x04; // Read Holding Registers
    let starting_address = 0x0c;
    let quantity = 2;

    let request = energy_modbus_rtu_request(device_id, function_code, starting_address, quantity);
    log::info!("SDM230 modbus PDU: {request:02x?}");

    loop {
        let mut buf = [0u8; 24];
        sleep(Duration::from_millis(1000)).await;
        if let Err(e) = tx.write(&request).await {
            log::error!("{e:?}")
        }

        let val = match timeout(Duration::from_millis(500), rx.read(&mut buf)).await {
            Ok(Ok(_)) => f32::from_be_bytes(buf[3..=6].try_into().unwrap_or_default()),
            Err(e) => {
                log::error!("Meter TCP read error {e:?}");
                continue;
            }
            _ => continue,
        };
        log::info!("Meter value {} (-ve is export to load)", val);
        {
            *METER.clone().lock().await = val
        }
    }
}

fn energy_modbus_rtu_request(
    device_id: u8,
    function_code: u8,
    starting_address: u16,
    quantity: u16,
) -> [u8; 8] {
    let mut request = [0u8; 8];
    request[0] = device_id;
    request[1] = function_code;
    [request[2], request[3]] = starting_address.to_be_bytes();
    [request[4], request[5]] = quantity.to_be_bytes();
    let crc = calculate_crc(&request[0..6]);
    [request[6], request[7]] = crc.to_le_bytes();
    request
}

#[inline]
fn calculate_crc(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for byte in data {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc >>= 1;
                crc ^= 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}
