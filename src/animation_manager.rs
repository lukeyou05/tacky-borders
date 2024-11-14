use std::thread;
use std::time;
use windows::{Win32::Foundation::*, Win32::UI::WindowsAndMessaging::*};

use crate::border_config::CONFIG;
use crate::utils::*;
use crate::BORDERS;

pub fn animation_manager() {
    println!("Spawning animation manager!");
    let _ = thread::spawn(|| loop {
        for value in BORDERS.lock().unwrap().values() {
            let border_window: HWND = HWND(*value as _);
            if is_window_visible(border_window) {
                unsafe {
                    let _ = PostMessageW(border_window, WM_APP_ANIMATE, WPARAM(0), LPARAM(0));
                }
            }
        }
        // TODO destroy this thread and create a new one when someone reloads the config because
        // currently, I need to constantly check the config via get_sleep_duration if I want to
        // account for changes in animation_fps. If I just destroy and re-create the thread, I can
        // just call get_sleep_duration one time outside the loop instead.
        thread::sleep(time::Duration::from_millis(get_sleep_duration()));
    });
}

fn get_sleep_duration() -> u64 {
    (1000 / CONFIG.lock().unwrap().global.animation_fps.unwrap_or(30)) as u64
}
