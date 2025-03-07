#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[macro_use]
extern crate log;
extern crate sp_log;

use anyhow::Context;
use tacky_borders::utils::{LogIfErr, imm_disable_ime, set_process_dpi_awareness_context};
use tacky_borders::{
    create_borders_for_existing_windows, create_logger, register_border_window_class,
    set_event_hook, sys_tray_icon,
};
use windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

fn main() {
    if let Err(e) = create_logger() {
        println!("[ERROR] {}", e);
    };

    info!("starting tacky-borders");

    // xFFFFFFFF can be used to disable IME windows for all threads in the current process.
    if !imm_disable_ime(0xFFFFFFFF).as_bool() {
        error!("could not disable ime!");
    }

    set_process_dpi_awareness_context(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
        .context("could not make process dpi aware")
        .log_if_err();

    let hwineventhook = set_event_hook();

    // This is responsible for the actual tray icon window, so it must be kept in scope
    let tray_icon_res = sys_tray_icon::create_tray_icon(hwineventhook);
    if let Err(e) = tray_icon_res {
        // TODO for some reason if I use {:#} or {:?}, it repeatedly prints the error. Could be
        // something to do with how it implements .source()?
        error!("could not create tray icon: {e:#?}");
    }

    register_border_window_class().log_if_err();
    create_borders_for_existing_windows().log_if_err();

    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    info!("exiting tacky-borders");
}
