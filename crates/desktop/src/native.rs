use gpui::Window;

pub fn open_core() -> Result<(), String> {
    let directory = std::env::current_exe().map_err(|_| "Could not locate Companion")?;
    let mut directory = directory
        .parent()
        .ok_or("Could not locate Companion")?
        .to_path_buf();
    if directory
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == env!("CARGO_PKG_VERSION"))
    {
        directory = directory
            .parent()
            .and_then(|path| path.parent())
            .ok_or("Could not locate Companion")?
            .to_path_buf();
    }
    std::process::Command::new(directory.join("nolae-companion.exe"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| "Open Nolae Companion from its shortcut.".into())
}

#[cfg(windows)]
pub fn secure_stdio() {
    use windows::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        if let Ok(handle) = unsafe { GetStdHandle(kind) } {
            unsafe {
                let _ = SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
            }
        }
    }
}

#[cfg(not(windows))]
pub fn secure_stdio() {}

#[cfg(windows)]
pub fn activate(window: &mut Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{SW_RESTORE, ShowWindow},
    };
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        unsafe {
            let _ = ShowWindow(HWND(handle.hwnd.get() as *mut _), SW_RESTORE);
        }
    }
    window.activate_window();
}

#[cfg(not(windows))]
pub fn activate(window: &mut Window) {
    window.activate_window();
}

#[cfg(windows)]
pub fn show_error(message: &str) {
    use windows::{
        Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
        core::{PCWSTR, w},
    };
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("Nolae Companion"),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
pub fn show_error(message: &str) {
    eprintln!("{message}");
}
