use std::time::Instant;
use std::time::Duration;
use std::sync::LazyLock;
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

// TODO replace static mut with some sort of cell for more safe behavior.
//static TIMER: Cell<LazyLock<Instant>> = Cell::new(LazyLock::new(|| Instant::now()));
//static mut TIMER: LazyLock<Instant> = LazyLock::new(|| Instant::now());
const REFRESH_INTERVAL: Duration = Duration::from_millis(7);

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
    //let before = std::time::Instant::now();
    match event {
        //TODO find a better way to prevent reentrancy (currently i have a workaround using a timer)
        EVENT_OBJECT_LOCATIONCHANGE => {
            //let timer = unsafe { &*TIMER };
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
            let hwnd_isize = hwnd.0 as isize;
            let border_option = borders.get(&hwnd_isize);

            if border_option.is_some() {
                let border_window: HWND = HWND(*border_option.unwrap() as _);
                unsafe {
                    // I'm just using a timer here to make sure we're not sending too many messages
                    // in a short time span bc if that happens, then the border window will keep
                    // processing the backlog of WM_MOVE messages even after the tracking window
                    // has stopped moving (for a long time too) (I REFACTORED SOME CODE AND THIS
                    // GOT FIXED SOMEHOW????? oh i think it's cuz i set
                    // D2D1_PRESENT_OPTIONS_IMMEDIATELY or whatever it was).
                    /*if timer.elapsed() >= REFRESH_INTERVAL {
                        // Reset the timer and re-initialize it because it is a LazyLock 
                        TIMER = LazyLock::new(|| Instant::now());
                        &*TIMER;

                        SendMessageW(border_window, WM_MOVE, WPARAM(0), LPARAM(0));
                        //println!("Elapsed time (event_hook, total): {:.2?}", before.elapsed());
                    }*/

                    SendMessageW(border_window, WM_MOVE, WPARAM(0), LPARAM(0));
                    //println!("Elapsed time (event_hook, total): {:.2?}", before.elapsed());
                }
            }
            drop(borders);
        },
        EVENT_OBJECT_FOCUS => {
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
           
            // I have to loop through because it doesn't always send this event for each window
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

                spawn_border_thread(hwnd);
            }
        },
        // Destroying the border everytime it is hidden may increase CPU usage (or maybe not
        // because there are no longer unnecessary message loops), but it will save memory.
        EVENT_OBJECT_HIDE => {
            // I have to explicitly check IsWindowVisible because for whatever fucking reason,
            // EVENT_OBJECT_HIDE is sent even when the window is still visible.
            if unsafe { !IsWindowVisible(hwnd).as_bool() } {
                // Due to the fact that these callback functions can be re-entered, I can just
                // spawn a new thread here to ensure the border gets destroyed even if re-entrancy
                // happens.
                destroy_border_thread(hwnd);
            }
        },
        _ => {}
    }
}
