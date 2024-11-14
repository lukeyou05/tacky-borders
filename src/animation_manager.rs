use std::thread;
use std::time;
use windows::{Win32::Foundation::*, Win32::UI::WindowsAndMessaging::*};

use crate::utils::*;
use crate::BORDERS;

pub fn animation_manager() {
    let _ = thread::spawn(|| loop {
        for value in BORDERS.lock().unwrap().values() {
            let border_window: HWND = HWND(*value as _);
            if is_window_visible(border_window) {
                unsafe {
                    let _ = PostMessageW(border_window, WM_APP_ANIMATE, WPARAM(0), LPARAM(0));
                }
            }
        }
        // Hard coded 30fps right now
        thread::sleep(time::Duration::from_millis(33));
    });
}
