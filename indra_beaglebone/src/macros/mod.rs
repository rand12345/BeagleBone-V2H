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
macro_rules! async_timeout_loop {
    ($timeout_ms:expr, $loop_ms:expr, $init:expr) => {{
        let timeout_duration = std::time::Duration::from_millis($timeout_ms);
        let loop_duration = std::time::Duration::from_millis($loop_ms);

        let instant = std::time::Instant::now();
        let mut condition = false;

        let result = async move {
            while instant.elapsed() < timeout_duration && !condition {
                let init_result = $init().await;
                condition = init_result;

                tokio::time::sleep(loop_duration).await;
            }

            if condition {
                Ok(())
            } else {
                Err(IndraError::Timeout)
            }
        };

        Box::pin(result)
    }};
}
#[macro_export]
macro_rules! async_timeout_result {
    ($timeout_ms:expr, $loop_ms:expr, $init:expr) => {{
        let timeout_duration = std::time::Duration::from_millis($timeout_ms);
        let loop_duration = std::time::Duration::from_millis($loop_ms);

        let instant = std::time::Instant::now();
        let mut condition = false;

        let result = async move {
            while instant.elapsed() < timeout_duration && !condition {
                let init_result = $init().await;
                condition = init_result.is_ok();

                tokio::time::sleep(loop_duration).await;
            }

            if condition {
                Ok(())
            } else {
                Err(IndraError::Timeout)
            }
        };

        Box::pin(result)
    }};
}

#[macro_export]
macro_rules! timeout_condition {
    ($timeout_ms:expr, $loop_ms:expr, $condition:expr) => {{
        let timeout_duration = std::time::Duration::from_millis($timeout_ms);
        let loop_duration = std::time::Duration::from_millis($loop_ms);

        let instant = std::time::Instant::now();
        while instant.elapsed() < timeout_duration && !$condition {
            tokio::time::sleep(loop_duration).await;
        }

        if $condition {
            Ok(())
        } else {
            Err(IndraError::Timeout)
        }
    }};
}
