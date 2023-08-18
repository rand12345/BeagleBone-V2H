use std::net::AddrParseError;

use rumqttc::ClientError;
// use tokio_socketcan::CANFrame;

#[derive(Debug)]
pub enum PreError {
    Error,
    CanTx(u8),
    BadSlice,
    PinInitError(u64),
    PinReleaseError(u64),
    MqttSub(ClientError),
    MqttSend(ClientError),
    SocketError(AddrParseError),
    SocketConnectError(std::io::Error),
    // ChannelSend(tokio::sync::mpsc::error::SendError<CANFrame>),
    CanOpen(tokio_socketcan::Error),
}
impl std::error::Error for PreError {}
impl std::fmt::Display for PreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use PreError::*;
        match self {
            Error => write!(f, "Invalid data"),
            BadSlice => write!(f, "Bad slice data"),
            CanTx(n) => write!(f, "Bad can{n} TX"),
            PinInitError(p) => write!(f, "Pin init fail for {p}"),
            PinReleaseError(p) => write!(f, "Pin release fail for {p}"),
            MqttSub(e) => write!(f, "MQTT subscription failed {e:?}"),
            MqttSend(e) => write!(f, "MQTT send failed {e:?}"),
            SocketError(e) => write!(f, "Meter address parsing failed {e:?}"),
            SocketConnectError(e) => write!(f, "Meter TCP connect failed {e:?}"),
            // ChannelSend(e) => write!(f, "Channel send failed {e:?}"),
            CanOpen(e) => write!(f, "Can bus open failed {e:?}"),
        }
    }
}
