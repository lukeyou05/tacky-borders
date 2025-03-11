use serial_test::serial;
use std::{thread, time};
use tacky_borders::utils::get_window_class;
use tacky_borders::{
    create_borders_for_existing_windows, destroy_borders, register_border_window_class,
    reload_borders,
};
use windows::Win32::Foundation::{HWND, LPARAM, TRUE};
use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
use windows::core::BOOL;

#[test]
#[serial]
fn test_destroy_borders() -> anyhow::Result<()> {
    register_border_window_class()?;

    for _ in 0..5 {
        // This is kinda jank becauuse we have to wait for the border windows to actually be created
        // (they're all in separate threads), but 50ms should be more than long enough for that.
        create_borders_for_existing_windows()?;
        thread::sleep(time::Duration::from_millis(50));

        destroy_borders();

        unsafe { EnumWindows(Some(enum_windows_tests_callback), LPARAM::default()) }?;
    }

    Ok(())
}

#[test]
#[serial]
// This tests whether all borders are properly cleaned up when reload_borders() is called
fn test_reload_borders() -> anyhow::Result<()> {
    register_border_window_class()?;
    create_borders_for_existing_windows()?;

    for _ in 0..5 {
        reload_borders();
    }
    destroy_borders();

    unsafe { EnumWindows(Some(enum_windows_tests_callback), LPARAM::default()) }?;

    Ok(())
}

unsafe extern "system" fn enum_windows_tests_callback(_hwnd: HWND, _lparam: LPARAM) -> BOOL {
    let window_class = get_window_class(_hwnd).unwrap();
    assert!(window_class != "border");

    TRUE
}

// TODO: test border window rect with positive border offsets and negative effects translations
