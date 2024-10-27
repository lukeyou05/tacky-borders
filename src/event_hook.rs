use windows::{
    Win32::Foundation::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
};

use crate::window_border::WindowBorder;
use crate::BORDERS;
use crate::set_event_hook;
use crate::spawn_border_thread;
use crate::destroy_border_thread;

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
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
            let hwnd_isize = hwnd.0 as isize;
            let border_option = borders.get(&hwnd_isize);

            if border_option.is_some() {
                let border_window: HWND = HWND(*border_option.unwrap() as _);
                unsafe { SendMessageW(border_window, WM_MOVE, WPARAM(0), LPARAM(0)); }
            }
            drop(borders);
        },
        EVENT_OBJECT_REORDER => {
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
           
            // I have to loop through because for whatever reason, EVENT_OBJECT_REORDER only gets
            // sent with some random memory address that might be important but idk.
            for key in borders.keys() {
                let border_window: HWND = HWND(*borders.get(&key).unwrap() as _);
                unsafe { SendMessageW(border_window, WM_SETFOCUS, WPARAM(0), LPARAM(0)) };
            }
        },
        EVENT_OBJECT_SHOW => {
            // I may not have to check IsWindowVisible
            if unsafe { IsWindowVisible(hwnd).as_bool() } {
                // Check if the window is a tool window or popup
                let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
                let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };

                if ex_style & WS_EX_TOOLWINDOW.0 != 0 || style & WS_CHILD.0 != 0 || style & WS_POPUP.0 != 0 {
                    return;
                }

                println!("creating window border for: {:?}", hwnd);
                spawn_border_thread(hwnd);
            }
        },
        // TODO I don't know if destroying the window borders or just hiding them would be better.
        EVENT_OBJECT_HIDE => {
            // I have to explicitly check IsWindowVisible because for whatever fucking reason,
            // EVENT_OBJECT_HIDE can be sent even when the window is still visible.
            if unsafe { !IsWindowVisible(hwnd).as_bool() } {
                // Due to the fact that these callback functions can be re-entered, I can just
                // spawn a new thread here to ensure the border gets destroyed even if re-entrancy
                // happens.
                destroy_border_thread(hwnd);
            }
        },
        EVENT_OBJECT_CLOAKED => {
            destroy_border_thread(hwnd);
        },
        _ => {}
    }
}
