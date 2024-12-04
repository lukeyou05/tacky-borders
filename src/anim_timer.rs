use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

use crate::utils::WM_APP_ANIMATE;
use crate::{post_message_w, SendHWND};

#[derive(Debug, Clone)]
pub struct AnimationTimer {
    stop_flag: Arc<Mutex<bool>>,
}

impl AnimationTimer {
    pub fn start(hwnd: HWND, interval_ms: u64) -> Self {
        let stop_flag = Arc::new(Mutex::new(false));
        let stop_flag_clone = stop_flag.clone();

        // Wrap HWND in a struct that implements Send and Sync to move it into the thread
        let window = SendHWND(hwnd);

        // Spawn a worker thread for the timer
        thread::spawn(move || {
            let window_sent = window;
            let interval = Duration::from_millis(interval_ms);

            while !*stop_flag_clone.lock().unwrap() {
                if post_message_w(window_sent.0, WM_APP_ANIMATE, WPARAM(0), LPARAM(0)).is_err() {
                    error!("The animation timer failed to send the animate message");
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
