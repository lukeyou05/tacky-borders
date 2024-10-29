use windows::{
    Win32::Foundation::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
    Win32::Graphics::Dwm::*,
};

use crate::window_border::WindowBorder;
use crate::BORDERS;
use crate::SendHWND;
use crate::set_event_hook;
use crate::utils::*;

pub extern "system" fn handle_win_event_main(
    h_win_event_hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    dw_event_thread: u32,
    dwms_event_time: u32,
) {
    if id_object == OBJID_CURSOR.0 {
        return;
    }

    match event {
        EVENT_OBJECT_LOCATIONCHANGE => {
            if has_filtered_style(hwnd) {
                return;
            }

            let border_window = get_border_from_window(hwnd); 
            if border_window.is_some() {
                unsafe { SendMessageW(border_window.unwrap(), WM_MOVE, WPARAM(0), LPARAM(0)); }
            }
        },
        EVENT_OBJECT_FOCUS => {
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
           
            // I have to loop through because for whatever reason, EVENT_OBJECT_REORDER only gets
            // sent with some random memory address that might be important but idk.
            for value in borders.values() {
                let border_window: HWND = HWND(*value as _);
                if unsafe { IsWindowVisible(border_window).as_bool() } {
                    unsafe { PostMessageW(border_window, WM_SETFOCUS, WPARAM(0), LPARAM(0)) };
                }
            }
            drop(borders);
        },
        EVENT_OBJECT_SHOW => {
            show_border_for_window(hwnd, 300);
        },
        EVENT_OBJECT_HIDE => {
            // I have to check IsWindowVisible because for some reason, EVENT_OBJECT_HIDE can be
            // sent even while the window is still visible (it does this for Vesktop)
            if unsafe { !IsWindowVisible(hwnd).as_bool() } {
                hide_border_of_window(hwnd);
            } 
        },
        EVENT_OBJECT_UNCLOAKED => {
            show_border_for_window(hwnd, 0);
        },
        EVENT_OBJECT_CLOAKED => {
            hide_border_of_window(hwnd);
        },
        EVENT_SYSTEM_MINIMIZEEND => {
            let border_window = get_border_from_window(hwnd); 
            if border_window.is_some() {
                unsafe { SendMessageW(border_window.unwrap(), WM_QUERYOPEN, WPARAM(0), LPARAM(0)); }
            }
        },
        // TODO this is called an unnecessary number of times which can reduce performance. I need
        // to find a way to filter more windows out.
        EVENT_OBJECT_DESTROY => {
            if has_filtered_style(hwnd) {
                return;
            } else {
                destroy_border_of_window(hwnd);

                // Use below to debug whether window borders are properly destroyed
                let mutex = unsafe { &*BORDERS };
                let borders = mutex.lock().unwrap();
                println!("borders after destroying window: {:?}", borders);
                drop(borders);
            }
        },
        _ => {}
    }
}
