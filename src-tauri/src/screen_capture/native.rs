use super::{Region, MAX_PIXELS};
#[derive(Clone, Debug)]
pub(super) struct Target {
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub hwnd: usize,
    pid: u32,
    created: u64,
    class: String,
    lease: Option<std::sync::Arc<WindowLease>>,
}
#[derive(Debug)]
struct WindowLease {
    hwnd: usize,
    name: Vec<u16>,
    token: usize,
}
#[cfg(windows)]
pub(super) struct PhysicalPixels(windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
#[cfg(windows)]
impl PhysicalPixels {
    pub fn new() -> Self {
        use windows_sys::Win32::UI::HiDpi::*;
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}
#[cfg(windows)]
impl Drop for PhysicalPixels {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}
#[cfg(windows)]
impl Drop for WindowLease {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            if GetPropW(self.hwnd as _, self.name.as_ptr()) as usize == self.token {
                RemovePropW(self.hwnd as _, self.name.as_ptr());
            }
        }
    }
}
impl Target {
    pub fn client_offset(&self) -> Result<(u32, u32), String> {
        #[cfg(windows)]
        {
            let _physical = PhysicalPixels::new();
            use windows_sys::Win32::{
                Foundation::{POINT, RECT},
                Graphics::{Dwm::*, Gdi::ClientToScreen},
            };
            let mut origin = POINT { x: 0, y: 0 };
            let mut bounds: RECT = unsafe { std::mem::zeroed() };
            unsafe {
                if ClientToScreen(self.hwnd as _, &mut origin) == 0
                    || DwmGetWindowAttribute(
                        self.hwnd as _,
                        DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                        &mut bounds as *mut _ as _,
                        std::mem::size_of::<RECT>() as u32,
                    ) < 0
                {
                    return Err("The selected window's client bounds are unavailable.".into());
                }
            }
            if origin.x < bounds.left || origin.y < bounds.top {
                return Err(
                    "The selected window's client bounds are incompatible with graphics capture."
                        .into(),
                );
            }
            Ok((
                (origin.x - bounds.left) as u32,
                (origin.y - bounds.top) as u32,
            ))
        }
        #[cfg(not(windows))]
        Err("Window geometry requires Windows.".into())
    }
    pub fn kind(&self) -> &str {
        if self.hwnd == 0 {
            "monitor"
        } else {
            "window"
        }
    }
    pub fn dimensions(&self, region: Option<Region>) -> Result<(u32, u32), String> {
        let (w, h) = if let Some(r) = region {
            if self.hwnd != 0
                || r.width == 0
                || r.height == 0
                || r.x.checked_add(r.width).is_none_or(|v| v > self.width)
                || r.y.checked_add(r.height).is_none_or(|v| v > self.height)
            {
                return Err("Choose a region entirely inside the selected monitor.".into());
            }
            (r.width, r.height)
        } else {
            (self.width, self.height)
        };
        if w < 2 || h < 2 || w as u64 * h as u64 > MAX_PIXELS {
            return Err(
                "Capture dimensions must contain 2–24 million pixels and fit the selected source."
                    .into(),
            );
        }
        Ok((w, h))
    }
    pub fn input_args(&self, region: Option<Region>, fps: u32) -> Result<Vec<String>, String> {
        let (w, h) = self.dimensions(region)?;
        let mut args = vec![
            "-f".into(),
            "gdigrab".into(),
            "-framerate".into(),
            fps.to_string(),
            "-draw_mouse".into(),
            "0".into(),
        ];
        if self.hwnd == 0 {
            let r = region.unwrap_or(Region {
                x: 0,
                y: 0,
                width: w,
                height: h,
            });
            args.extend([
                "-offset_x".into(),
                (self.left as i64 + r.x as i64).to_string(),
                "-offset_y".into(),
                (self.top as i64 + r.y as i64).to_string(),
                "-video_size".into(),
                format!("{w}x{h}"),
            ]);
        }
        args.extend([
            "-i".into(),
            if self.hwnd == 0 {
                "desktop".into()
            } else {
                format!("hwnd={}", self.hwnd)
            },
        ]);
        Ok(args)
    }
    pub fn validate(&self) -> Result<(), String> {
        validate(self)
    }
    pub fn lease(&self) -> Result<Self, String> {
        self.validate()?;
        let mut target = self.clone();
        if self.hwnd != 0 {
            #[cfg(windows)]
            {
                use windows_sys::Win32::UI::WindowsAndMessaging::*;
                let name: Vec<u16> = format!("nabnab-capture-{}\0", super::id("s"))
                    .encode_utf16()
                    .collect();
                let token =
                    super::NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as usize;
                unsafe {
                    if SetPropW(self.hwnd as _, name.as_ptr(), token as _) == 0 {
                        return Err("The selected window cannot be safely bound for capture. Choose a window running with the same Windows privileges.".into());
                    }
                }
                target.lease = Some(std::sync::Arc::new(WindowLease {
                    hwnd: self.hwnd,
                    name,
                    token,
                }));
            }
        }
        Ok(target)
    }
}
#[cfg(windows)]
mod win {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, HWND, LPARAM, RECT},
        Graphics::{Dwm::*, Gdi::*},
        System::Threading::*,
        UI::WindowsAndMessaging::*,
    };
    fn wide_string(s: &[u16]) -> String {
        String::from_utf16_lossy(&s[..s.iter().position(|c| *c == 0).unwrap_or(s.len())])
    }
    fn process_created(pid: u32) -> Option<u64> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut times = [FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            }; 4];
            let ok = GetProcessTimes(
                handle,
                &mut times[0],
                &mut times[1],
                &mut times[2],
                &mut times[3],
            );
            CloseHandle(handle);
            (ok != 0)
                .then_some(((times[0].dwHighDateTime as u64) << 32) | times[0].dwLowDateTime as u64)
        }
    }
    pub fn window(hwnd: HWND) -> Option<Target> {
        let _physical = PhysicalPixels::new();
        unsafe {
            if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                return None;
            }
            let mut cloaked = 0u32;
            let _ =
                DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut cloaked as *mut _ as _, 4);
            if cloaked != 0 {
                return None;
            }
            let mut title = [0u16; 512];
            if GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) == 0 {
                return None;
            }
            let mut class = [0u16; 256];
            GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
            let mut rect: RECT = std::mem::zeroed();
            if GetClientRect(hwnd, &mut rect) == 0 || rect.right <= 1 || rect.bottom <= 1 {
                return None;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let created = process_created(pid)?;
            Some(Target {
                label: wide_string(&title),
                width: rect.right as u32,
                height: rect.bottom as u32,
                left: 0,
                top: 0,
                hwnd: hwnd as usize,
                pid,
                created,
                class: wide_string(&class),
                lease: None,
            })
        }
    }
    unsafe extern "system" fn window_cb(hwnd: HWND, data: LPARAM) -> i32 {
        let targets = &mut *(data as *mut Vec<Target>);
        if targets.len() < 128 {
            if let Some(t) = window(hwnd) {
                targets.push(t);
            }
        }
        1
    }
    unsafe extern "system" fn monitor_cb(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(monitor, &mut info as *mut _ as _) != 0 {
            let r = info.monitorInfo.rcMonitor;
            (&mut *(data as *mut Vec<Target>)).push(Target {
                label: format!(
                    "Monitor {} · {} × {}",
                    wide_string(&info.szDevice),
                    r.right - r.left,
                    r.bottom - r.top
                ),
                width: (r.right - r.left) as u32,
                height: (r.bottom - r.top) as u32,
                left: r.left,
                top: r.top,
                hwnd: 0,
                pid: 0,
                created: 0,
                class: wide_string(&info.szDevice),
                lease: None,
            });
        }
        1
    }
    pub fn enumerate() -> Result<Vec<Target>, String> {
        let _physical = PhysicalPixels::new();
        let mut targets = monitors();
        unsafe {
            EnumWindows(Some(window_cb), &mut targets as *mut _ as _);
        }
        Ok(targets)
    }
    fn monitors() -> Vec<Target> {
        let mut targets = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(monitor_cb),
                &mut targets as *mut _ as _,
            );
        }
        targets
    }
    pub fn validate(t: &Target) -> Result<(), String> {
        if let Some(lease) = &t.lease {
            unsafe {
                if GetPropW(t.hwnd as _, lease.name.as_ptr()) as usize != lease.token {
                    return Err("The selected window was destroyed or its handle reused. Select and preview it again.".into());
                }
            }
        }
        if t.hwnd != 0 {
            let now = window(t.hwnd as _).ok_or("The selected window was closed or minimized.")?;
            if now.pid != t.pid || now.created != t.created || now.class != t.class {
                return Err("The selected window changed. Select and preview it again.".into());
            }
            if now.width != t.width || now.height != t.height {
                return Err("The selected window changed size. The captured portion is recoverable; preview its new size.".into());
            }
        } else {
            if !monitors().iter().any(|n| {
                n.hwnd == 0
                    && n.class == t.class
                    && n.width == t.width
                    && n.height == t.height
                    && n.left == t.left
                    && n.top == t.top
            }) {
                return Err("The selected display changed or disconnected.".into());
            }
        }
        Ok(())
    }
    pub fn clock() -> Result<(i64, i64), String> {
        use windows_sys::Win32::System::Performance::*;
        unsafe {
            let mut qpc = 0;
            let mut frequency = 0;
            if QueryPerformanceCounter(&mut qpc) == 0
                || QueryPerformanceFrequency(&mut frequency) == 0
                || frequency <= 0
            {
                return Err("The capture clock is unavailable.".into());
            }
            let qpc_us = ((qpc as i128) * 1_000_000 / frequency as i128) as i64;
            Ok((super::super::epoch_us(), qpc_us))
        }
    }
}
#[cfg(windows)]
pub(super) use win::{clock, enumerate, validate};
#[cfg(not(windows))]
pub fn enumerate() -> Result<Vec<Target>, String> {
    Err("Screen capture is available on Windows 10/11.".into())
}
#[cfg(not(windows))]
fn validate(_: &Target) -> Result<(), String> {
    Err("Screen capture requires Windows.".into())
}
#[cfg(not(windows))]
pub fn clock() -> Result<(i64, i64), String> {
    Err("Capture clock requires Windows.".into())
}
