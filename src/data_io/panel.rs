#![allow(dead_code)]
use std::time::Duration;

use crate::{
    chademo::state::{BOOSTPIN, ONOFFPIN},
    error::IndraError,
    log_error,
};
use embedded_hal::i2c::{I2c, Operation as I2cOperation};
use linux_embedded_hal::I2cdev;
const L1BS: u8 = 2 << 6;
const L2BS: u8 = 2 << 4;
const L3BS: u8 = 2 << 2;
const L4BS: u8 = 2;
const L1BF: u8 = 3 << 6;
const L2BF: u8 = 3 << 4;
const L3BF: u8 = 3 << 2;
const L4BF: u8 = 3;
const L1: u8 = 1 << 6;
const L2: u8 = 1 << 4;
const L3: u8 = 1 << 2;
const L4: u8 = 1;
const ONOFFLED: u8 = 1 << 6;
const BOOSTLED: u8 = 1 << 4;
const RED: u8 = 1 << 6;
const GREEN: u8 = 1 << 4;
const BLUE: u8 = 1 << 2;
const WHITE: u8 = 1;
const ALL_OFF: u8 = 0x55;
const ALL_ON: u8 = 0;
const PCS0: u8 = 0x2B; // slow
const PWM0: u8 = 0x80;
const PCS1: u8 = 0x0A; // fast
const PWM1: u8 = 0xC0;

const LOGO: u8 = 9;
const BUTTONS: u8 = 7;
const UPPERBAR: u8 = 6;
const LOWERBAR: u8 = 8;
const ADDR: u8 = 0x60;

/**
 *
 * Implement I2C
 * addr 0x60
 * write()
 */
use futures::future::join_all;
use futures::StreamExt;
use sysfs_gpio::{Direction, Edge, Pin};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    time::sleep,
};

pub enum ButtonTriggered {
    OnOff,
    Boost,
}
pub struct Buttons([Pin; 2]);

async fn monitor_pin(
    pin: Pin,
    state_sender: Sender<ButtonTriggered>,
) -> Result<(), sysfs_gpio::Error> {
    pin.export()?;
    pin.set_direction(Direction::In)?;
    pin.set_edge(Edge::FallingEdge)?;
    let mut gpio_events = pin.get_value_stream()?;
    while let Some(evt) = gpio_events.next().await {
        let val = evt.unwrap();
        match (pin.get_pin_num(), val) {
            (BOOSTPIN, 0) => {
                // send state update to toggle charge only
                log_error!(
                    "toggle charge button",
                    state_sender.send(ButtonTriggered::Boost).await
                );
            }
            (ONOFFPIN, 0) => {
                // send state update to toggle Stage 1 or shutdown only
                log_error!(
                    "toggle on/off",
                    state_sender.send(ButtonTriggered::OnOff).await
                );
            }
            _ => (),
        }
    }
    Ok(())
}

pub async fn stream_buttons(state_sender: Sender<ButtonTriggered>) {
    log::info!("Starting led_event_listener");
    let onoff = Pin::new(ONOFFPIN);
    let boost = Pin::new(BOOSTPIN);
    let buttons = Buttons { 0: [onoff, boost] };
    join_all(
        buttons
            .0
            .into_iter()
            .map(|pin| tokio::task::spawn(monitor_pin(pin, state_sender.clone()))),
    )
    .await;
}

pub async fn led_event_listener(mut state_recv: Receiver<Led>) -> Result<(), IndraError> {
    log::info!("Starting led_event_listener");
    let dev = I2cdev::new("/dev/i2c-2").expect("Cannot access /dev/i2c-2");
    let mut pca = Pca9552::new(dev);

    if let Err(e) = pca.init().await {
        log::error!("Init {e:?}")
    } else {
        log::info!("I2C ok")
    };
    tokio::spawn(async move {
        while let Some(event) = state_recv.recv().await {
            let result = match event {
                Led::Logo(colour) => pca.logo_led(colour),
                Led::Buttons(b) => match b {
                    ButtonTriggered::OnOff => pca.on_led_toggle(),
                    ButtonTriggered::Boost => pca.boost_led_toggle(),
                },
                Led::EnergyBar(val, discharging) => {
                    pca.upper_from_percentage_animated(val, discharging)
                }
                Led::SocBar(val) => pca.lower_from_percentage(val),
            };
            if let Err(e) = result {
                log::error!("{e:?}")
            }
        }
    });

    Ok(())
}

struct Pca9552<I2C> {
    i2c: I2C,
    on: bool,
    boost: bool,
    buttons: u8,
    upper: u8,
    lower: u8,
}

impl<I2C> Pca9552<I2C>
where
    I2C: I2c,
{
    pub fn new(i2c: I2C) -> Self {
        Pca9552 {
            i2c,
            buttons: ONOFFLED | BOOSTLED,
            on: true,
            boost: false,
            upper: ALL_OFF,
            lower: ALL_OFF,
        }
    }
    pub async fn init(&mut self) -> Result<&mut Self, I2C::Error> {
        sleep(Duration::from_millis(50)).await;
        self.write(&[PCS0])?;
        sleep(Duration::from_millis(50)).await;
        self.write(&[PWM0])?;
        sleep(Duration::from_millis(50)).await;
        self.write(&[PCS1])?;
        sleep(Duration::from_millis(50)).await;
        self.write(&[PWM1])?;
        sleep(Duration::from_millis(50)).await;
        self.write(&[LOGO, RED])?;
        sleep(Duration::from_millis(50)).await;
        self.write(&[BUTTONS, self.buttons])?;
        sleep(Duration::from_millis(50)).await;
        self.write(&[UPPERBAR, self.upper])?;
        sleep(Duration::from_millis(50)).await;
        self.write(&[LOWERBAR, self.lower])?;
        sleep(Duration::from_millis(50)).await;
        Ok(self)
    }

    fn write(&mut self, tx_buf: &[u8]) -> Result<u8, I2C::Error> {
        let mut rx_buf = [0, 0];
        let mut ops = [I2cOperation::Write(tx_buf), I2cOperation::Read(&mut rx_buf)];
        self.i2c.transaction(ADDR, &mut ops).and(Ok(rx_buf[0]))?;
        // .map_err(|_| IndraError::I2cWriteError)?;
        Ok(rx_buf[0])
    }
    pub fn logo_led(&mut self, colour: State) -> Result<&mut Self, I2C::Error> {
        self.write(&[LOGO, colour.into()])?;
        Ok(self)
    }
    pub fn on_led_toggle(&mut self) -> Result<&mut Self, I2C::Error> {
        self.on = !self.on;
        self.buttons = if self.on {
            self.buttons | ONOFFLED
        } else {
            self.buttons & !ONOFFLED
        };
        self.write(&[BUTTONS, self.buttons])?;
        Ok(self)
    }

    // If no activity for 5 minutes (something global), go dark
    pub async fn lights_out(&mut self) -> Result<&mut Self, I2C::Error> {
        sleep(Duration::from_millis(50)).await;
        self.write(&[BUTTONS, L4BS])?; // Should flash OnOff button
        sleep(Duration::from_millis(50)).await;
        self.write(&[UPPERBAR, ALL_OFF])?;
        sleep(Duration::from_millis(50)).await;
        self.write(&[LOWERBAR, ALL_OFF])?;
        sleep(Duration::from_millis(50)).await;
        Ok(self)
    }
    pub fn boost_led_toggle(&mut self) -> Result<&mut Self, I2C::Error> {
        self.boost = !self.boost;
        self.buttons = if self.boost {
            self.buttons | BOOSTLED
        } else {
            self.buttons & !BOOSTLED
        };
        self.write(&[BUTTONS, self.buttons])?;
        Ok(self)
    }
    pub fn lower_from_percentage_animated(&mut self, val: u8) -> Result<&mut Self, I2C::Error> {
        self.lower = mirror_bit_pairs(animated_bars(val));
        self.write(&[LOWERBAR, self.lower])?;
        Ok(self)
    }
    pub fn upper_from_percentage_animated(
        &mut self,
        val: u8,
        discharging: bool,
    ) -> Result<&mut Self, I2C::Error> {
        let led_val = animated_bars(val);
        self.upper = if !discharging {
            mirror_bit_pairs(led_val)
        } else {
            led_val
        };
        self.write(&[UPPERBAR, self.upper])?;
        Ok(self)
    }
    pub fn lower_from_percentage(&mut self, val: u8) -> Result<&mut Self, I2C::Error> {
        self.lower = mirror_bit_pairs(standard_bars(val));
        self.write(&[LOWERBAR, self.lower])?;
        Ok(self)
    }
    pub fn upper_from_percentage(
        &mut self,
        val: u8,
        discharging: bool,
    ) -> Result<&mut Self, I2C::Error> {
        let led_val = standard_bars(val);
        self.upper = if !discharging {
            mirror_bit_pairs(led_val)
        } else {
            led_val
        };
        self.write(&[UPPERBAR, self.upper])?;
        Ok(self)
    }
}

pub enum State {
    Error,
    Idle,
    Charging,
    V2h,
    Off,
}

impl Into<u8> for State {
    fn into(self) -> u8 {
        match self {
            State::Error => RED,
            State::Idle => WHITE,
            State::Charging => BLUE,
            State::V2h => GREEN,
            State::Off => ALL_ON, // off really
        }
    }
}

pub enum Led {
    Logo(State),
    Buttons(ButtonTriggered),
    EnergyBar(u8, bool),
    SocBar(u8),
}
fn animated_bars(val: u8) -> u8 {
    match val {
        0 => ALL_OFF,
        1..=25 => L1BF | L2 | L3 | L4,
        26..=50 => L2BF | L3 | L4,
        51..=75 => L3BF | L4,
        76..=99 => L4BF,
        _ => ALL_ON,
    }
}
fn standard_bars(val: u8) -> u8 {
    match val {
        0 => ALL_OFF,
        1..=25 => L1BS | L2 | L3 | L4,
        26..=50 => L2 | L3 | L4,
        51..=75 => L3 | L4,
        76..=99 => L4,
        _ => ALL_ON,
    }
}

fn mirror_bit_pairs(input: u8) -> u8 {
    let mut output = 0;
    for i in 0..4 {
        let bit_pair = ((input >> (2 * i)) & 0x03) << (6 - 2 * i);
        output |= bit_pair;
    }
    output
}
