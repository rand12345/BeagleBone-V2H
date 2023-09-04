use crate::MAX_AMPS;
use serde::{Deserialize, Serialize};
use std::ops::Deref;

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_json() {
        let cp = ChargeParameters {
            amps: Some(15),
            eco: Some(false),
            soc_limit: Some(80),
        };
        let op = OperationMode::Charge(cp);
        let json = serde_json::to_string(&op).unwrap();
        println!("{json}")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ChargeParameters {
    amps: Option<u8>,
    eco: Option<bool>,
    soc_limit: Option<u8>,
}

impl ChargeParameters {
    pub fn get_amps(&self) -> u8 {
        match self.amps {
            Some(amps) => amps,
            None => MAX_AMPS,
        }
    }
    pub fn set_amps(&mut self, limit: u8) -> Self {
        self.amps = Some(limit);
        *self.deref()
    }
    pub fn get_eco(&self) -> bool {
        match self.eco {
            Some(b) => b,
            None => false,
        }
    }
    pub fn set_eco(&mut self, enabled: bool) -> Self {
        self.eco = Some(enabled);
        *self.deref()
    }
    pub fn get_soc_limit(&self) -> Option<u8> {
        self.soc_limit
    }
    pub fn set_soc_limit(&mut self, soc_limit: u8) -> Self {
        self.soc_limit = Some(soc_limit);
        *self.deref()
    }
}
impl Default for ChargeParameters {
    fn default() -> Self {
        Self {
            amps: None,
            eco: None,
            soc_limit: None,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy)]
pub enum OperationMode {
    /// Bidirectional load matching
    V2h,
    /// Custom charge profile
    Charge(ChargeParameters),
    /// Safe idle mode - plug unlocked
    #[default]
    Idle,
    /// Bad state
    Uninitalised,
    /// Shutdown all peripherals and safetly unlock plug
    Quit,
    /// Forced discharge mode
    Discharge(ChargeParameters),
}

impl OperationMode {
    pub fn eco_charge(&mut self, enabled: bool) {
        let cp = ChargeParameters::default().set_eco(enabled);
        *self = Self::Charge(cp);
    }
    pub fn limit_soc(&mut self, limit: u8) {
        let cp = ChargeParameters::default().set_soc_limit(limit);
        *self = Self::Charge(cp)
    }
    pub fn limit_amps(&mut self, limit: u8) {
        let cp = ChargeParameters::default().set_amps(limit);
        *self = Self::Charge(cp)
    }
    pub fn is_eco(&self) -> bool {
        match self {
            Self::Charge(p) => p.eco.is_some(),
            _ => false,
        }
    }
    pub fn is_discharge(&self) -> bool {
        matches!(self, Self::Discharge(_))
    }
    pub fn soc_limit(&self) -> Option<u8> {
        match self {
            Self::Charge(p) => p.soc_limit,
            _ => None,
        }
    }
    pub fn boost(&mut self) {
        use OperationMode::*;
        *self = match self {
            Quit => Quit,
            Uninitalised => Idle,
            Discharge(_) | V2h | Idle => Charge(ChargeParameters::default()),
            Charge(_) => V2h,
        }
    }
    pub fn onoff(&mut self) {
        use OperationMode::*;
        *self = match self {
            Quit => Quit,
            Uninitalised => Idle,
            Discharge(_) | V2h | Charge(_) => Idle,
            Idle => V2h,
        }
    }
    pub fn idle(&mut self) {
        use OperationMode::*;
        *self = Idle;
    }
    pub fn is_idle(&self) -> bool {
        use OperationMode::*;
        matches!(self, Idle)
    }
    pub fn is_quit(&self) -> bool {
        use OperationMode::*;
        matches!(self, Quit)
    }
    pub fn is_uninitalised(&self) -> bool {
        use OperationMode::*;
        matches!(self, Uninitalised)
    }
    pub fn is_inactive(&self) -> bool {
        self.is_idle() || self.is_quit() || self.is_uninitalised()
    }

    pub fn is_charge(&self) -> bool {
        use OperationMode::*;
        matches!(self, Charge(_))
    }
    pub fn is_v2h(&self) -> bool {
        use OperationMode::*;
        matches!(self, V2h)
    }
}
