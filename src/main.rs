use chademo::{
    ev_thread,
    state::{self, ChargerState},
};
use data_io::{config::APP_CONFIG, keyboard::scan_kb, meter, mqtt};
use pre_charger::{pre_commands::PreCmd, pre_thread};
use tokio::{
    signal::unix::{signal, SignalKind},
    sync::{mpsc, Barrier},
    // time::sleep,
};

mod api;
mod chademo;
mod data_io;
mod error;
mod macros;
mod pre_charger;

const MAX_SOC: u8 = 100;
const MIN_SOC: u8 = 30;
const MAX_AMPS: u8 = 15;
const METER_BIAS: f32 = -0.1;

/**
 *
 * Todo:
 *      API
 *          Add GetParams and return error with message (add to JS)
 *          Access config (write to disk on save)
 *          JS not updating DOM ???
 *
 *      Scheduler (new)
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
    simple_logger::init_with_level(log::Level::Debug).expect("Logger init failed");

    // temp whilst testing
    let _pca9552_reset = state::pin_init_out_high(state::RESETPCAPIN).unwrap();
    let _masterpin = state::pin_init_out_high(state::MASTERCONTACTOR).unwrap();
    let _pre_ac_contactor = state::pin_init_out_high(state::PREACPIN).unwrap();

    let (precmd_sender, precmd_receiver) = mpsc::channel::<PreCmd>(100);
    let (gpiocmd_sender, gpiocmd_receiver) = mpsc::channel::<ChargerState>(100);
    let (but_sender, but_receiver) = mpsc::channel::<crate::data_io::panel::ButtonTriggered>(10);
    let (led_sender, led_receiver) = mpsc::channel::<crate::data_io::panel::Led>(10);

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

    tokio::spawn(crate::data_io::panel::stream_buttons(but_sender));
    tokio::spawn(crate::data_io::panel::led_event_listener(led_receiver));

    // let pwm = pre_charger::pwm::Pwm::new(0, 0, 1000).unwrap(); // number depends on chip, etc.
    // let mut fan = pre_charger::fans::Fan::new(pwm);

    // loop {
    //     // fan.update(40.0);
    //     sleep(std::time::Duration::from_secs(1)).await;
    //     log::info!("sleep");
    //     if let Err(e) = led_sender
    //         .send(data_io::panel::Led::Logo(data_io::panel::State::Charging))
    //         .await
    //     {
    //         log::error!("{e:?}")
    //     };
    //     log::warn!("{fan:?}");
    //     fan.update(50.0);
    //     sleep(std::time::Duration::from_secs(1)).await;
    //     let _ = led_sender
    //         .send(data_io::panel::Led::Logo(data_io::panel::State::Error))
    //         .await;
    // }

    tokio::spawn(state::init_state(gpiocmd_receiver, led_sender.clone()));
    tokio::spawn(meter::meter(app_config.meter.clone())); // rtu over tcp SDM230 modbus meter

    tokio::spawn(pre_thread::pre_thread(
        "CAN0",
        pre_init_barrier.clone(),
        precmd_receiver,
    ));
    pre_init_barrier.wait().await; // Halts progress until Pre charger initalised

    tokio::spawn(ev_thread::ev100ms(
        precmd_sender.clone(),
        gpiocmd_sender.clone(),
        but_receiver,
        led_sender.clone(),
    ));
    tokio::spawn(mqtt::mqtt_task(app_config.mqtt.clone()));
    tokio::spawn(api::run());

    // temp keyboard interface (never returns)

    scan_kb(&precmd_sender1, &gpiocmd_sender2).await;

    Ok(())
}
