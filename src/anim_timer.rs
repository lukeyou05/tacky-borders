use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::utils::WM_APP_ANIMATE;
use crate::SendHWND;

#[derive(Debug, Clone)]
pub struct AnimationTimer {
    stop_flag: Arc<Mutex<bool>>,
}

impl AnimationTimer {
    pub fn start(hwnd: HWND, interval_ms: u64) -> Self {
        unsafe {
            // Create a stop flag
            let stop_flag = Arc::new(Mutex::new(false));
            let stop_flag_clone = stop_flag.clone();

            // Wrap HWND in a struct to move it into the thread safely
            let win = SendHWND(hwnd);

            // Spawn a worker thread for the timer
            thread::spawn(move || {
                let window = win;
                let interval = Duration::from_millis(interval_ms);
                while !*stop_flag_clone.lock().unwrap() {
                    if PostMessageW(window.0, WM_APP_ANIMATE, WPARAM(0), LPARAM(0)).is_err() {
                        error!("The animation timer failed to send the animate message");
                        break;
                    }
                    thread::sleep(interval);
                }
                //debug!("stop flag for timer received!");
            });

            // Return the timer instance
            Self { stop_flag }
        }
    }

    pub fn stop(&mut self) {
        // Signal the worker thread to stop
        if let Ok(mut flag) = self.stop_flag.lock() {
            *flag = true;
        }
    }
}
