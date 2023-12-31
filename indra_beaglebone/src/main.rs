// #![allow(dead_code)]
#![allow(unused_imports)]
#![feature(async_closure)]
use chademo::{
    ev_connect,
    state::{self},
};
use data_io::{config::APP_CONFIG, db::Database, meter, mqtt, panel};
use global_state::OperationMode;
use statics::OPERATIONAL_MODE;
use tokio::{
    signal::unix::{signal, SignalKind},
    sync::OnceCell,
};

mod api;
mod chademo;
mod data_io;
mod error;
mod global_state;
mod macros;
mod pre_charger;
mod scheduler;

const MAX_SOC: u8 = 90;
const MIN_SOC: u8 = 31;
const MAX_AMPS: u8 = 16;
const METER_BIAS: f32 = 0.0;

static POOL: OnceCell<Database> = OnceCell::const_new();

/**
 *
 * Todo:
 *
 *      API
 *          Add GetParams and return error with message (add to JS)
 *          Access config (write to disk on save) (server done)
 *          
 *
 *      Config
 *          Min/max V2H SoC - web ui
 *
 *      eStop
 *          Detect input pin?
 *
 */
#[tokio::main]
async fn main() -> Result<(), &'static str> {
    #[cfg(feature = "tracing")]
    console_subscriber::ConsoleLayer::builder()
        .retention(Duration::from_secs(60))
        .server_addr(([0, 0, 0, 0], 5556))
        .init();

    #[cfg(feature = "logging-verbose")]
    simple_logger::init_with_level(log::Level::Trace).expect("Logger init failed");
    #[cfg(not(feature = "logging-verbose"))]
    simple_logger::init_with_level(log::Level::Debug).expect("Logger init failed");

    POOL.get_or_try_init(|| async { Database::new().await })
        .await
        .expect("SQLx error");

    let (led_tx, led_rx) = statics::led_channel();
    let (mode_tx, mode_rx) = statics::chademo_channel();
    let (events_tx, events_rx) = statics::events_channel();

    let app_config = &APP_CONFIG.clone();

    let _pca9552_reset = state::pin_init_out_high(state::RESETPCAPIN).unwrap();
    let _master = state::pin_init_out_high(state::MASTERCONTACTOR).unwrap();

    tokio::spawn(meter::meter(app_config.meter.clone())); // rtu over tcp SDM230 modbus meter
    tokio::spawn(panel::panel_event_listener(led_rx, mode_tx.clone()));
    tokio::spawn(scheduler::init(events_rx, mode_tx.clone()));
    tokio::spawn(api::run(events_tx, mode_tx.clone()));
    tokio::spawn(data_io::db::init(10_000));
    tokio::spawn(mqtt::mqtt_task(app_config.mqtt.clone()));
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let mut ctrl_c =
        signal(SignalKind::interrupt()).expect("Failed to create Ctrl-C signal handler");
    let eb = mode_tx.clone();
    tokio::spawn(async move {
        loop {
            ctrl_c.recv().await;
            log::warn!("CTRL-C caught - sending Quit instruction");
            let _ = eb.send(OperationMode::Quit).await;
            ctrl_c.recv().await;
            log::warn!("CTRL-C caught again - forcing exit");
            std::process::exit(1)
        }
    });

    // Final loop
    ev_connect::ev100ms(led_tx, mode_rx)
        .await
        .map_err(|_| &*"ev100ms thread died")
}

pub mod statics {
    use std::sync::Arc;

    use tokio::sync::{mpsc, Mutex};
    use tokio_socketcan::CANFrame;

    use crate::{
        chademo::state::Chademo, //ChargerState,State
        data_io::{db::ChademoDbRow, panel::LedCommand},
        global_state::OperationMode,
        pre_charger::PreCommand,
        scheduler::Events,
    };

    lazy_static::lazy_static! {
        // pub static ref STATE: Arc<Mutex<State>> = Arc::new(Mutex::new(State(ChargerState::Idle)));
        pub static ref CHADEMO: Arc<Mutex<Chademo>> = Arc::new(Mutex::new(Chademo::new()));
        pub static ref OPERATIONAL_MODE: Arc<Mutex<OperationMode>> =
            Arc::new(Mutex::new(OperationMode::default()));
    }

    pub type Channel<T> = (mpsc::Sender<T>, mpsc::Receiver<T>);
    pub type PreRx = mpsc::Receiver<PreCommand>;
    pub type PreTx = mpsc::Sender<PreCommand>;
    pub type PreChannel = Channel<PreCommand>;
    pub type ChademoRx = mpsc::Receiver<OperationMode>;
    pub type ChademoTx = mpsc::Sender<OperationMode>;
    pub type ChademoChannel = Channel<OperationMode>;
    pub type LedChannel = Channel<LedCommand>;
    pub type LedRx = mpsc::Receiver<LedCommand>;
    pub type LedTx = mpsc::Sender<LedCommand>;
    pub type EventsRx = mpsc::Receiver<Events>;
    pub type EventsTx = mpsc::Sender<Events>;
    pub type EventsChannel = Channel<Events>;

    pub type PreRxMutex = Arc<Mutex<PreRx>>;

    pub fn chademo_channel() -> ChademoChannel {
        mpsc::channel::<OperationMode>(100)
    }
    pub fn pre_channel() -> PreChannel {
        mpsc::channel::<PreCommand>(100)
    }
    pub fn led_channel() -> LedChannel {
        mpsc::channel::<LedCommand>(100)
    }
    pub fn events_channel() -> EventsChannel {
        mpsc::channel::<Events>(100)
    }

    // pub fn mpsc_channel<T>(buf: usize) -> Channel<T> {
    //     mpsc::channel::<T>(buf)
    // }

    pub fn mutex<T>(i: T) -> Arc<Mutex<T>> {
        Arc::new(Mutex::new(i))
    }
}
