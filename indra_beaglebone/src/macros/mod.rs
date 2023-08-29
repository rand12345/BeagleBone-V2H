#[macro_export]
macro_rules! log_error {
    ($stage:expr, $expr:expr) => {
        match $expr {
            Ok(_) => log::debug!("{} Ok()", $stage),
            Err(error) => log::error!("{} {}", $stage, error),
        }
    };
}

#[macro_export]
macro_rules! spawn_with_eventbus {
    ($eventbus:expr, $($task:expr),*) => {
        $(
            tokio::spawn($task($eventbus.clone()));
        )*
    };
}
