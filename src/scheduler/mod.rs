use chrono::{DateTime, Local, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use std::{borrow::BorrowMut, fs, time::Duration};
use tokio::{select, sync::watch::Receiver, time::sleep};

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
struct Action {
    charge: bool,
    sleep: bool,
    eco: bool,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
struct Event {
    time: NaiveTime,
    action: Action,
    // You can add other fields here as needed
}
impl Event {
    pub fn new(hh: u32, mm: u32, ss: u32) -> Self {
        let secs = hh * 60 + mm * 60 + ss;
        let time = NaiveTime::from_num_seconds_from_midnight_opt(secs, 0).unwrap();
        Self {
            time: time,
            action: Action {
                charge: false,
                sleep: false,
                eco: false,
            },
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Events(Vec<Event>);

fn gen_config() -> Events {
    let mut events: Vec<Event> = vec![];
    events.push(Event::new(1, 2, 3));
    events.push(Event::new(2, 3, 3));
    Events(events)
}

pub async fn init() {
    // Load configuration from a TOML file
    let mut events: Events = get_config()
        .try_into()
        .expect("Failed to deserialize events");
    events.0.sort_by(|a, b| a.time.cmp(&b.time));
    log::info!("Loaded {:#?}", events);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Events>(1);
    let (tx_break, rx_break) = tokio::sync::watch::channel::<bool>(false);
    tokio::spawn(async move {
        let rx_b = rx_break.clone();
        loop {
            // 24/7 loop
            // do events and also wait for reload signal

            if let Some(new_events) = rx.recv().await {
                log::info!("Found new scheduled events: {new_events:?}");
                process_events(new_events, rx_b.clone()).await;
            };
        }
    });
    // send inital data
    let _ = tx.send(events).await;
    // listen to event loop
    // if reload config happens send tx_break(true)
    loop {
        // if let Some(Config(new_events)) = eventloop.recv().await {
        // let _ = tx.send(new_events).await;
        // tx_break.send(true);
    }
}

fn get_config() -> toml::Value {
    let config_contents = match fs::read_to_string("events.toml") {
        Ok(c) => c,
        _ => {
            // Generate a default events list
            let config = gen_config();
            let toml_string = toml::to_string_pretty(&config).unwrap();
            fs::write("events.toml", &toml_string).unwrap();
            toml_string
        }
    };

    let config: toml::Value =
        toml::from_str(&config_contents).expect("Failed to parse configuration as TOML");
    config
}

async fn process_events(events: Events, mut rx_break: Receiver<bool>) {
    loop {
        for event in events.0.iter() {
            let current_time = chrono::Local::now().time();
            let next_event_time = event.time;
            if current_time > next_event_time {
                // skip old
                log::warn!("Skipping over expired event {event:?}");
                continue;
            }
            if current_time <= next_event_time {
                let time_until_next_event = next_event_time - current_time;
                let sleep_duration =
                    Duration::from_secs(time_until_next_event.num_seconds() as u64);
                log::info!("Waiting {sleep_duration:?} for next event {event:?}");

                // break issued on schedule change
                select! {
                    _ = sleep(sleep_duration) => (),
                    _ = rx_break.changed() => return
                }

                // let event = events.0.remove(0);
                println!("Event due: {:#?}", event);
            }
        }
        // If all events have been processed, reload events for the next day
        // and recursively call process_events
        let now = Local::now().naive_local();

        let duration = (now + chrono::Duration::days(1))
            .date()
            .and_hms_milli_opt(0, 0, 0, 0)
            .unwrap()
            .signed_duration_since(now)
            .to_std()
            .unwrap();

        // break issued on schedule change
        select! {
            _ = sleep(duration) => (),
            _ = rx_break.changed() => return
        } // sleep until midnight
    }
}
