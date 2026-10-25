//! Win32 calls are confined here. All HWND operations run on the message-loop thread.
use crate::monitor::{Bounds, Monitor};
use anyhow::{Context, Result, ensure};
use std::{
    num::NonZeroIsize,
    ptr::{null, null_mut},
};
use windows_sys::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SetLastError, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
    core::w,
};

pub(crate) struct DpiContext(DPI_AWARENESS_CONTEXT);
impl DpiContext {
    pub fn physical() -> Result<Self> {
        Self::set(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
    }
    fn set(context: DPI_AWARENESS_CONTEXT) -> Result<Self> {
        // SAFETY: documented context values or a context obtained from a live HWND.
        let previous = unsafe { SetThreadDpiAwarenessContext(context) };
        ensure!(
            !previous.is_null(),
            "could not set thread DPI awareness: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(previous))
    }
    fn for_window(window: HWND) -> Result<Self> {
        // SAFETY: the caller verifies the shell window before using it.
        Self::set(unsafe { GetWindowDpiAwarenessContext(window) })
    }
}
impl Drop for DpiContext {
    fn drop(&mut self) {
        // SAFETY: restore the context returned for this same thread.
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

fn wide_string(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|v| *v == 0).unwrap_or(value.len())])
}

pub(crate) fn monitors() -> Result<Vec<Monitor>> {
    struct Enumeration {
        monitors: Vec<Monitor>,
        failed: bool,
    }
    unsafe extern "system" fn visit(handle: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        // SAFETY: EnumDisplayMonitors invokes this synchronously with our stack context.
        let state = unsafe { &mut *(data as *mut Enumeration) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: handle is provided by Windows; info is writable with cbSize set.
        if unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) } == 0 {
            state.failed = true;
            return 0;
        }
        // Bound enumeration before allocating additional monitor records.
        if state.monitors.len() >= 64 {
            state.failed = true;
            return 0;
        }
        let name = wide_string(&info.szDevice);
        let mut device = DISPLAY_DEVICEW {
            cb: size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: NUL-terminated device name and a writable DISPLAY_DEVICEW.
        let found = unsafe {
            EnumDisplayDevicesW(
                info.szDevice.as_ptr(),
                0,
                &mut device,
                EDD_GET_DEVICE_INTERFACE_NAME,
            )
        } != 0;
        let id = if found {
            wide_string(&device.DeviceID)
        } else {
            String::new()
        };
        let rect = info.monitorInfo.rcMonitor;
        let mut dpi = (96, 96);
        // SAFETY: enumerated monitor and writable DPI values. A failed optional
        // query retains 96 DPI; physical geometry still comes from MONITORINFO.
        unsafe {
            GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi.0, &mut dpi.1);
        }
        state.monitors.push(Monitor {
            id: if id.is_empty() { name.clone() } else { id },
            name,
            description: if found {
                wide_string(&device.DeviceString)
            } else {
                String::new()
            },
            rect: Bounds {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            dpi,
        });
        1
    }
    let _dpi = DpiContext::physical()?;
    let mut state = Enumeration {
        monitors: Vec::new(),
        failed: false,
    };
    // SAFETY: the callback and state remain valid for this synchronous enumeration.
    let result = unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(visit),
            &mut state as *mut _ as LPARAM,
        )
    };
    ensure!(
        result != 0 && !state.failed,
        "could not enumerate Windows monitors"
    );
    state.monitors.sort_by(|a, b| a.id.cmp(&b.id));
    for monitor in &state.monitors {
        monitor.rect.size(i32::MAX as u32)?;
    }
    Ok(state.monitors)
}

/// Explorer's wallpaper WorkerW is an undocumented shell convention. Never fall
/// back to an ordinary top-level window. Raised desktops use a child below icons.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Desktop {
    parent: HWND,
    worker: HWND,
    icons: HWND,
    progman: HWND,
}
impl Desktop {
    fn layered(self) -> bool {
        !self.icons.is_null()
    }
}
impl Desktop {
    pub fn discover() -> Result<Self> {
        // SAFETY: all names are static NUL-terminated strings; no pointers are retained.
        unsafe {
            let progman = FindWindowW(w!("Progman"), null());
            ensure!(!progman.is_null(), "Explorer desktop is unavailable");
            // Ask Explorer to create its wallpaper host, bounding an unresponsive shell.
            let mut result = 0;
            ensure!(
                SendMessageTimeoutW(
                    progman,
                    0x052c,
                    0x0d,
                    0,
                    SMTO_ABORTIFHUNG,
                    1000,
                    &mut result
                ) != 0,
                "Explorer did not respond to wallpaper-host initialization"
            );
            let _ = SendMessageTimeoutW(
                progman,
                0x052c,
                0x0d,
                1,
                SMTO_ABORTIFHUNG,
                1000,
                &mut result,
            );
            let mut worker: HWND = null_mut();
            unsafe extern "system" fn visit(window: HWND, data: LPARAM) -> i32 {
                // SAFETY: synchronous EnumWindows callback with a writable HWND context.
                unsafe {
                    if !FindWindowExW(window, null_mut(), w!("SHELLDLL_DefView"), null()).is_null()
                    {
                        let candidate = FindWindowExW(null_mut(), window, w!("WorkerW"), null());
                        if !candidate.is_null()
                            && FindWindowExW(candidate, null_mut(), w!("SHELLDLL_DefView"), null())
                                .is_null()
                        {
                            *(data as *mut HWND) = candidate;
                            return 0;
                        }
                    }
                    1
                }
            }
            EnumWindows(Some(visit), &mut worker as *mut _ as LPARAM);
            let raised =
                GetWindowLongPtrW(progman, GWL_EXSTYLE) as u32 & WS_EX_NOREDIRECTIONBITMAP != 0;
            let icons = if raised {
                FindWindowExW(progman, null_mut(), w!("SHELLDLL_DefView"), null())
            } else {
                null_mut()
            };
            if raised {
                worker = FindWindowExW(progman, null_mut(), w!("WorkerW"), null());
                ensure!(
                    !icons.is_null(),
                    "raised Explorer desktop has no icon layer"
                );
            }
            ensure!(
                !worker.is_null(),
                "Explorer has no supported wallpaper WorkerW; desktop attachment is unavailable"
            );
            Ok(Self {
                parent: if raised { progman } else { worker },
                worker,
                icons,
                progman,
            })
        }
    }
    pub fn valid(self) -> bool {
        // SAFETY: IsWindow accepts stale handles. Check class too, since HWNDs can be reused.
        unsafe {
            let mut class = [0u16; 32];
            IsWindow(self.worker) != 0
                && FindWindowW(w!("Progman"), null()) == self.progman
                && (GetWindowLongPtrW(self.progman, GWL_EXSTYLE) as u32 & WS_EX_NOREDIRECTIONBITMAP
                    != 0)
                    == self.layered()
                && GetClassNameW(self.worker, class.as_mut_ptr(), class.len() as i32) > 0
                && wide_string(&class) == "WorkerW"
                && FindWindowExW(self.worker, null_mut(), w!("SHELLDLL_DefView"), null()).is_null()
                && (!self.layered()
                    || (GetParent(self.worker) == self.parent
                        && GetParent(self.icons) == self.parent))
        }
    }
}

pub(crate) struct WindowClass;
impl WindowClass {
    pub fn register() -> Result<Self> {
        // SAFETY: static class name and callback; class is unregistered after all windows drop.
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(null()),
                lpszClassName: w!("WispWallpaper"),
                ..Default::default()
            };
            ensure!(
                RegisterClassW(&class) != 0,
                "could not register Wisp window class: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(Self)
    }
}
impl Drop for WindowClass {
    fn drop(&mut self) {
        // SAFETY: Host and its windows have been dropped before the class guard.
        unsafe {
            UnregisterClassW(w!("WispWallpaper"), GetModuleHandleW(null()));
        }
    }
}
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Windows invokes this callback for registered Wisp windows. No Rust
    // state is borrowed by this callback; teardown is deferred to the outer loop.
    unsafe {
        match message {
            WM_NCDESTROY => {
                RemovePropW(hwnd, w!("WispWindowToken"));
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_NCHITTEST => HTTRANSPARENT as LRESULT,
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                ValidateRect(hwnd, null());
                0
            }
            WM_CLOSE => {
                PostQuitMessage(0);
                0
            }
            WM_QUERYENDSESSION => 1,
            WM_ENDSESSION if wparam != 0 => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// Owns only our child HWND, never an Explorer window. Kept after the GPU surface
/// in Output's field order so the surface is released first on every error path.
pub(crate) struct Window {
    handle: HWND,
    token: usize,
}
static NEXT_WINDOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
impl Window {
    pub fn create(desktop: Desktop, rect: Bounds) -> Result<Self> {
        ensure!(desktop.valid(), "wallpaper desktop disappeared");
        let _dpi = DpiContext::for_window(desktop.parent)?;
        let mut points = [
            POINT {
                x: rect.left,
                y: rect.top,
            },
            POINT {
                x: rect.right,
                y: rect.bottom,
            },
        ];
        // SAFETY: live desktop HWND and writable points. Convert physical monitor
        // coordinates into the parent's DPI context before mapping to its client area.
        unsafe {
            for point in &mut points {
                ensure!(
                    PhysicalToLogicalPointForPerMonitorDPI(desktop.parent, point) != 0,
                    "could not convert desktop coordinates"
                );
            }
            SetLastError(0);
            let mapped = MapWindowPoints(null_mut(), desktop.parent, points.as_mut_ptr(), 2);
            ensure!(
                mapped != 0 || std::io::Error::last_os_error().raw_os_error() == Some(0),
                "could not map monitor into desktop"
            );
            let width = points[1]
                .x
                .checked_sub(points[0].x)
                .context("desktop width overflow")?;
            let height = points[1]
                .y
                .checked_sub(points[0].y)
                .context("desktop height overflow")?;
            ensure!(width > 0 && height > 0, "empty desktop client bounds");
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TRANSPARENT
                    | if desktop.layered() { WS_EX_LAYERED } else { 0 },
                w!("WispWallpaper"),
                w!("Wisp Wallpaper"),
                WS_CHILD | WS_DISABLED | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                points[0].x,
                points[0].y,
                width,
                height,
                desktop.parent,
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            ensure!(
                !hwnd.is_null(),
                "could not create desktop child: {}",
                std::io::Error::last_os_error()
            );
            let token = NEXT_WINDOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if SetPropW(hwnd, w!("WispWindowToken"), token as _) == 0 {
                DestroyWindow(hwnd);
                anyhow::bail!("could not mark owned wallpaper window");
            }
            let window = Self {
                handle: hwnd,
                token,
            };
            if desktop.layered() {
                ensure!(
                    SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) != 0,
                    "could not initialize raised desktop opacity"
                );
            }
            window.place(desktop)?;
            Ok(window)
        }
    }
    pub fn raw_handle(&self) -> raw_window_handle::Win32WindowHandle {
        let mut handle = raw_window_handle::Win32WindowHandle::new(
            NonZeroIsize::new(self.handle as isize).unwrap(),
        );
        // SAFETY: obtains this executable's module, which lives for the process lifetime.
        handle.hinstance = NonZeroIsize::new(unsafe { GetModuleHandleW(null()) } as isize);
        handle
    }
    pub fn valid(&self, desktop: Desktop) -> bool {
        // SAFETY: these queries accept stale HWNDs.
        unsafe { self.owned() && GetParent(self.handle) == desktop.parent }
    }
    fn owned(&self) -> bool {
        // SAFETY: querying the token also rejects a destroyed HWND reused by Windows.
        unsafe {
            IsWindow(self.handle) != 0
                && GetPropW(self.handle, w!("WispWindowToken")) as usize == self.token
        }
    }
    pub fn place(&self, desktop: Desktop) -> Result<()> {
        ensure!(self.valid(desktop), "wallpaper window disappeared");
        if desktop.layered() {
            // SAFETY: move only our child, keeping it immediately below Explorer's icons.
            ensure!(
                unsafe {
                    SetWindowPos(
                        self.handle,
                        desktop.icons,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    )
                } != 0,
                "could not position wallpaper below desktop icons"
            );
        }
        Ok(())
    }
    pub fn show(&self) {
        // SAFETY: owned child window; never activates or modifies the parent.
        unsafe {
            ShowWindow(self.handle, SW_SHOWNOACTIVATE);
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: destruction is on the creating thread after its GPU surface drops.
        // Explorer may already have destroyed its children during a restart.
        unsafe {
            if self.owned() {
                DestroyWindow(self.handle);
            }
        }
    }
}

pub(crate) fn cursor() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    // SAFETY: writable stack point, no input capture or window activation.
    (unsafe { GetPhysicalCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}
pub(crate) fn pump_messages() -> bool {
    // SAFETY: this thread owns the windows and pumps only its own message queue.
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
            if message.message == WM_QUIT {
                return false;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    true
}
pub(crate) fn wait(duration: std::time::Duration) {
    // Round up to avoid a sub-millisecond busy loop. Wake promptly on window messages.
    let milliseconds = duration.as_millis().saturating_add(1).min(100) as u32;
    // SAFETY: no handles; the wait watches this thread's message queue only.
    unsafe {
        MsgWaitForMultipleObjectsEx(0, null(), milliseconds, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an interactive Windows Explorer desktop"]
    fn desktop_children_have_monitor_bounds_and_do_not_take_input() {
        let _dpi = DpiContext::physical().unwrap();
        let _class = WindowClass::register().unwrap();
        let desktop = Desktop::discover().unwrap();
        let monitors = monitors().unwrap();
        assert!(!monitors.is_empty());
        let foreground = unsafe { GetForegroundWindow() };
        let mut windows = Vec::new();
        for monitor in monitors {
            let window = Window::create(desktop, monitor.rect).unwrap();
            assert!(window.valid(desktop));
            // SAFETY: queries of the owned window and a writable rectangle.
            unsafe {
                let style = GetWindowLongPtrW(window.handle, GWL_STYLE) as u32;
                let extended = GetWindowLongPtrW(window.handle, GWL_EXSTYLE) as u32;
                assert_ne!(style & WS_CHILD, 0);
                assert_ne!(style & WS_DISABLED, 0);
                assert_eq!(style & WS_POPUP, 0);
                assert_ne!(extended & WS_EX_NOACTIVATE, 0);
                assert_eq!(
                    SendMessageW(window.handle, WM_NCHITTEST, 0, 0),
                    HTTRANSPARENT as isize
                );
                let mut actual = RECT::default();
                assert_ne!(GetWindowRect(window.handle, &mut actual), 0);
                assert_eq!(
                    (actual.left, actual.top, actual.right, actual.bottom),
                    (
                        monitor.rect.left,
                        monitor.rect.top,
                        monitor.rect.right,
                        monitor.rect.bottom
                    )
                );
                assert_eq!(GetForegroundWindow(), foreground);
            }
            windows.push(window);
        }
        let handles: Vec<_> = windows.iter().map(|w| w.handle).collect();
        drop(windows);
        for handle in handles {
            assert_eq!(unsafe { IsWindow(handle) }, 0);
        }
        assert!(
            desktop.valid(),
            "cleanup must leave Explorer windows intact"
        );
    }
}
