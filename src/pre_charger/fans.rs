use super::pwm::Pwm;
use crate::log_error;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Default, Copy, Clone)]
struct Duty {
    val: u8,
    duration: Option<Instant>,
}

impl Duty {
    pub fn new() -> Duty {
        Duty {
            val: 0,
            duration: Some(Instant::now()),
        }
    }
    /// Returns true if time > duration
    fn elapsed(&self, time: Duration) -> bool {
        match self.duration {
            None => false,
            Some(t) => t.elapsed().cmp(&time).is_gt(),
        }
    }
}
impl Into<u8> for Duty {
    fn into(self) -> u8 {
        self.val
    }
}

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
    pub fn disable(&mut self) {
        log_error!("PWM disable", self.pwm.enable(false));
        log_error!("PWM unexport", self.pwm.unexport());
    }
    pub fn update(&mut self, temp: f32) {
        let elapsed = self.duty.elapsed(Duration::from_secs(20));
        let new_duty = Duty {
            val: self.temp_to_duty(temp),
            duration: None,
        };

        if self.duty.val != new_duty.val {
            if self.duty.val > new_duty.val && !elapsed {
                // falling -> overrun fan for 20 seconds
                return;
            }
            self.duty = if new_duty.val < 20 {
                Duty::new()
            } else {
                new_duty
            }; // pwm noise below 20%
            log_error!("Set pwm", self.pwm.set_duty(self.duty.into()));
        }
    }

    fn temp_to_duty(&self, value: impl Into<f32>) -> u8 {
        // specify voltage range against fsd soc
        const CELL100: f32 = 60.0;
        const CELL0: f32 = 35.0;

        let old_range = CELL100 - CELL0;
        let new_range = 100.0 - 0.1;
        let value: f32 = value.into();
        (((((value - CELL0) * new_range) / old_range) + 0.1) as u8).min(100)
    }
}
