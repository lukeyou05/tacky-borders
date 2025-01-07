use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::time::interval;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

use crate::post_message_w;
use crate::utils::WM_APP_ANIMATE;

#[derive(Debug, Default)]
pub struct AnimationTimer {
    stop_flag: Arc<Mutex<bool>>,
    #[allow(unused)]
    tokio_runtime: Option<Runtime>, // This keeps the runtime in scope. Otherwise, the timer breaks.
}

impl AnimationTimer {
    pub fn start(hwnd: HWND, interval_ms: u64) -> Self {
        let stop_flag = Arc::new(Mutex::new(false));
        let stop_flag_clone = stop_flag.clone();

        let hwnd_isize = hwnd.0 as isize;

        // TODO: i might want to make other functions in the thread async instead
        let tokio_runtime = match Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                error!("could not create tokio runtime for animation timer: {err}");
                return Self::default();
            }
        };

        // Spawn an async worker closure for the timer
        tokio_runtime.spawn(async move {
            let mut interval = interval(Duration::from_millis(interval_ms));

            while !*stop_flag_clone.lock().unwrap() {
                let hwnd = HWND(hwnd_isize as _);
                if let Err(e) = post_message_w(hwnd, WM_APP_ANIMATE, WPARAM(0), LPARAM(0)) {
                    error!(
                        "could not send animation timer message for {:?}: {}",
                        hwnd, e
                    );
                    break;
                }
                interval.tick().await;
            }
        });

        // Return the timer instance
        Self {
            stop_flag,
            tokio_runtime: Some(tokio_runtime),
        }
    }

    pub fn stop(&mut self) {
        // Signal the worker thread to stop
        if let Ok(mut flag) = self.stop_flag.lock() {
            *flag = true;
        }
    }
}
