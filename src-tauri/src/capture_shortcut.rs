use std::{sync::mpsc, thread};

pub struct Shortcut {
    pub label: String,
    thread_id: u32,
    worker: Option<thread::JoinHandle<()>>,
}
impl Shortcut {
    #[cfg(windows)]
    pub fn start(callback: impl Fn() + Send + 'static) -> Result<Self, String> {
        use windows_sys::Win32::{System::Threading::GetCurrentThreadId, UI::{Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT}, WindowsAndMessaging::{GetMessageW, PeekMessageW, MSG, PM_NOREMOVE, WM_HOTKEY}}};
        let (sender, ready) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut message = MSG::default();
            unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE); }
            let mut label = None;
            for (key, name) in [(0x77, "Ctrl+Shift+F8"), (0x78, "Ctrl+Shift+F9"), (0x79, "Ctrl+Shift+F10")] {
                if unsafe { RegisterHotKey(std::ptr::null_mut(), 0x4e42, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, key) } != 0 {
                    label = Some(name.to_owned());
                    break;
                }
            }
            let Some(label) = label else { let _ = sender.send(Err("The replay shortcut is in use by another app. Save from the visible capture control or release Ctrl+Shift+F8, F9 or F10.".to_owned())); return; };
            if sender.send(Ok((unsafe { GetCurrentThreadId() }, label))).is_err() {
                unsafe { UnregisterHotKey(std::ptr::null_mut(), 0x4e42); }
                return;
            }
            while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
                if message.message == WM_HOTKEY && message.wParam == 0x4e42 { callback(); }
            }
            unsafe { UnregisterHotKey(std::ptr::null_mut(), 0x4e42); }
        });
        let (thread_id, label) = ready.recv().map_err(|_| "The replay shortcut could not be registered.")??;
        Ok(Self { label, thread_id, worker: Some(worker) })
    }
    #[cfg(not(windows))]
    pub fn start(_callback: impl Fn() + Send + 'static) -> Result<Self, String> {
        Err("Global replay shortcuts require Windows.".into())
    }
    pub fn stop(&mut self) {
        #[cfg(windows)]
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(self.thread_id, windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT, 0, 0); }
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}
impl Drop for Shortcut { fn drop(&mut self) { self.stop(); } }

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Registers and releases an OS hotkey; run deliberately in a desktop session."]
    fn native_replay_shortcut_dispatches_only_its_registered_message_and_releases_the_key() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_HOTKEY};
        let (sender, events) = mpsc::channel();
        let shortcut = Shortcut::start(move || { let _ = sender.send(()); }).unwrap();
        unsafe { PostThreadMessageW(shortcut.thread_id, WM_HOTKEY, 0x4e42, 0); }
        events.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        drop(shortcut);
        let replacement = Shortcut::start(|| {}).unwrap();
        assert!(replacement.label.starts_with("Ctrl+Shift+F"));
    }
}
