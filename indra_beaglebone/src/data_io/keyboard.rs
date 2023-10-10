// use crate::eventbus::Event;
// use crate::eventbus::Event::{ChademoCmd, PreCommand};
use crate::{
    chademo::state::{ChargerState, STATE},
    // eventbus::EvtBus,
    global_state::{ChargeParameters, OperationMode},
    log_error,
    pre_charger::PreCommand,
    statics::{ChademoTx, PreTx, OPERATIONAL_MODE},
};
use std::io::Read;

pub async fn scan_kb(pre_tx: PreTx, chademo_tx: ChademoTx) {
    // default to V2H
    // {
    //     *OPERATIONAL_MODE.clone().lock().await = OperationMode::V2h; // v2h
    //     if let Err(e) = gpiocmd_sender2.send(ChargerState::Stage1).await {
    //         eprintln!("{e:?}")
    //     }
    // }
    let operational_mode = OPERATIONAL_MODE.clone();
    loop {
        // Input: c for manual charge, d for V2H, s to stop, q to quit (+CR)
        let mut input = [0u8; 2];

        let _ = std::io::stdin().lock();
        match std::io::stdin().read(&mut input) {
            Ok(_) => {
                println!("Input received:");
                println!("{:?}", input[0]);
            }
            Err(e) => eprintln!("Error reading input: {}", e),
        };
        match input[0] {
            115 => {
                // "s" stop

                *operational_mode.lock().await = OperationMode::Idle;
                log_error!("kb", pre_tx.send(PreCommand::Disable).await);
                if let Err(e) = chademo_tx.send(ChargerState::Idle).await {
                    eprintln!("{e:?}")
                };
            }
            100 => {
                // "d" V2H (default)
                *operational_mode.lock().await = OperationMode::V2h;
                if matches!(STATE.lock().await.0, ChargerState::Idle) {
                    if let Err(e) = chademo_tx.send(ChargerState::Stage1).await {
                        eprintln!("{e:?}")
                    }
                } else {
                    if let Err(e) = chademo_tx.send(ChargerState::Stage6).await {
                        eprintln!("{e:?}")
                    }
                }
            }
            99 => {
                // "c" manual charge
                *operational_mode.lock().await = OperationMode::Charge(ChargeParameters::default());
                if matches!(STATE.lock().await.0, ChargerState::Idle) {
                    if let Err(e) = chademo_tx.send(ChargerState::Stage1).await {
                        eprintln!("{e:?}")
                    }
                } else {
                    if let Err(e) = chademo_tx.send(ChargerState::Stage6).await {
                        eprintln!("{e:?}")
                    }
                }
            }
            113 => {
                // "q" quit
                log_error!("kb", pre_tx.send(PreCommand::Disable).await);
                if let Err(e) = chademo_tx.send(ChargerState::Exiting).await {
                    log::error!("{e:?}")
                };
                println!(" q key captured. Exiting...");
                if let Err(e) = chademo_tx.send(ChargerState::Exiting).await {
                    log::error!("{e:?}")
                };
            }
            _ => continue,
        }
    }
}
