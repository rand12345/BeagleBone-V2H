use crate::{
    chademo::state::OPERATIONAL_MODE,
    data_io::mqtt::{MqttChademo, CHADEMO_DATA},
};
use futures_util::{future, StreamExt, TryStreamExt};
use log::info;
use serde::{Deserialize, Serialize};
use std::io::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

pub async fn run() -> Result<(), Error> {
    let addr = "0.0.0.0:5555".to_string();
    let try_socket = TcpListener::bind(&addr).await;
    let listener = try_socket.expect("Failed to bind");
    info!("Listening on: {}", addr);
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(accept_connection(stream));
    }
    Ok(())
}

async fn accept_connection(stream: TcpStream) {
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
    let process_incoming_ws = |m: Result<Message, Error>| -> Result<Message, Error> {
        println!("{:?}", m);

        if let Ok(Message::Text(cmd)) = m {
            info!("{:?}", cmd);
            match serde_json::from_str::<Instruction>(&cmd) {
                Ok(d) => match d.cmd {
                    Cmd::SetMode(mode) => {
                        // !!!
                        if let Ok(mut opm) = OPERATIONAL_MODE.try_lock() {
                            *opm = mode;
                            log::info!("Mode instruction {:?}", mode)
                        };
                        let response = Response::Mode(mode);
                        Ok(Message::Text(serde_json::to_string(&response).unwrap()))
                    }
                    Cmd::GetJson => {
                        let response = Response::Data(data);
                        Ok(Message::Text(serde_json::to_string(&response).unwrap()))
                    }
                },
                Err(_) => Ok(Message::Text(r#"{"ack": "err"}"#.to_owned())),
            }
        } else {
            return Err(Error::Utf8);
        }
    };

    // {"cmd": {"SetMode": "Charge"}}
    // {"cmd": "GetJson"}

    let result = read
        .try_filter(|msg| future::ready(msg.is_text() || msg.is_binary()))
        .map(|s| process_incoming_ws(s))
        .forward(write)
        .await;
    if let Err(e) = result {
        eprintln!("ws error {e:?}")
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
enum Cmd {
    SetMode(OperationMode),
    #[default]
    GetJson,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct Instruction {
    cmd: Cmd,
}
#[derive(Serialize, Deserialize, Debug)]
enum Response {
    Data(MqttChademo),
    Mode(OperationMode),
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy)]
pub enum OperationMode {
    V2h,
    Charge,
    #[default]
    Idle,
}
impl OperationMode {
    // pub fn action(&self) -> Result<(), TryLockError> {
    //     *OPERATIONAL_MODE.try_lock()? = *self;
    //     Ok(())
    // }
    pub fn boost(&mut self) {
        use OperationMode::*;
        *self = match self {
            V2h | Idle => Charge,
            Charge => V2h,
        }
    }
    pub fn onoff(&mut self) {
        use OperationMode::*;
        *self = match self {
            V2h | Charge => Idle,
            Idle => V2h,
        }
    }
    pub fn idle(&mut self) {
        use OperationMode::*;
        *self = Idle;
    }
    pub fn is_idle(&self) -> bool {
        use OperationMode::*;
        matches!(self, Idle)
    }
}
