// #![allow(dead_code)]
#![allow(unused_imports)]
use chademo::{
    ev_connect,
    state::{self},
};
use data_io::{config::APP_CONFIG, meter, mqtt, panel};
use global_state::OperationMode;
use statics::OPERATIONAL_MODE;
use tokio::signal::unix::{signal, SignalKind};

mod api;
mod chademo;
mod data_io;
mod error;
mod global_state;
mod macros;
mod pre_charger;
mod scheduler;

const MAX_SOC: u8 = 100;
const MIN_SOC: u8 = 30;
const MAX_AMPS: u8 = 16;
const METER_BIAS: f32 = 0.0;

/**
 *
 * Todo:
 *      CHAdeMO:
 *          Connection timeout -> returns to Idle
 *
 *      API
 *          Add GetParams and return error with message (add to JS)
 *          Access config (write to disk on save) (server done)
 *          JS not updating DOM ???
 *
 *      Scheduler (new)
 *          TOML done
 *
 *        
 *          Sched charge from time window - web ui
 *          TZ aware!
 *          Charge to SoC optional limiter
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
        // set how long the console will retain data from completed tasks
        .retention(Duration::from_secs(60))
        // set the address the server is bound to
        .server_addr(([0, 0, 0, 0], 5556))
        // ... other configurations ...
        .init();

    #[cfg(feature = "logging-verbose")]
    simple_logger::init_with_level(log::Level::Trace).expect("Logger init failed");
    #[cfg(not(feature = "logging-verbose"))]
    simple_logger::init_with_level(log::Level::Debug).expect("Logger init failed");

    let (led_tx, led_rx) = statics::led_channel();
    let (mode_tx, mode_rx) = statics::chademo_channel();
    let (events_tx, events_rx) = statics::events_channel();

    let app_config = &APP_CONFIG.clone();
    tokio::spawn(meter::meter(app_config.meter.clone())); // rtu over tcp SDM230 modbus meter

    let _pca9552_reset = state::pin_init_out_high(state::RESETPCAPIN).unwrap();
    let _master = state::pin_init_out_high(state::MASTERCONTACTOR).unwrap();

    let mut ctrl_c =
        signal(SignalKind::interrupt()).expect("Failed to create Ctrl-C signal handler");

    // let eb = chademo_tx.clone();
    tokio::spawn(async move {
        ctrl_c.recv().await;
        *OPERATIONAL_MODE.clone().lock().await = OperationMode::Quit;
    });
    tokio::spawn(panel::panel_event_listener(led_rx, mode_tx.clone()));
    tokio::spawn(scheduler::init(events_rx));
    tokio::spawn(api::run(events_tx, mode_tx.clone()));
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    tokio::spawn(mqtt::mqtt_task(app_config.mqtt.clone()));
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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
        data_io::panel::LedCommand,
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
    pub type CanSender = mpsc::Receiver<CANFrame>;
    pub type CanSend = mpsc::Sender<CANFrame>;
    pub type CanChannel = Channel<CANFrame>;
    pub type ChademoRx = mpsc::Receiver<OperationMode>;
    pub type ChademoTx = mpsc::Sender<OperationMode>;
    pub type ChademoChannel = Channel<OperationMode>;
    pub type LedChannel = Channel<LedCommand>;
    pub type LedRx = mpsc::Receiver<LedCommand>;
    pub type LedTx = mpsc::Sender<LedCommand>;
    pub type PreRxMutex = Arc<Mutex<PreRx>>;
    pub type EventsRx = mpsc::Receiver<Events>;
    pub type EventsTx = mpsc::Sender<Events>;
    pub type EventsChannel = Channel<Events>;

    pub fn can_channel() -> CanChannel {
        mpsc::channel::<CANFrame>(100)
    }
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

    pub fn mutex<T>(i: T) -> Arc<Mutex<T>> {
        Arc::new(Mutex::new(i))
    }
    // pub fn button_channel() -> ButtonChannel {
    //     mpsc::channel::<ButtonTriggered>(10)
    // }
}

// mod app {
//     use crate::chademo::ev_thread;

//     pub async fn run(eb: super::statics::ButtonChannel) {
//         let mut eb_rx = eb.1;

//         loop {
//             match eb_rx.recv().await {
//                 Some(v) => match v {
//                     crate::data_io::panel::ButtonTriggered::OnOff => todo!(),
//                     crate::data_io::panel::ButtonTriggered::Boost => todo!(),
//                 },
//                 None => todo!(),
//             };
//             break;
//             // Waking up
//         }
//     }
// }
