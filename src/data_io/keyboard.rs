use crate::{
    api::OperationMode,
    chademo::state::{ChargerState, OPERATIONAL_MODE, STATE},
    log_error,
    pre_charger::pre_commands::PreCmd,
};
use std::io::Read;
use tokio::sync::mpsc::Sender;

pub async fn scan_kb(precmd_sender1: &Sender<PreCmd>, gpiocmd_sender2: &Sender<ChargerState>) {
    // default to V2H
    // {
    //     *OPERATIONAL_MODE.clone().lock().await = OperationMode::V2h; // v2h
    //     if let Err(e) = gpiocmd_sender2.send(ChargerState::Stage1).await {
    //         eprintln!("{e:?}")
    //     }
    // }
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

                *OPERATIONAL_MODE.clone().lock().await = OperationMode::Idle;
                log_error!("kb", precmd_sender1.send(PreCmd::Disable).await);
                if let Err(e) = gpiocmd_sender2.send(ChargerState::Idle).await {
                    eprintln!("{e:?}")
                };
            }
            100 => {
                // "d" V2H (default)
                *OPERATIONAL_MODE.clone().lock().await = OperationMode::V2h;
                if matches!(STATE.lock().await.0, ChargerState::Idle) {
                    if let Err(e) = gpiocmd_sender2.send(ChargerState::Stage1).await {
                        eprintln!("{e:?}")
                    }
                } else {
                    if let Err(e) = gpiocmd_sender2.send(ChargerState::Stage6).await {
                        eprintln!("{e:?}")
                    }
                }
            }
            99 => {
                // "c" manual charge
                *OPERATIONAL_MODE.clone().lock().await = OperationMode::Charge;
                if matches!(STATE.lock().await.0, ChargerState::Idle) {
                    if let Err(e) = gpiocmd_sender2.send(ChargerState::Stage1).await {
                        eprintln!("{e:?}")
                    }
                } else {
                    if let Err(e) = gpiocmd_sender2.send(ChargerState::Stage6).await {
                        eprintln!("{e:?}")
                    }
                }
            }
            113 => {
                // "q" quit
                log_error!("kb", precmd_sender1.send(PreCmd::Disable).await);
                if let Err(e) = gpiocmd_sender2.send(ChargerState::Exiting).await {
                    log::error!("{e:?}")
                };
                println!(" q key captured. Exiting...");
                if let Err(e) = gpiocmd_sender2.send(ChargerState::Exiting).await {
                    log::error!("{e:?}")
                };
            }
            _ => continue,
        }
    }
}
