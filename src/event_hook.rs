use windows::{
    Win32::Foundation::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
    Win32::Graphics::Dwm::*,
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
        /*EVENT_OBJECT_CREATE => {
            // Check if the window is a tool window or popup
            let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
            let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };
            let mut is_cloaked = FALSE;
            let result = unsafe { DwmGetWindowAttribute(
                hwnd, 
                DWMWA_CLOAKED,
                std::ptr::addr_of_mut!(is_cloaked) as *mut _,
                size_of::<BOOL>() as u32
            ) };
            if result.is_err() {
                return;
            }

            if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_POPUP.0 == 0 && style & WS_CHILD.0 == 0 && !is_cloaked.as_bool() {
                //println!("creating window border for: {:?}", hwnd);
                spawn_border_thread(hwnd, 300);
            }
        },*/
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
            if unsafe { IsWindowVisible(hwnd).as_bool() } {
                // Check if the window is a tool window or popup
                let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
                let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };
                let mut is_cloaked = FALSE;
                let result = unsafe { DwmGetWindowAttribute(
                    hwnd, 
                    DWMWA_CLOAKED,
                    std::ptr::addr_of_mut!(is_cloaked) as *mut _,
                    size_of::<BOOL>() as u32
                ) };
                if result.is_err() {
                    return;
                }

                if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_POPUP.0 == 0 && style & WS_CHILD.0 == 0 && !is_cloaked.as_bool() {
                    //println!("creating window border for: {:?}", hwnd);
                    spawn_border_thread(hwnd, 0);
                }
            }
        },
        EVENT_OBJECT_HIDE => {
            // I have to explicitly check IsWindowVisible because for whatever reason,
            // EVENT_OBJECT_HIDE can be sent even when the window is still visible (happens with
            // Vesktop, for example).
            if unsafe { !IsWindowVisible(hwnd).as_bool() } {
                destroy_border_thread(hwnd);
            }
        },
        EVENT_OBJECT_UNCLOAKED => {
            if unsafe { IsWindowVisible(hwnd).as_bool() } {
                let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
                let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };
                let mut is_cloaked = FALSE;
                let result = unsafe { DwmGetWindowAttribute(
                    hwnd, 
                    DWMWA_CLOAKED,
                    std::ptr::addr_of_mut!(is_cloaked) as *mut _,
                    size_of::<BOOL>() as u32
                ) };
                if result.is_err() {
                    return;
                }

                if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_POPUP.0 == 0 && style & WS_CHILD.0 == 0 && !is_cloaked.as_bool() {
                    //println!("creating window border for: {:?}", hwnd);
                    spawn_border_thread(hwnd, 0);
                }
            }
        },
        EVENT_OBJECT_CLOAKED => {
            destroy_border_thread(hwnd);
        },
        _ => {}
    }
}
