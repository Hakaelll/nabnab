//! Window-only Windows Graphics Capture. No picker, desktop fallback, or border suppression.
use super::*;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;
static WINDOW_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);

struct WindowCaptureLease;
impl Drop for WindowCaptureLease {
    fn drop(&mut self) {
        WINDOW_CAPTURE_ACTIVE.store(false, Ordering::Release);
    }
}
pub(super) struct Frame {
    pub bytes: Vec<u8>,
    pub epoch_us: i64,
}
pub(super) struct Stream {
    pub frames: mpsc::Receiver<Frame>,
    pub error: Arc<Mutex<Option<String>>>,
    stop: watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Stream {
    pub fn start(target: native::Target, fps: u32) -> Result<Self, String> {
        if WINDOW_CAPTURE_ACTIVE.swap(true, Ordering::AcqRel) {
            return Err("Windows is still closing the previous window capture. Try again when it finishes, or restart nabnab if it stays busy.".into());
        }
        let lease = WindowCaptureLease;
        let (sender, frames) = mpsc::channel(1);
        let (stop, rx) = watch::channel(false);
        let error = Arc::new(Mutex::new(None));
        let failure = error.clone();
        let thread = std::thread::Builder::new()
            .name("nabnab-window-capture".into())
            .spawn(move || {
                let _lease = lease;
                if let Err(e) = capture(target, fps, rx, sender) {
                    *failure.lock().unwrap() = Some(e);
                }
            })
            .map_err(|error| format!("Could not start window capture: {error}"))?;
        Ok(Self {
            frames,
            error,
            stop,
            thread: Some(thread),
        })
    }
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
    pub async fn finish(&mut self) {
        self.stop();
        // An unresponsive WinRT close must not hold the application or encoder open.
        let _ = tokio::time::timeout(Duration::from_secs(1), async {
            while self
                .thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take().filter(|thread| thread.is_finished()) {
            let _ = thread.join();
        }
    }
}
pub(super) async fn one(
    target: native::Target,
    mut cancel: watch::Receiver<bool>,
) -> Result<Frame, String> {
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let mut stream = Stream::start(target, 1)?;
    let frame = tokio::select! {
        frame=stream.frames.recv()=>frame.ok_or_else(||stream.error.lock().unwrap().clone().unwrap_or("Windows Graphics Capture produced no frame.".into())),
        _=tokio::time::sleep(Duration::from_secs(8))=>Err("Windows Graphics Capture did not produce a frame. Check that the selected window is visible and not protected.".into()),
        _=cancel.changed()=>Err(engine::CANCELLED.into()),
    };
    stream.finish().await;
    frame
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::{
        core::{factory, Interface},
        Graphics::{
            Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession},
            DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
            SizeInt32,
        },
        Win32::{
            Foundation::{HMODULE, HWND},
            Graphics::{
                Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL},
                Direct3D11::*,
                Dxgi::{Common::*, IDXGIDevice},
            },
            System::WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
                RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
            },
        },
    };
    struct Apartment;
    impl Apartment {
        fn new() -> Result<Self, String> {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|e| format!("Could not initialize Windows Graphics Capture: {e}"))?;
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }
    struct Capture {
        pool: Direct3D11CaptureFramePool,
        session: GraphicsCaptureSession,
        context: ID3D11DeviceContext,
        staging: ID3D11Texture2D,
        size: SizeInt32,
    }
    impl Drop for Capture {
        fn drop(&mut self) {
            let _ = self.session.Close();
            let _ = self.pool.Close();
        }
    }
    impl Capture {
        fn new(target: &native::Target) -> Result<Self, String> {
            target.validate()?;
            if !GraphicsCaptureSession::IsSupported().map_err(|e| e.to_string())? {
                return Err("Windows Graphics Capture is unsupported on this device. Window capture requires Windows 10 version 1903 or later and a compatible graphics device.".into());
            }
            let interop: IGraphicsCaptureItemInterop =
                factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                    .map_err(|e| e.to_string())?;
            // WinRT can unload the capture DLL while a closed window's callback is still returning.
            use windows_sys::Win32::System::LibraryLoader::{
                GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                GET_MODULE_HANDLE_EX_FLAG_PIN,
            };
            let capture_entry = interop.vtable().CreateForWindow as *const u16;
            let mut module = std::ptr::null_mut();
            if unsafe {
                GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                    capture_entry,
                    &mut module,
                )
            } == 0
            {
                return Err(
                    "Could not keep Windows Graphics Capture available for this session.".into(),
                );
            }
            let item:GraphicsCaptureItem=unsafe{interop.CreateForWindow(HWND(target.hwnd as _))}.map_err(|e|format!("Could not open the explicitly selected window with Windows Graphics Capture: {e}"))?;
            let size = item.Size().map_err(|e| e.to_string())?;
            if size.Width < 2
                || size.Height < 2
                || size.Width as u64 * size.Height as u64 > MAX_PIXELS + 2_000_000
            {
                return Err("The selected window frame exceeds the capture pixel limit.".into());
            }
            let mut device = None;
            let mut context = None;
            let mut feature = D3D_FEATURE_LEVEL::default();
            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut feature),
                    Some(&mut context),
                )
            }
            .map_err(|e| format!("Could not create the graphics capture device: {e}"))?;
            let device = device.ok_or("Windows returned no capture device.")?;
            let context = context.ok_or("Windows returned no capture context.")?;
            let dxgi: IDXGIDevice = device.cast().map_err(|e| e.to_string())?;
            let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
                .and_then(|v| v.cast())
                .map_err(|e| e.to_string())?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )
            .map_err(|e| e.to_string())?;
            let session = pool
                .CreateCaptureSession(&item)
                .map_err(|e| e.to_string())?;
            let description = D3D11_TEXTURE2D_DESC {
                Width: size.Width as u32,
                Height: size.Height as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            unsafe { device.CreateTexture2D(&description, None, Some(&mut staging)) }
                .map_err(|e| e.to_string())?;
            let staging = staging.ok_or("Windows returned no capture readback texture.")?;
            session.StartCapture().map_err(|e| e.to_string())?;
            Ok(Self {
                pool,
                session,
                context,
                staging,
                size,
            })
        }
        fn next(
            &self,
            target: &native::Target,
            clock: (i64, i64),
        ) -> Result<Option<Frame>, String> {
            let frame = match self.pool.TryGetNextFrame() {
                Ok(frame) => frame,
                Err(e) => {
                    // The nullable WinRT result projects as Error(S_OK) before a frame is available.
                    if matches!(e.code().0 as u32, 0 | 0x8000000A | 0x80004003) {
                        return Ok(None);
                    }
                    return Err(format!("Windows Graphics Capture stopped: {e}"));
                }
            };
            let result = (|| {
                let size = frame.ContentSize().map_err(|e| e.to_string())?;
                if size.Width != self.size.Width || size.Height != self.size.Height {
                    return Err("The selected window changed size. Recover the available recording and preview its new size.".into());
                }
                let timestamp = frame
                    .SystemRelativeTime()
                    .map_err(|e| e.to_string())?
                    .Duration;
                let surface = frame.Surface().map_err(|e| e.to_string())?;
                let access: IDirect3DDxgiInterfaceAccess =
                    surface.cast().map_err(|e| e.to_string())?;
                let texture: ID3D11Texture2D =
                    unsafe { access.GetInterface() }.map_err(|e| e.to_string())?;
                unsafe {
                    self.context.CopyResource(&self.staging, &texture);
                }
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                unsafe {
                    self.context
                        .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                }
                .map_err(|e| e.to_string())?;
                let result = (|| {
                    let (left, top) = target.client_offset()?;
                    if left + target.width > size.Width as u32
                        || top + target.height > size.Height as u32
                        || mapped.pData.is_null()
                        || mapped.RowPitch < self.size.Width as u32 * 4
                    {
                        return Err(
                            "The selected window's client geometry changed during capture.".into(),
                        );
                    }
                    let row = target.width as usize * 4;
                    let mut bytes = vec![0u8; row * target.height as usize];
                    for y in 0..target.height as usize {
                        let offset =
                            (top as usize + y) * mapped.RowPitch as usize + left as usize * 4;
                        unsafe {
                            bytes[y * row..(y + 1) * row].copy_from_slice(
                                std::slice::from_raw_parts(
                                    (mapped.pData as *const u8).add(offset),
                                    row,
                                ),
                            );
                        }
                    }
                    Ok(Frame {
                        bytes,
                        epoch_us: clock.0 + timestamp / 10 - clock.1,
                    })
                })();
                unsafe {
                    self.context.Unmap(&self.staging, 0);
                }
                result
            })();
            let _ = frame.Close();
            result.map(Some)
        }
    }
    pub(super) fn capture(
        target: native::Target,
        fps: u32,
        stop: watch::Receiver<bool>,
        sender: mpsc::Sender<Frame>,
    ) -> Result<(), String> {
        let _apartment = Apartment::new()?;
        let capture = Capture::new(&target)?;
        let clock = native::clock()?;
        let mut cached = None;
        let mut first_time = 0;
        let mut next = Instant::now();
        let period = Duration::from_nanos(1_000_000_000 / fps as u64);
        while !*stop.borrow() {
            let remaining = next.saturating_duration_since(Instant::now());
            if cached.is_some() && !remaining.is_zero() {
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
                continue;
            }
            target.validate()?;
            for _ in 0..3 {
                match capture.next(&target, clock)? {
                    Some(frame) => {
                        cached = Some(frame);
                    }
                    None => break,
                }
            }
            if Instant::now() >= next {
                if let Some(frame) = &cached {
                    if first_time == 0 {
                        first_time = frame.epoch_us;
                        next = Instant::now();
                    }
                    let mut frame = Frame {
                        bytes: frame.bytes.clone(),
                        epoch_us: first_time,
                    };
                    let waiting = Instant::now();
                    loop {
                        match sender.try_send(frame) {
                            Ok(()) => break,
                            Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
                            Err(mpsc::error::TrySendError::Full(pending)) => {
                                if *stop.borrow() {
                                    return Ok(());
                                }
                                if waiting.elapsed() > VIDEO_STALL_TIMEOUT {
                                    return Err("The video encoder could not keep up with window capture. Recover the available recording or lower the frame rate.".into());
                                }
                                frame = pending;
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                    next += period;
                    if Instant::now().saturating_duration_since(next) > VIDEO_STALL_TIMEOUT {
                        return Err("The graphics capture clock fell behind. Recover the available recording.".into());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}
#[cfg(windows)]
use win::capture;
#[cfg(not(windows))]
fn capture(
    _: native::Target,
    _: u32,
    _: watch::Receiver<bool>,
    _: mpsc::Sender<Frame>,
) -> Result<(), String> {
    Err("Window capture requires Windows Graphics Capture.".into())
}
