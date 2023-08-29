use crate::{error::IndraError, statics::EventsRx};
use chrono::{Local, NaiveTime};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::{fs, select, sync::watch::Receiver, time::sleep};

const EVENT_FILE: &str = "events.toml";

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone, Copy)]
pub enum Action {
    Charge,
    Discharge,
    Sleep,
    V2h,
    Eco,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone, Copy)]
pub struct Event {
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
            action: Action::Sleep,
        }
    }
    pub fn to_eb_message(&self) -> ! {
        match self.action {
            Action::Charge => todo!(),
            Action::Discharge => todo!(),
            Action::Sleep => todo!(),
            Action::V2h => todo!(),
            Action::Eco => todo!(),
        };
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Events(Vec<Event>);

fn gen_config() -> Events {
    let mut events: Vec<Event> = vec![];
    events.push(Event::new(1, 2, 3));
    events.push(Event::new(2, 3, 3));
    Events(events)
}

pub async fn init(mut ev: EventsRx) {
    // Load configuration from a TOML file
    let mut events: Events = get_eventfile()
        .await
        .try_into()
        .expect("Failed to deserialize events");
    // Sort earliest event first
    events.0.sort_by(|a, b| a.time.cmp(&b.time));
    log::info!("Loaded {:#?}", events);
    let (tx_break, rx_break) = tokio::sync::watch::channel::<bool>(false);
    // listen to event loop
    // if reload events happens send tx_break() to kill spawned thread
    loop {
        if let Some(mut new_events) = ev.recv().await {
            // match get_eventfile().await.try_into::<Events>() {
            // Ok(mut new_events) => {
            let _ = tx_break.send(true); // if this is not received, try toggling bool
            new_events.0.sort_by(|a, b| a.time.cmp(&b.time));
            log::info!("Spawning new scheduler: {new_events:?}");
            tokio::spawn(process_events(new_events, rx_break.clone()));
        }
    }
}

pub async fn get_eventfile() -> toml::Value {
    let events_contents = match fs::read_to_string(EVENT_FILE).await {
        Ok(c) => c,
        _ => {
            // Generate a default events list
            let events = gen_config();
            update_eventfile(&events)
                .await
                .expect("Default events error")
        }
    };

    let events: toml::Value =
        toml::from_str(&events_contents).expect("Failed to parse events content as TOML");
    events
}
pub fn get_eventfile_sync() -> Option<String> {
    std::fs::read_to_string(EVENT_FILE).ok()
}

async fn update_eventfile(events: &Events) -> Result<String, IndraError> {
    let toml_string = toml::to_string_pretty(&events).map_err(|e| IndraError::TomlParse(e))?;
    fs::write(EVENT_FILE, &toml_string)
        .await
        .map_err(|e| IndraError::FileAccess(e))?;
    Ok(toml_string)
}

async fn process_events(events: Events, mut rx_break: Receiver<bool>) {
    loop {
        let mut todays_events = events.clone();

        if let Some(event) = todays_events.0.pop() {
            let current_time = chrono::Local::now().time();
            let next_event_time = event.time;
            if current_time > next_event_time {
                // skip old
                log::warn!("Skipping over expired event {event:?}");
            } else if current_time <= next_event_time {
                let time_until_next_event = next_event_time - current_time;
                let sleep_duration =
                    Duration::from_secs(time_until_next_event.num_seconds() as u64);
                log::info!("Waiting {sleep_duration:?} for next event {event:?}");

                // break issued on schedule change
                select! {
                    _ = sleep(sleep_duration) => (),
                    _ = rx_break.changed() => return
                }

                // let event = todays_events.0.pop();
                // eb.send(..some event)
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
