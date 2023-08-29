use crate::{
    // chademo::state::ChargerState,
    data_io::panel::ButtonTriggered,
    pre_charger::PreCmd,
};

pub type EvtBus = (flume::Sender<Event>, flume::Receiver<Event>);
// type EventBus = (flume::Sender<Event>, flume::Receiver<Event>);

#[derive(Copy, Clone, Debug)]
pub enum Event {
    NewSchedule,
    Powerdown,
    PreCommand(PreCmd),
    // ChademoCmd(ChargerState),
    ButtonResponse(ButtonTriggered),
    // LedCommand(Led),
}

pub fn get_eb(amount: usize) -> EvtBus {
    flume::bounded::<Event>(100)
}
