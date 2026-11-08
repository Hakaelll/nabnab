use crate::AppState;
use std::sync::Arc;
use tauri::{ipc::Response, AppHandle, State};

#[cfg(windows)]
mod embedded;

#[derive(serde::Serialize)]
pub struct EmbeddedTrack {
    index: u32,
    codec: &'static str,
    title: Option<String>,
    language: Option<String>,
}

#[cfg(windows)]
const MAX_SCRIPT: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Geometry {
    width: u16,
    height: u16,
    source_width: u16,
    source_height: u16,
    size: u8,
    lift: u8,
}

#[derive(serde::Deserialize)]
pub struct StyledFrame {
    id: String,
    time: u32,
    geometry: Geometry,
    force: bool,
}

#[cfg(windows)]
mod native {
    use super::{Geometry, StyledFrame, MAX_SCRIPT};
    use std::{
        ffi::{c_char, c_void},
        fs::File,
        io::Read,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        ptr, slice,
        sync::{mpsc, OnceLock},
    };
    use tokio::sync::oneshot;
    use windows_sys::Win32::{
        Foundation::{FreeLibrary, HMODULE},
        System::LibraryLoader::{
            GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        },
    };

    const MAX_AREA: usize = 480_000;
    type Handle = *mut c_void;
    type Create = unsafe extern "C" fn() -> Handle;
    type RendererInit = unsafe extern "C" fn(Handle) -> Handle;
    type Destroy = unsafe extern "C" fn(Handle);
    type ReadTrack = unsafe extern "C" fn(Handle, *mut c_char, usize, *const c_char) -> Handle;
    type SetSize = unsafe extern "C" fn(Handle, i32, i32);
    type SetFonts =
        unsafe extern "C" fn(Handle, *const c_char, *const c_char, i32, *const c_char, i32);
    type SetFlag = unsafe extern "C" fn(Handle, i32);
    type SetScale = unsafe extern "C" fn(Handle, f64);
    type Render = unsafe extern "C" fn(Handle, Handle, i64, *mut i32) -> *const Image;

    // libass 0.17, ABI 9. The bitmap length excludes padding after its last row.
    #[repr(C)]
    struct Image {
        width: i32,
        height: i32,
        stride: i32,
        bitmap: *const u8,
        rgba: u32,
        x: i32,
        y: i32,
        next: *const Image,
    }

    struct Library(HMODULE);
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe { FreeLibrary(self.0) };
        }
    }

    struct Renderer {
        id: String,
        context: Handle,
        renderer: Handle,
        track: Handle,
        destroy_context: Destroy,
        destroy_renderer: Destroy,
        destroy_track: Destroy,
        set_size: SetSize,
        set_storage: SetSize,
        set_scale: SetScale,
        set_line_position: SetScale,
        render: Render,
        geometry: Geometry,
        _library: Library,
    }

    impl Drop for Renderer {
        fn drop(&mut self) {
            unsafe {
                if !self.track.is_null() {
                    (self.destroy_track)(self.track);
                }
                if !self.renderer.is_null() {
                    (self.destroy_renderer)(self.renderer);
                }
                if !self.context.is_null() {
                    (self.destroy_context)(self.context);
                }
            }
        }
    }

    impl Renderer {
        fn open(root: &Path, id: String, script: &mut [u8]) -> Result<Self, String> {
            let path: Vec<_> = root
                .join("ass-9.dll")
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let module = unsafe {
                LoadLibraryExW(
                    path.as_ptr(),
                    ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
            };
            if module.is_null() {
                return Err("The bundled libass subtitle renderer is unavailable.".into());
            }
            let library = Library(module);
            let symbol = |name: &[u8]| {
                unsafe { GetProcAddress(library.0, name.as_ptr()) }
                    .ok_or("The bundled libass subtitle renderer has an incompatible API.")
            };
            macro_rules! load {
                ($name:literal, $kind:ty) => {{
                    // The symbols and signatures match the libass ABI declared above.
                    unsafe {
                        std::mem::transmute::<unsafe extern "system" fn() -> isize, $kind>(symbol(
                            concat!($name, "\0").as_bytes(),
                        )?)
                    }
                }};
            }
            let create = load!("ass_library_init", Create);
            let init_renderer = load!("ass_renderer_init", RendererInit);
            let read_track = load!("ass_read_memory", ReadTrack);
            let set_fonts = load!("ass_set_fonts", SetFonts);
            let extract_fonts = load!("ass_set_extract_fonts", SetFlag);
            let limit_cache = load!("ass_set_cache_limits", SetSize);
            let mut engine = Self {
                id,
                context: ptr::null_mut(),
                renderer: ptr::null_mut(),
                track: ptr::null_mut(),
                destroy_context: load!("ass_library_done", Destroy),
                destroy_renderer: load!("ass_renderer_done", Destroy),
                destroy_track: load!("ass_free_track", Destroy),
                set_size: load!("ass_set_frame_size", SetSize),
                set_storage: load!("ass_set_storage_size", SetSize),
                set_scale: load!("ass_set_font_scale", SetScale),
                set_line_position: load!("ass_set_line_position", SetScale),
                render: load!("ass_render_frame", Render),
                geometry: Geometry::default(),
                _library: library,
            };
            unsafe {
                engine.context = create();
                if engine.context.is_null() {
                    return Err("Could not start libass.".into());
                }
                extract_fonts(engine.context, 0);
                engine.track = read_track(
                    engine.context,
                    script.as_mut_ptr().cast(),
                    script.len(),
                    ptr::null(),
                );
                if engine.track.is_null() {
                    return Err("Could not read the ASS/SSA subtitle track.".into());
                }
                engine.renderer = init_renderer(engine.context);
                if engine.renderer.is_null() {
                    return Err("Could not start the styled subtitle renderer.".into());
                }
                limit_cache(engine.renderer, 1024, 16);
                // Provider 1 selects an available system font provider, including DirectWrite.
                set_fonts(
                    engine.renderer,
                    ptr::null(),
                    c"Arial".as_ptr(),
                    1,
                    ptr::null(),
                    0,
                );
            }
            Ok(engine)
        }

        fn frame(&mut self, request: StyledFrame) -> Result<Vec<u8>, String> {
            let geometry = request.geometry;
            let changed_size = geometry != self.geometry;
            if changed_size {
                unsafe {
                    (self.set_size)(
                        self.renderer,
                        i32::from(geometry.width),
                        i32::from(geometry.height),
                    );
                    (self.set_storage)(
                        self.renderer,
                        i32::from(geometry.source_width),
                        i32::from(geometry.source_height),
                    );
                    (self.set_scale)(self.renderer, [0.8, 1.0, 1.25][usize::from(geometry.size)]);
                    (self.set_line_position)(self.renderer, f64::from(geometry.lift));
                }
                self.geometry = geometry;
            }
            let mut change = 0;
            let image = unsafe {
                (self.render)(
                    self.renderer,
                    self.track,
                    i64::from(request.time),
                    &mut change,
                )
            };
            if change == 0 && !changed_size && !request.force {
                return Ok(vec![0]);
            }
            composite(image, i32::from(geometry.width), i32::from(geometry.height))
        }
    }

    fn composite(mut pointer: *const Image, width: i32, height: i32) -> Result<Vec<u8>, String> {
        let mut images = Vec::new();
        let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
        let mut work = 0usize;
        while !pointer.is_null() {
            let image = unsafe { &*pointer };
            pointer = image.next;
            if image.width == 0 || image.height == 0 {
                continue;
            }
            if image.x < 0
                || image.y < 0
                || image.width < 0
                || image.height < 0
                || image.x > width - image.width
                || image.y > height - image.height
                || image.stride < image.width
                || image.stride > 16384
                || image.bitmap.is_null()
            {
                return Err("Invalid subtitle bitmap.".into());
            }
            work += (image.width * image.height) as usize;
            if work > 12 * MAX_AREA || images.len() >= 4096 {
                return Err("The subtitle frame exceeds the render limit.".into());
            }
            left = left.min(image.x);
            top = top.min(image.y);
            right = right.max(image.x + image.width);
            bottom = bottom.max(image.y + image.height);
            images.push(image);
        }
        if images.is_empty() {
            return Ok(Vec::new());
        }
        let (stride, rows) = ((right - left) as usize, (bottom - top) as usize);
        let mut output = Vec::with_capacity(8 + stride * rows * 4);
        for value in [left, top, stride as i32, rows as i32] {
            output.extend_from_slice(&(value as u16).to_le_bytes());
        }
        output.resize(8 + stride * rows * 4, 0);
        for image in images {
            let [red, green, blue, transparency] = image.rgba.to_be_bytes();
            let color = [red, green, blue].map(u32::from);
            for y in 0..image.height as usize {
                let row = unsafe {
                    slice::from_raw_parts(
                        image.bitmap.add(y * image.stride as usize),
                        image.width as usize,
                    )
                };
                let start =
                    8 + 4 * (((image.y - top) as usize + y) * stride + (image.x - left) as usize);
                let pixels = &mut output[start..start + row.len() * 4];
                for (mask, pixel) in row.iter().zip(pixels.chunks_exact_mut(4)) {
                    let alpha = (u32::from(*mask) * u32::from(255 - transparency) + 127) / 255;
                    let remaining = 255 - alpha;
                    for channel in 0..3 {
                        pixel[channel] =
                            ((color[channel] * alpha + u32::from(pixel[channel]) * remaining + 127)
                                / 255) as u8;
                    }
                    pixel[3] = (alpha + (u32::from(pixel[3]) * remaining + 127) / 255) as u8;
                }
            }
        }
        for pixel in output[8..].chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            if alpha > 0 {
                for channel in &mut pixel[..3] {
                    *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
        Ok(output)
    }

    pub(super) enum Source {
        File(PathBuf),
        Embedded(Vec<u8>),
    }

    enum Message {
        Open {
            id: String,
            root: PathBuf,
            source: Source,
            reply: oneshot::Sender<Result<(), String>>,
        },
        Frame(StyledFrame, oneshot::Sender<Result<Vec<u8>, String>>),
        Close(String),
    }

    fn read(path: &Path) -> Result<Vec<u8>, String> {
        let path = crate::library::authorized_path(path)?;
        if !path.is_file()
            || !path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("ass") || ext.eq_ignore_ascii_case("ssa")
            })
        {
            return Err("Choose a local ASS or SSA subtitle file.".into());
        }
        let mut script = Vec::new();
        File::open(path)
            .and_then(|file| file.take(MAX_SCRIPT as u64 + 1).read_to_end(&mut script))
            .map_err(|_| "Could not read the styled subtitle file.")?;
        Ok(script)
    }

    fn worker() -> &'static mpsc::SyncSender<Message> {
        static WORKER: OnceLock<mpsc::SyncSender<Message>> = OnceLock::new();
        WORKER.get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel(4);
            std::thread::spawn(move || {
                // All libass resources stay on this thread, including the system font provider.
                let mut active: Option<Renderer> = None;
                for message in receiver {
                    match message {
                        Message::Open {
                            id,
                            root,
                            source,
                            reply,
                        } => {
                            let script = match source {
                                Source::File(path) => read(&path),
                                Source::Embedded(script) => Ok(script),
                            };
                            let result = script.and_then(|mut script| {
                                if script.len() > MAX_SCRIPT
                                    || script.contains(&0)
                                    || std::str::from_utf8(&script).is_err()
                                {
                                    return Err(
                                        "The ASS/SSA track must be UTF-8 and at most 2 MiB.".into(),
                                    );
                                }
                                active = Some(Renderer::open(&root, id, &mut script)?);
                                Ok(())
                            });
                            let _ = reply.send(result);
                        }
                        Message::Frame(frame, reply) => {
                            let result = active
                                .as_mut()
                                .filter(|renderer| renderer.id == frame.id)
                                .ok_or_else(|| {
                                    "The styled subtitle track is no longer open.".to_string()
                                })
                                .and_then(|renderer| renderer.frame(frame));
                            let _ = reply.send(result);
                        }
                        Message::Close(id) => {
                            if active.as_ref().is_some_and(|renderer| renderer.id == id) {
                                active = None;
                            }
                        }
                    }
                }
            });
            sender
        })
    }

    pub(super) async fn open(id: String, root: PathBuf, source: Source) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        worker()
            .try_send(Message::Open {
                id,
                root,
                source,
                reply,
            })
            .map_err(|_| "The subtitle renderer is busy.")?;
        result.await.map_err(|_| "The subtitle renderer stopped.")?
    }

    pub(super) async fn frame(frame: StyledFrame) -> Result<Vec<u8>, String> {
        let geometry = frame.geometry;
        if geometry.width == 0
            || geometry.height == 0
            || geometry.width > 960
            || geometry.height > 960
            || geometry.source_width == 0
            || geometry.source_height == 0
            || usize::from(geometry.width) * usize::from(geometry.height) > MAX_AREA
            || geometry.size > 2
            || geometry.lift > 60
        {
            return Err("Invalid subtitle render dimensions.".into());
        }
        let (reply, result) = oneshot::channel();
        worker()
            .try_send(Message::Frame(frame, reply))
            .map_err(|_| "The subtitle renderer is busy.")?;
        result.await.map_err(|_| "The subtitle renderer stopped.")?
    }

    pub(super) fn close(id: String) {
        let _ = worker().try_send(Message::Close(id));
    }
}

#[cfg(windows)]
fn validate_id(id: &str) -> Result<(), String> {
    if id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        Ok(())
    } else {
        Err("Invalid styled subtitle session.".into())
    }
}

#[tauri::command]
pub async fn styled_caption_open(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    path: Option<std::path::PathBuf>,
) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        use tauri_plugin_dialog::DialogExt;
        validate_id(&id)?;
        let root = crate::dependencies::bundled_root(&app)?;
        state
            .styled_caption_request
            .run(&id, async {
                let path = match path {
                    Some(path) => path,
                    None => {
                        let Some(file) = tauri::async_runtime::spawn_blocking(move || {
                            app.dialog()
                                .file()
                                .add_filter("Styled subtitles", &["ass", "ssa"])
                                .blocking_pick_file()
                        })
                        .await
                        .map_err(|_| "Could not open the subtitle file picker.")?
                        else {
                            return Ok(None);
                        };
                        file.into_path()
                            .map_err(|_| "Choose a local ASS or SSA subtitle file.")?
                    }
                };
                let title = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                native::open(id.clone(), root, native::Source::File(path)).await?;
                Ok(Some(title))
            })
            .await
    }
    #[cfg(not(windows))]
    {
        let _ = (app, state, id, path);
        Err("The bundled ASS/SSA renderer is available on Windows.".into())
    }
}

#[tauri::command]
pub async fn styled_caption_tracks(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    video: String,
    id: String,
) -> Result<Vec<EmbeddedTrack>, String> {
    #[cfg(windows)]
    {
        validate_id(&id)?;
        let (root, path) = embedded::input(&app, &state, &video)?;
        state
            .styled_caption_probe
            .run(&id, embedded::bounded(embedded::tracks(&root, &path)))
            .await
    }
    #[cfg(not(windows))]
    {
        let _ = (app, state, video, id);
        Ok(Vec::new())
    }
}

#[tauri::command]
pub async fn styled_caption_embedded(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    video: String,
    id: String,
    index: u32,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        validate_id(&id)?;
        let (root, path) = embedded::input(&app, &state, &video)?;
        let renderer = crate::dependencies::bundled_root(&app)?;
        state
            .styled_caption_request
            .run(&id, async {
                let script = embedded::bounded(embedded::extract(&root, &path, index)).await?;
                native::open(id.clone(), renderer, native::Source::Embedded(script)).await
            })
            .await
    }
    #[cfg(not(windows))]
    {
        let _ = (app, state, video, id, index);
        Err("The bundled ASS/SSA renderer is available on Windows.".into())
    }
}

#[tauri::command]
pub async fn styled_caption_frame(request: StyledFrame) -> Result<Response, String> {
    #[cfg(windows)]
    {
        native::frame(request).await.map(Response::new)
    }
    #[cfg(not(windows))]
    {
        let _ = request;
        Err("The bundled ASS/SSA renderer is available on Windows.".into())
    }
}

#[tauri::command]
pub fn styled_caption_close(state: State<'_, Arc<AppState>>, id: String) {
    state.styled_caption_probe.cancel(&id);
    state.styled_caption_request.cancel(&id);
    #[cfg(windows)]
    native::close(id);
    #[cfg(not(windows))]
    let _ = id;
}
