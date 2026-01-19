#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[macro_use]
extern crate log;
extern crate sp_log;

use anyhow::Context;
use std::sync::LazyLock;
use tacky_borders::sys_tray_icon::create_tray_icon;
use tacky_borders::utils::{
    LogIfErr, imm_disable_ime, set_process_dpi_awareness_context, spawn_window_state_poller,
};
use tacky_borders::{
    APP_STATE, create_borders_for_existing_windows, is_unwanted_instance,
    register_border_window_class, set_event_hook,
};
use windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

fn main() {
    if is_unwanted_instance() {
        return;
    }

    // Force initialization of our app state
    let _ = LazyLock::force(&APP_STATE);

    info!("starting tacky-borders");

    // xFFFFFFFF (-1) is used to disable IME windows for all threads in the current process.
    imm_disable_ime(0xFFFFFFFF)
        .ok()
        .context("could not disable ime")
        .log_if_err();

    set_process_dpi_awareness_context(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
        .context("could not make process dpi aware")
        .log_if_err();

    let hwineventhook = set_event_hook();

    // This owns the tray icon window, so it must be kept in scope
    let tray_icon_res = create_tray_icon(hwineventhook);
    if let Err(err) = tray_icon_res {
        error!("could not create tray icon: {err}");
    }

    register_border_window_class().log_if_err();
    create_borders_for_existing_windows().log_if_err();
    spawn_window_state_poller();

    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    info!("exiting tacky-borders");
}
