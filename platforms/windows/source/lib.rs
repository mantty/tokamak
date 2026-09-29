//! The Windows app shell. The platform pack ships it with the runtime as a
//! static library, which an app build links into the app's executable.
#![cfg(windows)]

mod shell;

/// The app executable's entry point, which the C runtime's startup for
/// Windows applications calls.
#[unsafe(export_name = "wWinMain")]
pub extern "system" fn win_main(
    _instance: *mut std::ffi::c_void,
    _previous_instance: *mut std::ffi::c_void,
    _command_line: *mut u16,
    _show: i32,
) -> i32 {
    match shell::run() {
        Ok(()) => 0,
        Err(error) => {
            show_startup_error(&error);
            1
        }
    }
}

fn show_startup_error(error: &anyhow::Error) {
    use windows::{
        Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
        core::{HSTRING, w},
    };

    let message = HSTRING::from(format!("The app could not start.\n\n{error:#}"));
    unsafe {
        MessageBoxW(None, &message, w!("tokamak"), MB_OK | MB_ICONERROR);
    }
}
