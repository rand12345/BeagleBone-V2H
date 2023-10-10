use super::pwm::Pwm;
use crate::log_error;
use std::time::Duration;
use tokio::time::Instant;

// Setpoints
const FAN100: f32 = 70.0;
const FAN0: f32 = 50.0;

#[derive(Default, Copy, Clone, Debug)]
struct Duty {
    val: u8,
    duration: Option<Instant>,
}

impl Duty {
    pub fn new(val: u8) -> Duty {
        Duty {
            val,
            duration: Some(Instant::now()),
        }
    }
    /// Returns true if time > duration
    fn elapsed(&self, time: Duration) -> bool {
        match self.duration {
            None => true,
            Some(t) => t.elapsed().cmp(&time).is_gt(),
        }
    }
}
impl Into<u8> for Duty {
    fn into(self) -> u8 {
        self.val
    }
}

#[derive(Debug)]
pub struct Fan {
    duty: Duty,
    pwm: Pwm,
}

impl Fan {
    pub fn new(pwm: Pwm) -> Self {
        if pwm.export().is_err() {
            panic!("PWM EXPORT FAIL")
        }
        if pwm.enable(true).is_err() {
            if pwm.enable(false).is_err() {
                log::warn!("PWM ENABLE RETRY")
            } else if pwm.enable(true).is_err() {
                panic!("PWM ENABLE FAILED")
            }
        }
        Self {
            duty: Duty::default(),
            pwm,
        }
    }
    pub fn stop(&mut self) {
        let _ = self.pwm.set_duty(0);
        log_error!("PWM disable", self.pwm.enable(false));
    }
    pub fn update(&mut self, temp: f32) -> u8 {
        let elapsed = self.duty.elapsed(Duration::from_secs(20));
        let new_duty = Duty::new(temp_to_duty(temp));

        if self.duty.val != new_duty.val {
            if self.duty.val > new_duty.val && !elapsed {
                // falling -> overrun fan for 20 seconds
                return self.duty.val;
            }

            self.duty = new_duty; // pwm noise below 20%??
            if self.duty.val < 20 {
                self.stop()
            } else {
                log_error!("PWM enable", self.pwm.enable(true));
                if let Err(e) = self.pwm.set_duty(self.duty.val) {
                    log::error!("Duty update error {e}")
                }
            }
        };
        self.duty.val
    }
}

fn temp_to_duty(value: impl Into<f32>) -> u8 {
    let old_range = FAN100 - FAN0;
    let new_range = 100.0 - 0.1;
    let value: f32 = value.into();
    let duty = (((value - FAN0) * new_range / old_range) + 0.1) as u8;
    duty.min(100)
}
