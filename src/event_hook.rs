use anyhow::Context;
use std::thread;
use std::time;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED,
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, OBJID_CURSOR,
    OBJID_WINDOW,
};

use crate::utils::{
    destroy_border_for_window, get_border_for_window, get_foreground_window,
    hide_border_for_window, is_window_visible, post_message_w, send_notify_message_w,
    show_border_for_window, LogIfErr, WM_APP_FOREGROUND, WM_APP_LOCATIONCHANGE, WM_APP_MINIMIZEEND,
    WM_APP_MINIMIZESTART, WM_APP_REORDER,
};
use crate::APP_STATE;

pub extern "system" fn process_win_event(
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
            for value in APP_STATE.borders.lock().unwrap().values() {
                let border_window = HWND(*value as _);
                if is_window_visible(border_window) {
                    post_message_w(border_window, WM_APP_REORDER, WPARAM(0), LPARAM(0))
                        .context("EVENT_OBJECT_REORDER")
                        .log_if_err();
                }
            }
        }
        // Both the HWND passed by the event and the one returned by GetForegroundWindow() should
        // refer to the same "active" window, but they don't.
        EVENT_SYSTEM_FOREGROUND => {
            let potential_active_hwnd = get_foreground_window();

            // I GIVE UP I ACTUALLY GIVE UP im just gonna poll
            if potential_active_hwnd != _hwnd && !APP_STATE.is_polling_active_window() {
                poll_active_window_with_limit(3);
            } else {
                handle_foreground_event(potential_active_hwnd, _hwnd);
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

fn poll_active_window_with_limit(max_polls: u32) {
    APP_STATE.set_polling_active_window(true);

    let _ = thread::spawn(move || {
        for _ in 0..max_polls {
            let current_active_hwnd = HWND(*APP_STATE.active_window.lock().unwrap() as _);
            let new_active_hwnd = get_foreground_window();

            if new_active_hwnd != current_active_hwnd && !new_active_hwnd.is_invalid() {
                handle_foreground_event(new_active_hwnd, current_active_hwnd);
            }

            thread::sleep(time::Duration::from_millis(50));
        }

        APP_STATE.set_polling_active_window(false);
    });
}

fn handle_foreground_event(potential_active_hwnd: HWND, event_hwnd: HWND) {
    let new_active_window = match !potential_active_hwnd.is_invalid() {
        true => potential_active_hwnd.0 as isize,
        false => event_hwnd.0 as isize,
    };
    *APP_STATE.active_window.lock().unwrap() = new_active_window;

    // Send foreground messages to all the border windows
    for (key, val) in APP_STATE.borders.lock().unwrap().iter() {
        let border_window = HWND(*val as _);
        // NOTE: some apps can become foreground even if they're not visible, so we also
        // have to check the keys against the active_window HWND from earlier
        if is_window_visible(border_window) || *key == new_active_window {
            post_message_w(border_window, WM_APP_FOREGROUND, WPARAM(0), LPARAM(0))
                .context("EVENT_OBJECT_FOCUS")
                .log_if_err();
        }
    }
}
