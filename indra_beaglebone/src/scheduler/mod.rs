use crate::{
    error::IndraError,
    global_state::{ChargeParameters, OperationMode},
    log_error,
    statics::{ChademoTx, EventsRx},
    MAX_AMPS,
};
use chrono::{Local, NaiveTime};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::{fs, select, time::sleep};

const EVENT_FILE: &str = "events.json";

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
impl Into<OperationMode> for Event {
    fn into(self) -> OperationMode {
        use OperationMode::*;
        match self.action {
            Action::Charge => Charge(ChargeParameters::default()),
            Action::Discharge => Charge(ChargeParameters::default()),
            Action::Sleep => Idle,
            Action::V2h => V2h,
            Action::Eco => Charge(ChargeParameters::default().set_eco(true)),
        }
    }
}
impl Event {
    pub fn new(hh: u32, mm: u32, ss: u32) -> Self {
        let secs = hh * 3600 + mm * 60 + ss;
        let time = NaiveTime::from_num_seconds_from_midnight_opt(secs, 0).unwrap();
        Self {
            time: time,
            action: Action::Sleep,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Events(Vec<Event>);

impl Default for Events {
    fn default() -> Self {
        gen_config()
    }
}

fn gen_config() -> Events {
    let mut events: Vec<Event> = vec![];
    events.push(Event::new(1, 2, 3));
    events.push(Event::new(2, 3, 3));
    Events(events)
}

use tokio::task::JoinHandle;

pub async fn init(mut ev: EventsRx, mode_tx: ChademoTx) {
    // Load configuration from a TOML file
    let mut events: Events = get_eventfile()
        .await
        .try_into()
        .expect("Failed to deserialize events");
    // Sort earliest event first
    events.0.sort_by(|a, b| a.time.cmp(&b.time));
    log::info!("Loaded {:#?}", events);

    let mut handles: Vec<JoinHandle<()>> = Vec::new(); // Store spawned task handles

    loop {
        if let Some(mut new_events) = ev.recv().await {
            new_events.0.sort_by(|a, b| a.time.cmp(&b.time));
            if let Err(e) = update_eventfile(&new_events).await {
                log::error!("Events store fail {e:?}");
                continue;
            };

            // Cancel previous tasks
            for handle in handles.drain(..) {
                handle.abort(); // Abort the previous tasks
            }

            log::warn!("Canceled previous tasks");

            sleep(Duration::from_secs(1)).await;
            new_events.0.sort_by(|a, b| a.time.cmp(&b.time));
            log::info!("Spawning new scheduler: {new_events:?}");

            // Spawn new task
            let handle = tokio::spawn(process_events(new_events, mode_tx.clone()));
            log::info!("Spawned new scheduler {}", handle.id());
            handles.push(handle);
        }
    }
}

pub async fn get_eventfile() -> Events {
    let events_contents = match fs::read_to_string(EVENT_FILE).await {
        Ok(c) => c,
        _ => {
            // Generate a default events list
            let events = gen_config();
            match update_eventfile(&events).await {
                Ok(ef) => ef,
                Err(e) => panic!("{e:?}"),
            }
        }
    };

    let events: Events =
        serde_json::from_str(&events_contents).expect("Failed to parse events content as JSON");
    events
}
pub fn get_eventfile_sync() -> Result<Events, IndraError> {
    let events_contents = match std::fs::read_to_string(EVENT_FILE) {
        Ok(c) => c,
        _ => {
            // Generate a default events list
            let events = gen_config();
            match update_eventfile_sync(&events) {
                Ok(ef) => ef,
                Err(e) => panic!("{e:?}"),
            }
        }
    };

    let events: Events =
        serde_json::from_str(&events_contents).expect("Failed to parse events content as JSON");
    Ok(events)
}

async fn update_eventfile(events: &Events) -> Result<String, IndraError> {
    let json_data = serde_json::to_string(&events).map_err(|e| IndraError::Serialise(e))?;
    dbg!(&json_data);
    fs::write(EVENT_FILE, &json_data)
        .await
        .map_err(|e| IndraError::FileAccess(e))?;
    Ok(json_data)
}
fn update_eventfile_sync(events: &Events) -> Result<String, IndraError> {
    let json_data = serde_json::to_string(&events).map_err(|e| IndraError::Serialise(e))?;
    dbg!(&json_data);
    std::fs::write(EVENT_FILE, &json_data).map_err(|e| IndraError::FileAccess(e))?;
    Ok(json_data)
}

async fn process_events(events: Events, mode_tx: ChademoTx) {
    // mut rx_break: Receiver<bool>,
    loop {
        let mut todays_events = events.clone();
        let id = tokio::task::id().to_string();
        log::warn!("{id} New thread");
        if let Some(event) = todays_events.0.pop() {
            let current_time = chrono::Local::now().time();
            let next_event_time = event.time;
            if current_time > next_event_time {
                // skip old
                log::warn!("{id} Skipping over expired event {event:?}");
            } else if current_time <= next_event_time {
                let time_until_next_event = next_event_time - current_time;
                let sleep_duration =
                    Duration::from_secs(time_until_next_event.num_seconds() as u64);
                log::info!(
                    "{id} Waiting {sleep_duration:?} until next event ({:?} {:?})",
                    event.time,
                    event.action
                );
                sleep(sleep_duration).await;
                if let Err(e) = mode_tx.send(event.into()).await {
                    log::error!("{e:?}")
                };
            }
        }

        // If all events have been processed, reload events for the next day
        // and recursively call process_events
        let now = Local::now().naive_local();

        let sleep_duration = (now + chrono::Duration::days(1))
            .date()
            .and_hms_milli_opt(0, 0, 0, 0)
            .unwrap()
            .signed_duration_since(now)
            .to_std()
            .unwrap();

        log::info!(
            "Waiting for midnight for next event reload ({:?})",
            sleep_duration
        );

        sleep(sleep_duration).await;
    }
}
