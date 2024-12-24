use anyhow::Context;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED,
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, OBJID_CURSOR,
    OBJID_WINDOW,
};

use crate::utils::{
    destroy_border_for_window, get_border_for_window, hide_border_for_window, is_window_visible,
    post_message_w, send_notify_message_w, show_border_for_window, LogIfErr, WM_APP_FOREGROUND,
    WM_APP_LOCATIONCHANGE, WM_APP_MINIMIZEEND, WM_APP_MINIMIZESTART, WM_APP_REORDER,
};
use crate::window_border::ACTIVE_WINDOW;
use crate::BORDERS;

pub extern "system" fn handle_win_event(
    _h_win_event_hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _dw_event_thread: u32,
    _dwms_event_time: u32,
) {
    // Ignore cursor events
    if _id_object == OBJID_CURSOR.0 {
        return;
    }

    match _event {
        EVENT_OBJECT_LOCATIONCHANGE => {
            if _id_child != CHILDID_SELF as i32 {
                return;
            }

            if let Some(border) = get_border_for_window(_hwnd) {
                send_notify_message_w(border, WM_APP_LOCATIONCHANGE, WPARAM(0), LPARAM(0))
                    .context("EVENT_OBJECT_LOCATIONCHANGE")
                    .log_if_err();
            }
        }
        EVENT_OBJECT_REORDER => {
            // Send reorder messages to all the border windows
            for value in BORDERS.lock().unwrap().values() {
                let border_window = HWND(*value as _);
                if is_window_visible(border_window) {
                    post_message_w(border_window, WM_APP_REORDER, WPARAM(0), LPARAM(0))
                        .context("EVENT_OBJECT_REORDER")
                        .log_if_err();
                }
            }
        }
        EVENT_SYSTEM_FOREGROUND => {
            let hwnd_isize = _hwnd.0 as isize;
            *ACTIVE_WINDOW.lock().unwrap() = hwnd_isize;

            // Send foreground messages to all the border windows
            for (key, val) in BORDERS.lock().unwrap().iter() {
                let border_window = HWND(*val as _);
                // Some apps like Flow Launcher can become focused even if they aren't visible yet,
                // so I also need to check if 'key' is equal to '_hwnd' (the foreground window)
                if is_window_visible(border_window) || key == &hwnd_isize {
                    post_message_w(border_window, WM_APP_FOREGROUND, WPARAM(0), LPARAM(0))
                        .context("EVENT_OBJECT_FOCUS")
                        .log_if_err();
                }
            }
        }
        EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED => {
            if _id_object == OBJID_WINDOW.0 {
                show_border_for_window(_hwnd);
            }
        }
        EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED => {
            if _id_object == OBJID_WINDOW.0 {
                hide_border_for_window(_hwnd);
            }
        }
        EVENT_SYSTEM_MINIMIZESTART => {
            if let Some(border) = get_border_for_window(_hwnd) {
                post_message_w(border, WM_APP_MINIMIZESTART, WPARAM(0), LPARAM(0))
                    .context("EVENT_SYSTEM_MINIMIZESTART")
                    .log_if_err();
            }
        }
        EVENT_SYSTEM_MINIMIZEEND => {
            if let Some(border) = get_border_for_window(_hwnd) {
                post_message_w(border, WM_APP_MINIMIZEEND, WPARAM(0), LPARAM(0))
                    .context("EVENT_SYSTEM_MINIMIZEEND")
                    .log_if_err();
            }
        }
        EVENT_OBJECT_DESTROY => {
            if _id_object == OBJID_WINDOW.0 && _id_child == CHILDID_SELF as i32 {
                destroy_border_for_window(_hwnd);
            }
        }
        _ => {}
    }
}
