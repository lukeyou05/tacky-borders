use anyhow::Context;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED,
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, OBJID_CLIENT,
    OBJID_CURSOR, OBJID_WINDOW,
};

use crate::APP_STATE;
use crate::utils::{
    LogIfErr, WM_APP_FOREGROUND, WM_APP_LOCATIONCHANGE, WM_APP_MINIMIZEEND, WM_APP_MINIMIZESTART,
    WM_APP_REORDER, destroy_border_for_window, get_border_for_window, get_foreground_window,
    hide_border_for_window, is_window_visible, post_message_w, send_notify_message_w,
    show_border_for_window,
};

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
            // Ignore OBJIDs not needed for window Z-order handling.
            if _id_object != OBJID_CLIENT.0 {
                return;
            }

            // Send reorder messages to all the border windows
            for value in APP_STATE.borders.lock().unwrap().values() {
                let border_window = HWND(*value as _);
                if is_window_visible(border_window) {
                    post_message_w(Some(border_window), WM_APP_REORDER, WPARAM(0), LPARAM(0))
                        .context("EVENT_OBJECT_REORDER")
                        .log_if_err();
                }
            }
        }
        // Neither the HWND passed by this event nor the one returned by GetForegroundWindow() are
        // accurate 100% of the time. I tried finding workarounds without polling, but gave up.
        EVENT_SYSTEM_FOREGROUND => {
            let potential_active_hwnd = get_foreground_window();

            // Immediately try these HWNDs, and if they're wrong, hope that polling works.
            handle_foreground_event(potential_active_hwnd, _hwnd);
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
                post_message_w(Some(border), WM_APP_MINIMIZESTART, WPARAM(0), LPARAM(0))
                    .context("EVENT_SYSTEM_MINIMIZESTART")
                    .log_if_err();
            }
        }
        EVENT_SYSTEM_MINIMIZEEND => {
            if let Some(border) = get_border_for_window(_hwnd) {
                post_message_w(Some(border), WM_APP_MINIMIZEEND, WPARAM(0), LPARAM(0))
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

pub fn handle_foreground_event(best_hwnd_guess: HWND, other_hwnd_guess: HWND) {
    let new_active_hwnd = match !best_hwnd_guess.is_invalid() {
        true => best_hwnd_guess,
        false => other_hwnd_guess,
    };
    *APP_STATE.active_window.lock().unwrap() = new_active_hwnd.0 as isize;

    // Send foreground messages to all the border windows
    for (key, val) in APP_STATE.borders.lock().unwrap().iter() {
        let border_window = HWND(*val as _);
        // Some apps can become foreground even if they're not visible, so we also have to check
        // the keys against the active_window HWND from earlier
        if is_window_visible(border_window) || *key == new_active_hwnd.0 as isize {
            post_message_w(Some(border_window), WM_APP_FOREGROUND, WPARAM(0), LPARAM(0))
                .context("EVENT_OBJECT_FOCUS")
                .log_if_err();
        }
    }
}
