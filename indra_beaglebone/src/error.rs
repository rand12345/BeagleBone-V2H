use rumqttc::ClientError;
use std::net::AddrParseError;
#[allow(dead_code)]
#[derive(Debug)]
pub enum IndraError {
    Error,
    CanTx(u8),
    BadSlice,
    PinInitError(u64),
    PinReleaseError(u64),
    MqttSub(ClientError),
    MqttSend(ClientError),
    SocketError(AddrParseError),
    SocketConnectError(std::io::Error),
    CanOpen(tokio_socketcan::Error),
    TomlParse(toml::ser::Error),
    FileAccess(std::io::Error),
    CanBusWrite(u8, tokio_socketcan::Error),
    CanBusWriteIo(u8, std::io::Error),
    CanBusRxTimeout(u8),
    PreInitFailed,
    PinAccess(sysfs_gpio::Error),
    Serialise(serde_json::Error),
    Deserialise(serde_json::Error),
    Timeout,
    CanTxError((std::io::Error, u8)),
    MeterOffline,
    // FileAccess(_),
    // I2cWriteError,
}
impl std::error::Error for IndraError {}
impl std::fmt::Display for IndraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use IndraError::*;
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
            CanOpen(e) => write!(f, "Can bus open failed {e:?}"),
            TomlParse(e) => write!(f, "Toml parse failed {e:?}"),
            FileAccess(e) => write!(f, "File access failed {e:?}"),
            CanBusWrite(n, e) => write!(f, "can{n} send failed {e:?}"),
            CanBusWriteIo(n, e) => write!(f, "can{n} IO send failed {e:?}"),
            CanBusRxTimeout(n) => write!(f, "can{n} read timout "),
            PreInitFailed => write!(f, "PreInitFailed"),
            PinAccess(e) => write!(f, "GPIO error {e:?} "),
            Serialise(e) => write!(f, "json serialise {e:?} "),
            Deserialise(e) => write!(f, "json deserialise {e:?} "),
            Timeout => write!(f, "Timeout"),
            CanTxError((e, n)) => write!(f, "CanTxError #{n} {e:?}"),
            MeterOffline => write!(f, "Meter is offline"),
        }
    }
}
