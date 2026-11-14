//! Windows desktop entry point; the console binary retains normal CLI output.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use std::{os::windows::process::CommandExt, process::Command};
    let result = std::env::current_exe().and_then(|path| {
        Command::new(path.with_file_name("su03t-update.exe"))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
    });
    if let Err(error) = result {
        let title: Vec<u16> = "SU-03T Tools\0".encode_utf16().collect();
        let message: Vec<u16> =
            format!("Could not start SU-03T Tools. Keep both executables together.\n{error}\0")
                .encode_utf16()
                .collect();
        // Both UTF-16 buffers remain alive and are NUL-terminated for the call.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The desktop launcher is for Windows; run su03t-update on this platform.");
    std::process::exit(1);
}
