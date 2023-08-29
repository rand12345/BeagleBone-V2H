use crate::{
    chademo::state::CHADEMO,
    data_io::mqtt::{MqttChademo, CHADEMO_DATA},
    global_state::OperationMode,
    log_error,
    scheduler::{get_eventfile_sync, Events},
    statics::{ChademoTx, EventsTx, OPERATIONAL_MODE},
};
use futures_util::{future, StreamExt, TryStreamExt};
use log::info;
use serde::{Deserialize, Serialize};
use std::{io::Error, str::FromStr, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    time::sleep,
};
use tokio_tungstenite::tungstenite::{self, Message};

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
        process_ws_message(&cmd, &events_tx, &mode_tx)
    };

    // {"cmd": {"SetMode": {"Charge": {"amps": 15, "eco": "", "soc_limit": 100}}}}
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
    events_tx: &EventsTx,
    mode_tx: &ChademoTx,
) -> Result<Message, tungstenite::Error> {
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
                log::info!("SetMode => Client {:?}", response);
                Ok(Message::Text(serde_json::to_string(&response).unwrap()))
            }
            Cmd::GetMode => {
                let mode = match CHADEMO.clone().try_lock() {
                    Ok(mode) => *mode.state(),
                    Err(_) => return Err(tungstenite::Error::Utf8),
                };

                let response = Response::Mode(mode);
                log::info!("GetMode => Client {:?}", response);
                Ok(Message::Text(serde_json::to_string(&response).unwrap()))
            }
            Cmd::GetData => match CHADEMO_DATA.clone().try_lock() {
                Ok(d) => {
                    let response = Response::Data(*d);
                    Ok(Message::Text(serde_json::to_string(&response).unwrap()))
                }
                Err(_) => Ok(Message::Text(BAD_ACK.to_string())),
            },
            Cmd::SetEvents(events) => {
                log::info!("SetEvents <= Client {:?}", events);
                let events_tx_c = events_tx.clone();
                let handle = tokio::task::spawn_blocking(move || events_tx_c.blocking_send(events));
                // if handle.is_finished() {
                while !handle.is_finished() {
                    // dangerous loop
                }
                Ok(Message::Text(r#"{"ack": "ok"}"#.to_string()))
            }
            Cmd::GetEvents => {
                let events = match get_eventfile_sync() {
                    Ok(events) => events,
                    Err(_) => return Ok(Message::Text(BAD_ACK.to_owned())),
                };
                let response = Response::Events(events);
                log::info!("GetEvents => Client {:?}", response);
                Ok(Message::Text(serde_json::to_string(&response).unwrap()))
            }
        },
        Err(e) => {
            log::error!("Could not deserialise Instruction {cmd} - {e:?}");
            Ok(Message::Text(BAD_ACK.to_owned()))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test() {
        let cmd: &str = r#"{
  "cmd": {
    "SetEvents": [
      {
        "time": "01:02:03",
        "action": "Sleep"
      },
      {
        "time": "02:03:03",
        "action": "Sleep"
      }
    ]
  }
}"#;
        // let cmd: &str = r#"{"cmd":"GetEvents"}"#;
        // let events = Events::default();
        // let _cmd = Cmd::SetEvents(events);
        // let a = Instruction { cmd: _cmd };
        // let json: String = serde_json::to_string(&a).unwrap();
        // println!("Outout: {json}");

        let _result = match serde_json::from_str::<Instruction>(&cmd) {
            Ok(d) => match d.cmd {
                Cmd::GetEvents => {
                    let events = match get_eventfile_sync() {
                        Ok(events) => events,
                        Err(e) => {
                            panic!("1Could not deserialise Instruction {cmd:?} {e:?}")
                        }
                    };
                    let response = Response::Events(events);
                    log::info!("GetEvents => Client {:?}", response);
                    let output = serde_json::to_string(&response).unwrap();
                    println!("GetEvents test {output}")
                }
                Cmd::SetEvents(events) => {
                    println!("SetEvents <= Client {:?}", events);
                }
                _ => {
                    println!("2Could not deserialise Instruction {cmd}");
                }
            },
            _ => {
                println!("3Could not deserialise Instruction {cmd}");
            }
        };

        // result
    }
}
#[derive(Serialize, Deserialize, Debug, Default)]
enum Cmd {
    SetMode(OperationMode),
    GetMode,
    #[default]
    GetData,
    SetEvents(Events),
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
    Events(Events),
}
