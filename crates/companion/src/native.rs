pub const TRAY_GUID: u128 = 0xac98148c_7cdc_4ea3_9c64_0c5ff3e42d81;

#[cfg(windows)]
mod platform {
    use windows::{
        Win32::{
            Foundation::{
                CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT,
                WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
            },
            System::{
                LibraryLoader::GetModuleHandleW,
                Threading::{
                    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent,
                },
            },
            UI::{
                Shell::{
                    NIF_GUID, NIF_INFO, NIIF_INFO, NIIF_NOSOUND, NIIF_RESPECT_QUIET_TIME,
                    NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
                },
                WindowsAndMessaging::*,
            },
        },
        core::{PCWSTR, w},
    };

    const CLASS: PCWSTR = w!("NolaeCompanionRuntime");
    const TITLE: PCWSTR = w!("Nolae Companion Runtime");
    const WAKE: u32 = WM_APP + 1;
    pub const SHOW: u32 = WM_APP + 2;

    pub struct Instance(HANDLE);
    impl Instance {
        pub fn acquire(show_existing: bool) -> Result<Option<Self>, Box<dyn std::error::Error>> {
            let handle = unsafe { CreateMutexW(None, false, w!("Local\\NolaeCompanion")) }?;
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(handle) }?;
                if show_existing {
                    for _ in 0..20 {
                        if let Ok(window) = unsafe { FindWindowW(CLASS, TITLE) } {
                            unsafe {
                                let _ = PostMessageW(Some(window), SHOW, WPARAM(0), LPARAM(0));
                            }
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                return Ok(None);
            }
            Ok(Some(Self(handle)))
        }
    }
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    #[derive(Clone, Copy)]
    pub struct Waker(usize);
    impl Waker {
        pub fn wake(self) {
            unsafe {
                let _ = PostMessageW(Some(HWND(self.0 as *mut _)), WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub struct EventLoop {
        window: HWND,
        stop: HANDLE,
    }
    impl EventLoop {
        pub fn new() -> windows::core::Result<Self> {
            let instance = unsafe { GetModuleHandleW(None) }?;
            let class = WNDCLASSW {
                hInstance: instance.into(),
                lpszClassName: CLASS,
                lpfnWndProc: Some(window_proc),
                ..Default::default()
            };
            unsafe {
                RegisterClassW(&class);
            }
            let window = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    CLASS,
                    TITLE,
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
            }?;
            let stop = unsafe { CreateEventW(None, true, false, w!("Local\\NolaeCompanionStop")) }?;
            Ok(Self { window, stop })
        }

        pub fn waker(&self) -> Waker {
            Waker(self.window.0 as usize)
        }

        pub fn wait(&self) -> windows::core::Result<Option<bool>> {
            let result = unsafe {
                MsgWaitForMultipleObjectsEx(
                    Some(&[self.stop]),
                    INFINITE,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                )
            };
            if result == WAIT_OBJECT_0 {
                return Ok(None);
            }
            if result == WAIT_FAILED {
                return Err(windows::core::Error::from_thread());
            }
            let mut message = MSG::default();
            let mut show = false;
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
                if message.message == WM_QUIT {
                    return Ok(None);
                }
                show |= message.hwnd == self.window && message.message == SHOW;
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            Ok(Some(show))
        }
    }
    impl Drop for EventLoop {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.window);
                let _ = CloseHandle(self.stop);
            }
        }
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    pub fn request_stop() -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..100 {
            if let Some(instance) = Instance::acquire(false)? {
                drop(instance);
                return Ok(());
            }
            match unsafe { OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\NolaeCompanionStop")) }
            {
                Ok(handle) => {
                    let result = unsafe { SetEvent(handle) };
                    unsafe { CloseHandle(handle) }?;
                    result?;
                }
                Err(error) if error.code() == windows::core::HRESULT::from_win32(2) => {}
                Err(error) => return Err(error.into()),
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("Nolae Companion is still closing. Try again.".into())
    }

    pub fn open_url(url: &str) {
        let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(url.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            );
        }
    }

    pub fn show_error(message: &str) {
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

    pub fn notify_update(tray: &tray_icon::TrayIcon, title: &str, message: &str) -> bool {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(tray.window_handle()),
            uFlags: NIF_GUID | NIF_INFO,
            guidItem: windows::core::GUID::from_u128(super::TRAY_GUID),
            dwInfoFlags: NIIF_INFO | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME,
            ..Default::default()
        };
        for (target, source) in data
            .szInfoTitle
            .iter_mut()
            .take(63)
            .zip(title.encode_utf16())
        {
            *target = source;
        }
        for (target, source) in data.szInfo.iter_mut().take(255).zip(message.encode_utf16()) {
            *target = source;
        }
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() }
    }
}

#[cfg(windows)]
pub use platform::*;
