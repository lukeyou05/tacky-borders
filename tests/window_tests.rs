use serial_test::serial;
use std::{thread, time};
use tacky_borders::utils::get_window_class;
use tacky_borders::{
    clear_borders, create_borders_for_existing_windows, register_border_window_class,
    reload_borders,
};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, TRUE};
use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

#[test]
#[serial]
fn clear_borders_test() -> anyhow::Result<()> {
    register_border_window_class()?;

    for _ in 0..5 {
        create_borders_for_existing_windows()?;

        // This is kinda jank becauuse we have to wait for the border windows to actually be created
        // (they're all in separate threads), but 50ms should be more than long enough for that.
        thread::sleep(time::Duration::from_millis(50));
        clear_borders();

        // Again, we have to wait a few ms for the threads to actually process their messages and
        // exit, but 50ms should also be enough for that
        thread::sleep(time::Duration::from_millis(50));
        unsafe { EnumWindows(Some(enum_windows_callback), LPARAM::default()) }?;
    }

    Ok(())
}

#[test]
#[serial]
// This tests whether all borders are properly cleaned up when reload_borders() is called, and if
// not, it tests whether we still have their handles so they can still be cleaned up when
// clear_borders() is called later.
fn reload_borders_test() -> anyhow::Result<()> {
    register_border_window_class()?;
    create_borders_for_existing_windows()?;

    // TODO: It fails if I move the thread::sleep out of the loop (i.e. if we reload borders too
    // quickly). This might be because we are sending windows messages before they are even
    // created, causing them to not close properly. In the real world, most people wouldn't be
    // making the borders reload that quickly, but it's still worth fixing.
    for _ in 0..5 {
        thread::sleep(time::Duration::from_millis(50));
        reload_borders();
    }

    thread::sleep(time::Duration::from_millis(500));
    clear_borders();

    thread::sleep(time::Duration::from_millis(500));
    unsafe { EnumWindows(Some(enum_windows_callback), LPARAM::default()) }?;

    Ok(())
}

unsafe extern "system" fn enum_windows_callback(_hwnd: HWND, _lparam: LPARAM) -> BOOL {
    let window_class = get_window_class(_hwnd).unwrap();
    assert!(window_class != "border");

    TRUE
}
