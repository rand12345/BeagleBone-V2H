use crate::chademo::state::{Chademo, ChargerState};
use crate::error::PreError;
use crate::log_error;
use crate::pre_charger::pre_thread::PreCharger;
use lazy_static::lazy_static;
use log::info;
use serde::{Deserialize, Serialize};
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

use super::config::MqttConfig;

lazy_static! {
    pub static ref CHADEMO_DATA: Arc<Mutex<MqttChademo>> =
        Arc::new(Mutex::new(MqttChademo::default()));
}

#[derive(Clone, Copy, Serialize, Deserialize, Default, Debug)]
pub struct MqttChademo {
    pub ac_w: f32,
    pub soc: f32,
    pub volts: f32,
    pub temp: f32,
    pub amps: f32,
    pub state: ChargerState,
    pub requested_amps: f32,
}

impl MqttChademo {
    pub fn from_pre(&mut self, pre: PreCharger) -> &mut Self {
        self.ac_w = pre.ac_power();
        self.temp = pre.temp();
        self.volts = pre.get_dc_output_volts();
        self.amps = pre.get_dc_output_amps();
        self
    }
    pub fn from_chademo(&mut self, chademo: Chademo) -> &mut Self {
        self.soc = chademo.soc().into();
        self.state = chademo.state();
        self.requested_amps = chademo.requested_amps();
        self
    }
}

pub async fn mqtt_task(config: MqttConfig) -> Result<(), PreError> {
    use rumqttc::{AsyncClient, MqttOptions, QoS};

    log::info!("Starting MQTT thread");
    if !config.enabled {
        log::warn!("MQTT not enabled in config");
        return Ok(());
    }

    let mut mqttoptions = MqttOptions::new(config.client_id, config.host, config.port);
    mqttoptions.set_keep_alive(Duration::from_secs(5));
    mqttoptions.set_credentials(config.username, config.password);
    mqttoptions.set_transport(rumqttc::Transport::Tcp);
    mqttoptions.set_clean_session(true);
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 2);
    tokio::spawn(async move {
        loop {
            if let Ok(mqtt_event) = eventloop.poll().await {
                if let ControlFlow::Break(_) = handle_mqtt_event(mqtt_event).await {
                    continue;
                }
            };
        }
    });

    client
        .subscribe(&config.sub, QoS::AtLeastOnce)
        .await
        .map_err(|e| PreError::MqttSub(e))?;
    let interval = config.interval;
    loop {
        sleep(Duration::from_secs(interval.into())).await;

        // send basic data as json string
        let msg = match serde_json::to_string(&*CHADEMO_DATA.lock().await) {
            Ok(d) => d,
            Err(e) => {
                log::error!("CHAdeMO Ser {e}");
                continue;
            }
        };
        let topic = config.topic.clone();
        info!("Sending {}={msg}", &topic);

        // spawn to avoid latency spikes
        let client_send = client.clone();
        tokio::task::spawn(async move {
            log_error!(
                "MQTT SEND",
                client_send
                    .publish(topic, QoS::AtLeastOnce, true, msg)
                    .await
                    .map_err(|e| PreError::MqttSend(e))
            );
        });
    }
}

async fn handle_mqtt_event(mqtt_event: rumqttc::Event) -> ControlFlow<()> {
    use rumqttc::Event::*;
    match mqtt_event {
        Incoming(mqtt_in) => {
            // use rumqttc::Packet::*;
            log::debug!("Incoming {:?}", mqtt_in);
            // if let Publish(msg) = mqtt_in {
            //     *CARDATA.lock().await = match serde_json::from_slice::<Data>(&msg.payload) {
            //         Ok(json) => json.inner,
            //         Err(e) => {
            //             log::error!("{e:?}");
            //             return ControlFlow::Break(());
            //         }
            //     };
            // }
        }
        Outgoing(mqtt_out) => {
            log::debug!("Outgoing {:?}", mqtt_out);
        }
    }
    ControlFlow::Continue(())
}
