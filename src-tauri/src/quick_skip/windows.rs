use std::{
    cell::{Cell, RefCell},
    ptr::null_mut,
    sync::{mpsc, Mutex},
    thread,
};
use tauri::{AppHandle, Emitter, EventTarget, Manager, WebviewUrl, WebviewWindowBuilder};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LMENU, VK_LWIN, VK_MENU,
            VK_RETURN, VK_RMENU, VK_RWIN, VK_SPACE, VK_UP,
        },
        WindowsAndMessaging::*,
    },
};

use super::{Action, Value, OVERLAY};

const INPUT_EVENT: &str = "quick-skip-input";
const WIDTH: f64 = 370.;
const HEIGHT: f64 = 480.;

#[derive(Default)]
pub struct DesktopWheel(Mutex<State>);

#[derive(Default)]
struct State {
    shortcut: Option<Shortcut>,
    ready: bool,
    configuration: Option<Value>,
    revision: u32,
    serial: u32,
    open: Option<u32>,
}

pub async fn on_main(
    app: AppHandle,
    work: impl FnOnce(&AppHandle) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let task = app.clone();
    app.run_on_main_thread(move || {
        let _ = send.send(work(&task));
    })
    .map_err(|error| error.to_string())?;
    receive.await.map_err(|error| error.to_string())?
}

impl DesktopWheel {
    pub fn configure(
        &self,
        app: &AppHandle,
        configuration: Option<Value>,
        revision: u32,
    ) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        if revision <= state.revision {
            return Ok(());
        }
        state.revision = revision;
        state.configuration = configuration;
        if state.configuration.is_none() {
            state.shortcut.take();
            if state.open.take().is_some() {
                if let Some(window) = app.get_webview_window(OVERLAY) {
                    if let Ok(handle) = window.hwnd() {
                        hide_overlay(handle.0 as _);
                    }
                }
            }
        } else if app.get_webview_window(OVERLAY).is_none() {
            WebviewWindowBuilder::new(app, OVERLAY, WebviewUrl::App("quick-skip.html".into()))
                .title("nabnab · Quick Skip")
                .decorations(false)
                .shadow(false)
                .resizable(false)
                .inner_size(WIDTH, HEIGHT)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .focusable(false)
                .visible(false)
                .build()
                .map_err(|error| error.to_string())?;
        } else {
            if state.ready && state.shortcut.is_none() {
                state.shortcut = Some(Shortcut::start(app.clone())?);
            }
            if let Some(gesture) = state.open {
                if let Some(playing) = state
                    .configuration
                    .as_ref()
                    .and_then(|value| value.get("playing"))
                {
                    let _ = app.emit_to(
                        EventTarget::webview(OVERLAY),
                        INPUT_EVENT,
                        serde_json::json!({"type":"playing","gesture":gesture,"playing":playing}),
                    );
                }
            }
        }
        Ok(())
    }

    pub fn action(&self, app: &AppHandle, action: Action) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        match action {
            Action::Ready => {
                state.ready = true;
                if state.configuration.is_some() && state.shortcut.is_none() {
                    state.shortcut = Some(Shortcut::start(app.clone())?);
                }
            }
            Action::Hide { gesture } if state.open == Some(gesture) => {
                state.open = None;
                if let Some(window) = app.get_webview_window(OVERLAY) {
                    let handle = window.hwnd().map_err(|error| error.to_string())?;
                    hide_overlay(handle.0 as _);
                }
            }
            Action::Transport { gesture, action } if state.open == Some(gesture) => {
                app.emit_to(EventTarget::webview("main"), "quick-skip-action", action)
                    .map_err(|error| error.to_string())?;
            }
            _ => {}
        }
        Ok(())
    }

    fn input(&self, app: &AppHandle, input: Input) -> tauri::Result<()> {
        let mut state = self.0.lock().unwrap();
        let Some(window) = app.get_webview_window(OVERLAY) else {
            return Ok(());
        };
        let target = EventTarget::webview(OVERLAY);
        if let Input::Open(x, y, step) = input {
            let Some(configuration) = state.configuration.clone() else {
                return Ok(());
            };
            let Some(monitor) = app.monitor_from_point(x.into(), y.into())? else {
                return Ok(());
            };
            let area = monitor.work_area();
            let scale = monitor.scale_factor();
            let width = (WIDTH * scale).round() as i32;
            let height = (HEIGHT * scale).round() as i32;
            let width = width.min(area.size.width as i32);
            let height = height.min(area.size.height as i32);
            let left = x + width > area.position.x + area.size.width as i32;
            let anchor_x = (if left { x - width } else { x }).clamp(
                area.position.x,
                area.position.x + area.size.width as i32 - width,
            );
            let top = (y - height / 2).clamp(
                area.position.y,
                area.position.y + area.size.height as i32 - height,
            );
            let gesture = state.serial.wrapping_add(1);
            state.serial = gesture;
            state.open = Some(gesture);
            // Move and reveal without taking focus from the window beneath the pointer.
            unsafe {
                SetWindowPos(
                    window.hwnd()?.0 as _,
                    HWND_TOPMOST,
                    anchor_x,
                    top,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            app.emit_to(
                target,
                INPUT_EVENT,
                serde_json::json!({
                    "type":"open", "gesture":gesture, "x":f64::from(x - anchor_x)/scale,
                    "y":f64::from(y - top)/scale, "step":step, "configuration":configuration,
                }),
            )?;
        } else if let Some(gesture) = state.open {
            let event = match input {
                Input::Open(..) => unreachable!(),
                Input::Move(step) => {
                    serde_json::json!({"type":"move","gesture":gesture,"step":step})
                }
                Input::Key(key) => serde_json::json!({"type":key,"gesture":gesture}),
                Input::Pointer(x, y) => {
                    let p = window.outer_position()?;
                    let size = window.outer_size()?;
                    if x >= p.x
                        && y >= p.y
                        && x < p.x + size.width as i32
                        && y < p.y + size.height as i32
                    {
                        return Ok(());
                    }
                    serde_json::json!({"type":"cancel","gesture":gesture})
                }
            };
            app.emit_to(target, INPUT_EVENT, event)?;
        }
        Ok(())
    }
}

fn hide_overlay(window: HWND) {
    unsafe {
        SetWindowPos(
            window,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_HIDEWINDOW | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        );
    }
}

pub fn shutdown(app: &AppHandle) {
    app.state::<DesktopWheel>()
        .0
        .lock()
        .unwrap()
        .shortcut
        .take();
    if let Some(window) = app.get_webview_window(OVERLAY) {
        let _ = window.destroy();
    }
}

enum Input {
    Open(i32, i32, i32),
    Move(i32),
    Key(&'static str),
    Pointer(i32, i32),
}

struct Shortcut {
    thread_id: u32,
    worker: Option<thread::JoinHandle<()>>,
}

struct InputHandler {
    app: AppHandle,
    main_window: isize,
    active: Cell<bool>,
    release_timer: Cell<usize>,
    wheel_delta: Cell<i32>,
}

impl InputHandler {
    fn send(&self, input: Input) {
        let app = self.app.clone();
        let _ = self.app.run_on_main_thread(move || {
            let _ = app.state::<DesktopWheel>().input(&app, input);
        });
    }

    fn finish(&self, key: &'static str) {
        if !self.active.replace(false) {
            return;
        }
        let timer = self.release_timer.replace(0);
        if timer != 0 {
            unsafe { KillTimer(null_mut(), timer) };
        }
        self.send(Input::Key(key));
    }
}

thread_local! {
    static HANDLER: RefCell<Option<InputHandler>> = const { RefCell::new(None) };
}

impl Shortcut {
    fn start(app: AppHandle) -> Result<Self, String> {
        let main_window = app
            .get_window("main")
            .ok_or("The nabnab window is unavailable.")?
            .hwnd()
            .map_err(|error| error.to_string())?
            .0 as isize;
        let (send, receive) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || unsafe {
            let mut message = MSG::default();
            PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
            let module = GetModuleHandleW(std::ptr::null());
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_input), module, 0);
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_input), module, 0);
            if !mouse.is_null() && !keyboard.is_null() {
                HANDLER.set(Some(InputHandler {
                    app,
                    main_window,
                    active: Cell::new(false),
                    release_timer: Cell::new(0),
                    wheel_delta: Cell::new(0),
                }));
                if send.send(Some(GetCurrentThreadId())).is_ok() {
                    while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                        if message.message == WM_TIMER {
                            HANDLER.with_borrow(|handler| {
                                if let Some(handler) = handler.as_ref() {
                                    if message.wParam == handler.release_timer.get()
                                        && !down(VK_MENU)
                                    {
                                        handler.wheel_delta.set(0);
                                        handler.finish("commit");
                                    }
                                }
                            });
                        }
                    }
                }
            } else {
                let _ = send.send(None);
            }
            if !mouse.is_null() {
                UnhookWindowsHookEx(mouse);
            }
            if !keyboard.is_null() {
                UnhookWindowsHookEx(keyboard);
            }
        });
        let thread_id = receive
            .recv()
            .ok()
            .flatten()
            .ok_or("Windows could not enable desktop Quick Skip.")?;
        Ok(Self {
            thread_id,
            worker: Some(worker),
        })
    }
}

impl Drop for Shortcut {
    fn drop(&mut self) {
        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn down(key: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(key)) < 0 }
}

unsafe extern "system" fn mouse_input(code: i32, message: WPARAM, value: LPARAM) -> LRESULT {
    let message = message as u32;
    if code >= 0
        && matches!(
            message,
            WM_MOUSEWHEEL | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
        )
    {
        let handled = HANDLER.with_borrow(|slot| {
            let Some(handler) = slot.as_ref() else {
                return false;
            };
            let mouse = unsafe { &*(value as *const MSLLHOOKSTRUCT) };
            let POINT { x, y } = mouse.pt;
            if message != WM_MOUSEWHEEL {
                if handler.active.get() {
                    handler.send(Input::Pointer(x, y));
                }
                return false;
            }
            if !down(VK_MENU)
                || down(VK_CONTROL)
                || down(VK_LWIN)
                || down(VK_RWIN)
                // The full app can display the wheel inline. The floating mini uses the desktop overlay.
                || (!handler.active.get() && unsafe { GetForegroundWindow() } as isize == handler.main_window)
            {
                return false;
            }
            let delta = handler.wheel_delta.get() - (mouse.mouseData >> 16) as i16 as i32;
            let step = delta / WHEEL_DELTA as i32;
            handler.wheel_delta.set(delta % WHEEL_DELTA as i32);
            if step != 0 {
                if handler.active.replace(true) {
                    handler.send(Input::Move(step));
                } else {
                    // Reconcile the gesture with Alt's state if Windows omits the key-up notification.
                    handler.release_timer.set(unsafe { SetTimer(null_mut(), 0, 40, None) });
                    handler.send(Input::Open(x, y, step));
                }
            }
            true
        });
        if handled {
            return 1;
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, message as WPARAM, value) }
}

unsafe extern "system" fn key_input(code: i32, message: WPARAM, value: LPARAM) -> LRESULT {
    let message = message as u32;
    if code >= 0 {
        let handled = HANDLER.with_borrow(|slot| {
            let Some(handler) = slot.as_ref() else {
                return false;
            };
            let release = matches!(message, WM_KEYUP | WM_SYSKEYUP);
            let key = unsafe { (*(value as *const KBDLLHOOKSTRUCT)).vkCode as u16 };
            if matches!(key, VK_MENU | VK_LMENU | VK_RMENU) {
                if release
                    && !(key == VK_LMENU && down(VK_RMENU) || key == VK_RMENU && down(VK_LMENU))
                {
                    handler.wheel_delta.set(0);
                    handler.finish("commit");
                }
                return false;
            }
            if !handler.active.get() {
                return false;
            }
            if release {
                return false;
            }
            match key {
                VK_UP => handler.send(Input::Move(-1)),
                VK_DOWN => handler.send(Input::Move(1)),
                VK_SPACE => handler.send(Input::Key("toggle")),
                VK_RETURN => handler.finish("commit"),
                VK_ESCAPE => handler.finish("cancel"),
                _ => {
                    handler.finish("cancel");
                    return false;
                }
            }
            true
        });
        if handled {
            return 1;
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, message as WPARAM, value) }
}
