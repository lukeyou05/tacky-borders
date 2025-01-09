use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

use crate::post_message_w;
use crate::utils::WM_APP_ANIMATE;

#[derive(Debug, Clone)]
pub struct AnimationTimer {
    stop_flag: Arc<Mutex<bool>>,
}

impl AnimationTimer {
    pub fn start(hwnd: HWND, interval_ms: u64) -> Self {
        let stop_flag = Arc::new(Mutex::new(false));
        let stop_flag_clone = stop_flag.clone();

        let hwnd_isize = hwnd.0 as isize;

        // Spawn a worker thread for the timer
        thread::spawn(move || {
            let hwnd = HWND(hwnd_isize as _);
            let interval = Duration::from_millis(interval_ms);

            while !*stop_flag_clone.lock().unwrap() {
                if let Err(e) = post_message_w(Some(hwnd), WM_APP_ANIMATE, WPARAM(0), LPARAM(0)) {
                    error!(
                        "could not send animation timer message for {:?}: {}",
                        hwnd, e
                    );
                    break;
                }
                thread::sleep(interval);
            }
        });

        // Return the timer instance
        Self { stop_flag }
    }

    pub fn stop(&mut self) {
        // Signal the worker thread to stop
        if let Ok(mut flag) = self.stop_flag.lock() {
            *flag = true;
        }
    }
}
