// Copyright 2016, Paul Osborne <osbpau@gmail.com>
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/license/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option.  This file may not be copied, modified, or distributed
// except according to those terms.
//
// Portions of this implementation are based on work by Nat Pryce:
// https://github.com/npryce/rusty-pi/blob/master/src/pi/gpio.rs

// #![crate_type = "lib"]
// #![crate_name = "sysfs_pwm"]

//! PWM access under Linux using the PWM sysfs interface
//!
//! Modified for Debian Buster /sys
//! Added duty_cycle() and fixed frequency in new() for simplicity

use std::fs::{self, File, OpenOptions};
use std::io::prelude::*;
use std::str::FromStr;

pub use error::Error;

#[derive(Debug)]
pub struct PwmChip {
    pub pwm_id: u32,
}

#[derive(Debug)]
pub struct Pwm {
    chip: PwmChip,
    channel: u32,
    period: u32,
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum Polarity {
    Normal,
    Inverse,
}

pub type Result<T> = ::std::result::Result<T, error::Error>;

/// Open the specified entry name as a writable file
fn pwm_file_wo(chip: &PwmChip, channel: u32, name: &str) -> Result<File> {
    // /sys/class/pwm/pwmchip7/pwm-7:0
    let f = (OpenOptions::new().write(true).open(format!(
        "/sys/class/pwm/pwmchip{}/pwm-{}:{}/{}",
        chip.pwm_id, chip.pwm_id, channel, name
    )))?;
    Ok(f)
}

/// Open the specified entry name as a readable file
#[allow(dead_code)]
fn pwm_file_ro(chip: &PwmChip, channel: u32, name: &str) -> Result<File> {
    let f = File::open(format!(
        "/sys/class/pwm/pwmchip{}/pwm-{}:{}/{}",
        chip.pwm_id, chip.pwm_id, channel, name
    ))?;
    Ok(f)
}

/// Get the u32 value from the given entry
#[allow(dead_code)]
fn pwm_file_parse<T: FromStr>(chip: &PwmChip, channel: u32, name: &str) -> Result<T> {
    let mut s = String::with_capacity(10);
    let mut f = pwm_file_ro(chip, channel, name)?;
    f.read_to_string(&mut s)?;
    match s.trim().parse::<T>() {
        Ok(r) => Ok(r),
        Err(_) => Err(Error::Unexpected(format!(
            "Unexpected value in file contents: {:?}",
            s
        ))),
    }
}

impl PwmChip {
    pub fn new(number: u32) -> Result<PwmChip> {
        fs::metadata(&format!("/sys/class/pwm/pwmchip{}", number))?;
        Ok(PwmChip { pwm_id: number })
    }

    #[allow(dead_code)]
    pub fn count(&self) -> Result<u32> {
        let npwm_path = format!("/sys/class/pwm/pwmchip{}/npwm", self.pwm_id);
        let mut npwm_file = File::open(&npwm_path)?;
        let mut s = String::new();
        npwm_file.read_to_string(&mut s)?;
        match s.parse::<u32>() {
            Ok(n) => Ok(n),
            Err(_) => Err(Error::Unexpected(format!(
                "Unexpected npwm contents: {:?}",
                s
            ))),
        }
    }

    pub fn export(&self, channel: u32) -> Result<()> {
        // only export if not already exported
        if let Err(_) = fs::metadata(&format!(
            "/sys/class/pwm/pwmchip{}/pwm-{}:{}",
            self.pwm_id, self.pwm_id, channel
        )) {
            let path = format!("/sys/class/pwm/pwmchip{}/export", self.pwm_id);
            let mut export_file = File::create(&path)?;
            let _ = export_file.write_all(format!("{}", channel).as_bytes());
        }
        Ok(())
    }

    pub fn unexport(&self, channel: u32) -> Result<()> {
        if let Ok(_) = fs::metadata(&format!(
            "/sys/class/pwm/pwmchip{}/pwm-{}:{}",
            self.pwm_id, self.pwm_id, channel
        )) {
            let path = format!("/sys/class/pwm/pwmchip{}/unexport", self.pwm_id);
            let mut export_file = File::create(&path)?;
            let _ = export_file.write_all(format!("{}", channel).as_bytes());
        }
        Ok(())
    }
}

impl Pwm {
    /// Create a new Pwm wiht the provided chip/number
    ///
    /// This function does not export the Pwm pin
    pub fn new(chip: u32, channel: u32, freqency: u32) -> Result<Pwm> {
        let chip: PwmChip = PwmChip::new(chip)?;
        assert!(freqency < 10_000);

        let period = 10u64.pow(10) / freqency as u64;
        Ok(Pwm {
            chip: chip,
            channel: channel,
            period: period as u32,
        })
    }

    /// Run a closure with the GPIO exported
    #[allow(dead_code)]
    #[inline]
    pub fn with_exported<F>(&self, closure: F) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        self.export()?;
        match closure() {
            Ok(()) => self.unexport(),
            Err(_) => self.unexport(),
        }
    }

    /// Export the Pwm for use
    pub fn export(&self) -> Result<()> {
        self.chip.export(self.channel)
    }

    /// Unexport the PWM
    pub fn unexport(&self) -> Result<()> {
        self.chip.unexport(self.channel)
    }

    /// Enable/Disable the PWM Signal
    pub fn enable(&self, enable: bool) -> Result<()> {
        self.set_period_ns()?;
        let mut enable_file = pwm_file_wo(&self.chip, self.channel, "enable")?;
        let contents = if enable { "1" } else { "0" };
        enable_file.write_all(contents.as_bytes())?;
        Ok(())
    }

    pub fn set_duty(&self, percentage: u8) -> Result<()> {
        assert!(percentage.le(&100));
        let duty_cycle_ns = (self.period as f32 * (percentage as f32 * 0.01)) as u32;
        self.set_duty_cycle_ns(duty_cycle_ns)
    }

    /// Get the currently configured duty_cycle in nanoseconds
    // pub fn get_duty_cycle_ns(&self) -> Result<u32> {
    //     pwm_file_parse::<u32>(&self.chip, self.channel, "duty_cycle")
    // }

    /// The active time of the PWM signal
    ///
    /// Value is in nanoseconds and must be less than the period.
    fn set_duty_cycle_ns(&self, duty_cycle_ns: u32) -> Result<()> {
        // we'll just let the kernel do the validation
        let mut duty_cycle_file = pwm_file_wo(&self.chip, self.channel, "duty_cycle")?;
        duty_cycle_file.write_all(format!("{}", duty_cycle_ns).as_bytes())?;
        Ok(())
    }

    /// Get the currently configured period in nanoseconds
    // pub fn get_period_ns(&self) -> Result<u32> {
    //     pwm_file_parse::<u32>(&self.chip, self.channel, "period")
    // }

    /// The period of the PWM signal in Nanoseconds
    fn set_period_ns(&self) -> Result<()> {
        let mut period_file = pwm_file_wo(&self.chip, self.channel, "period")?;
        period_file.write_all(format!("{}", self.period).as_bytes())?;
        Ok(())
    }
}

mod error {

    use std::convert;
    use std::fmt;
    use std::io;

    #[derive(Debug)]
    pub enum Error {
        /// Simple IO error
        Io(io::Error),
        /// Read unusual data from sysfs file.
        Unexpected(String),
    }

    impl ::std::error::Error for Error {
        fn cause(&self) -> Option<&dyn (::std::error::Error)> {
            match *self {
                Error::Io(ref e) => Some(e),
                _ => None,
            }
        }
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            match *self {
                Error::Io(ref e) => e.fmt(f),
                Error::Unexpected(ref s) => write!(f, "Unexpected: {}", s),
            }
        }
    }

    impl convert::From<io::Error> for Error {
        fn from(e: io::Error) -> Error {
            Error::Io(e)
        }
    }
}
