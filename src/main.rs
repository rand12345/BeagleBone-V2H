// use canbus::mpsc_can;
use chademo::{
    ev_thread,
    state::{self, ChargerState},
};
use data_io::{config::APP_CONFIG, keyboard::scan_kb, meter, mqtt};
use pre_charger::{pre_commands::PreCmd, pre_thread};
use tokio::{
    signal::unix::{signal, SignalKind},
    sync::{mpsc, Barrier},
};
// use tokio_socketcan::CANFrame;

// mod canbus;
mod chademo;
mod data_io;
mod error;
mod macros;
mod pre_charger;

const MAX_SOC: u8 = 100;
const MIN_SOC: u8 = 30;
const MAX_AMPS: u8 = 15;
const METER_BIAS: f32 = -0.1;

#[tokio::main]
async fn main() -> Result<(), &'static str> {
    simple_logger::init_with_level(log::Level::Debug).expect("Logger init failed");
    // let (tx0sender, tx0reciever) = mpsc::channel::<CANFrame>(10);
    // let (rx0sender, rx0receiver) = mpsc::channel::<CANFrame>(10);
    let (precmd_sender, precmd_receiver) = mpsc::channel::<PreCmd>(100);
    let (gpiocmd_sender, gpiocmd_receiver) = mpsc::channel::<ChargerState>(100);

    let pre_init_barrier = std::sync::Arc::new(Barrier::new(2));
    let gpiocmd_sender1 = gpiocmd_sender.clone();
    let gpiocmd_sender2 = gpiocmd_sender.clone();
    let precmd_sender1 = precmd_sender.clone();

    let app_config = &APP_CONFIG.clone();

    let mut ctrl_c =
        signal(SignalKind::interrupt()).expect("Failed to create Ctrl-C signal handler");

    tokio::spawn(async move {
        ctrl_c.recv().await;
        if gpiocmd_sender1.is_closed() {
            eprintln!("Exit before channel opened");
            std::process::exit(0)
        }
        if let Err(e) = gpiocmd_sender1.send(ChargerState::Exiting).await {
            eprintln!("{e:?}")
        };
        println!(" Ctrl-C captured, exiting.");
        if let Err(e) = gpiocmd_sender1.send(ChargerState::Exiting).await {
            eprintln!("{e:?}")
        };
    });

    tokio::spawn(state::init_state(gpiocmd_receiver));
    // tokio::spawn(mpsc_can::can_start("can0", rx0sender, tx0reciever));
    tokio::spawn(meter::meter(app_config.meter.clone())); // rtu over tcp SDM230 modbus meter

    tokio::spawn(pre_thread::init_pre(
        "CAN0",
        pre_init_barrier.clone(),
        // tx0sender,
        // rx0receiver,
        precmd_receiver,
    ));
    pre_init_barrier.wait().await; // Halts progress until Pre charger initalised

    tokio::spawn(ev_thread::ev100ms(
        precmd_sender.clone(),
        gpiocmd_sender.clone(),
    ));
    tokio::spawn(mqtt::mqtt_task(app_config.mqtt.clone()));

    // temp keyboard interface (never returns)

    scan_kb(&precmd_sender1, &gpiocmd_sender2).await;

    Ok(())
}
