use crate::{
    data_io::mqtt::{MqttChademo, CHADEMO_DATA},
    global_state::OperationMode,
    log_error,
    scheduler::{get_eventfile_sync, Events},
    statics::{ChademoTx, EventsTx, OPERATIONAL_MODE},
};
use futures_util::{future, StreamExt, TryStreamExt};
use log::info;
use serde::{Deserialize, Serialize};
use std::{io::Error, str::FromStr};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

const BAD_ACK: &str = r#"{"ack": "err"}"#; // temp, use error handling

pub async fn run(events_tx: EventsTx, mode_tx: ChademoTx) -> Result<(), Error> {
    let addr = "0.0.0.0:5555".to_string();
    let try_socket = TcpListener::bind(&addr).await;
    let listener = try_socket.expect("Failed to bind");
    info!("Listening on: {}", addr);
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(accept_connection(
            stream,
            events_tx.clone(),
            mode_tx.clone(),
        ));
    }
    Ok(())
}

async fn accept_connection(stream: TcpStream, events_tx: EventsTx, mode_tx: ChademoTx) {
    let addr = stream
        .peer_addr()
        .expect("connected streams should have a peer address");
    info!("Peer address: {}", addr);

    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Error during the websocket handshake occurred");

    info!("New WebSocket connection: {}", addr);

    let (write, read) = ws_stream.split();

    use tokio_tungstenite::tungstenite::Error;

    let data = { *CHADEMO_DATA.lock().await };
    let process_incoming_ws = |msg: Result<Message, Error>| -> Result<Message, Error> {
        let cmd = match msg {
            Ok(Message::Text(cmd)) => {
                info!("Text: {:?}", cmd);
                cmd
            }
            Ok(Message::Binary(cmd)) => {
                info!("Binary: {:x?}", cmd);
                let cmd = String::from_utf8_lossy(&cmd).into();
                cmd
            }
            _ => {
                return Err(Error::Utf8);
            }
        };
        process_ws_message(&cmd, data, &events_tx, &mode_tx)
    };

    // {"cmd": {"SetMode": {"Charge": {"amps": 15, "eco": false, "soc_limit": 100}}}}
    // {"cmd": {"SetMode": "V2h"}}
    // {"cmd": {"SetMode": "Idle"}}
    // {"cmd": "GetJson"}
    // {"cmd": "GetEvents"}
    // {"cmd": {"SetEvents": [{"time": "00:01:02", "Action": "Charge"}, {"time": "00:02:32", "Action": "V2h"}]}}

    // pub enum Action {
    //     Charge,
    //     Discharge,
    //     Sleep,
    //     V2h,
    //     Eco,
    // }

    // #[derive(Debug, Deserialize, Serialize, PartialEq, Clone, Copy)]
    // pub struct Event {
    //     time: NaiveTime,
    //     action: Action,
    // }

    let result = read
        .try_filter(|msg| future::ready(msg.is_text() || msg.is_binary()))
        .map(|s| process_incoming_ws(s))
        .forward(write)
        .await;
    if let Err(e) = result {
        eprintln!("ws error {e:?}")
    }
}

/// Cannot be async
fn process_ws_message(
    cmd: &str,
    data: MqttChademo,
    events_tx: &EventsTx,
    mode_tx: &ChademoTx,
) -> Result<Message, tokio_tungstenite::tungstenite::Error> {
    match serde_json::from_str::<Instruction>(&cmd) {
        Ok(d) => match d.cmd {
            Cmd::SetMode(mode) => {
                let mode_tx_blocking = mode_tx.clone();
                tokio::task::spawn_blocking(move || {
                    log_error!(
                        format!("Mode instruction {:?}", mode),
                        mode_tx_blocking.blocking_send(mode)
                    )
                });
                let response = Response::Mode(mode);
                Ok(Message::Text(serde_json::to_string(&response).unwrap()))
            }
            Cmd::GetJson => {
                let response = Response::Data(data);
                Ok(Message::Text(serde_json::to_string(&response).unwrap()))
            }
            Cmd::SetEvents(events) => {
                let val = match toml::Value::from_str(&events) {
                    Ok(val) => val,
                    Err(_) => return Ok(Message::Text(BAD_ACK.to_owned())),
                };

                match val.try_into::<Events>() {
                    Ok(nv) => {
                        if let Err(e) = events_tx.blocking_send(nv) {
                            log::error!("{e:?}");
                            return Ok(Message::Text(BAD_ACK.to_owned()));
                        };
                        Ok(Message::Text(events))
                    }
                    Err(_) => return Ok(Message::Text(BAD_ACK.to_owned())),
                }
            }
            Cmd::GetEvents => match get_eventfile_sync() {
                Some(events_json) => Ok(Message::Text(events_json)),
                None => return Ok(Message::Text(BAD_ACK.to_owned())),
            },
        },
        Err(_) => Ok(Message::Text(BAD_ACK.to_owned())),
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
enum Cmd {
    SetMode(OperationMode),
    #[default]
    GetJson,
    SetEvents(String),
    GetEvents,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct Instruction {
    cmd: Cmd,
}
#[derive(Serialize, Debug)]
enum Response {
    Data(MqttChademo),
    Mode(OperationMode),
}
