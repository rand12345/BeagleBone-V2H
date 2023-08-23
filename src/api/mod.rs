use crate::{
    chademo::state::OPERATIONAL_MODE,
    data_io::mqtt::{MqttChademo, CHADEMO_DATA},
    MAX_AMPS,
};
use futures_util::{future, StreamExt, TryStreamExt};
use log::info;
use serde::{Deserialize, Serialize};
use std::{io::Error, ops::Deref};
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
                            *opm = mode.clone();
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
#[derive(Serialize, Debug)]
enum Response {
    Data(MqttChademo),
    Mode(OperationMode),
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ChargeParameters {
    amps: u8,
    eco: bool,
    soc_limit: Option<u8>,
}

impl ChargeParameters {
    fn amps(&mut self, limit: u8) -> Self {
        self.amps = limit;
        *self.deref()
    }
    fn eco(&mut self, enabled: bool) -> Self {
        self.eco = enabled;
        *self.deref()
    }
    fn soc_limit(&mut self, soc_limit: u8) -> Self {
        self.soc_limit = Some(soc_limit);
        *self.deref()
    }
}
impl Default for ChargeParameters {
    fn default() -> Self {
        Self {
            amps: MAX_AMPS,
            eco: false,
            soc_limit: None,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub enum OperationMode {
    V2h,
    Charge(ChargeParameters),
    #[default]
    Idle,
}
impl OperationMode {
    pub fn eco_charge(&mut self, enabled: bool) {
        let cp = ChargeParameters::default().eco(enabled);
        *self = Self::Charge(cp)
    }
    pub fn limit_soc(&mut self, limit: u8) {
        let cp = ChargeParameters::default().soc_limit(limit);
        *self = Self::Charge(cp)
    }
    pub fn limit_amps(&mut self, limit: u8) {
        let cp = ChargeParameters::default().amps(limit);
        *self = Self::Charge(cp)
    }
    pub fn is_eco(&self) -> bool {
        match self {
            OperationMode::Charge(p) => p.eco,
            _ => false,
        }
    }
    pub fn soc_limit(&self) -> Option<u8> {
        match self {
            OperationMode::Charge(p) => p.soc_limit,
            _ => None,
        }
    }
    pub fn amps_limit(&self) -> u8 {
        match self {
            OperationMode::Charge(p) => p.amps,
            _ => 0,
        }
    }
    pub fn boost(&mut self) {
        use OperationMode::*;
        *self = match self {
            V2h | Idle => Charge(ChargeParameters::default()),
            Charge(_) => V2h,
        }
    }
    pub fn onoff(&mut self) {
        use OperationMode::*;
        *self = match self {
            V2h | Charge(_) => Idle,
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
    pub fn is_charge(&self) -> bool {
        use OperationMode::*;
        matches!(self, Charge(_))
    }
    pub fn is_v2h(&self) -> bool {
        use OperationMode::*;
        matches!(self, Charge(_))
    }
}
