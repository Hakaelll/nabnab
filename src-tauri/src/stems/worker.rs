#![cfg(windows)]

use super::{component, SAMPLE_RATE, STEMS};
use std::{
    ffi::{c_char, c_void, CString, OsString},
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr, slice,
};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LOAD_LIBRARY_SEARCH_SYSTEM32,
    },
};

#[repr(C)]
struct Config {
    vocals: *const c_char,
    accompaniment: *const c_char,
    uvr: *const c_char,
    threads: i32,
    debug: i32,
    provider: *const c_char,
}

#[repr(C)]
struct Stem {
    samples: *const *const f32,
    channels: i32,
    frames: i32,
}

#[repr(C)]
struct Output {
    stems: *const Stem,
    count: i32,
    sample_rate: i32,
}

type Create = unsafe extern "C" fn(*const Config) -> *const c_void;
type Destroy = unsafe extern "C" fn(*const c_void);
type Process =
    unsafe extern "C" fn(*const c_void, *const *const f32, i32, i32, i32) -> *const Output;
type DestroyOutput = unsafe extern "C" fn(*const Output);

struct Library(HMODULE);

impl Drop for Library {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.0) };
    }
}

struct Engine {
    handle: *const c_void,
    destroy: Destroy,
    process: Process,
    destroy_output: DestroyOutput,
    _library: Library,
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.handle) };
    }
}

impl Engine {
    fn load(root: &Path) -> Result<Self, String> {
        let path: Vec<u16> = root
            .join(component::LIBRARY)
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
            return Err(format!(
                "Could not load Spleeter. Check the Microsoft Visual C++ Runtime: {}",
                std::io::Error::last_os_error()
            ));
        }
        let library = Library(module);
        let symbol = |name: &[u8]| {
            unsafe { GetProcAddress(library.0, name.as_ptr()) }
                .ok_or("The Spleeter runtime has an incompatible API.")
        };
        // These signatures match the C ABI in the pinned sherpa-onnx 2.0.1 header.
        let (create, destroy, process, destroy_output): (Create, Destroy, Process, DestroyOutput) = unsafe {
            (
                std::mem::transmute(symbol(b"SherpaOnnxCreateOfflineSourceSeparation\0")?),
                std::mem::transmute(symbol(b"SherpaOnnxDestroyOfflineSourceSeparation\0")?),
                std::mem::transmute(symbol(b"SherpaOnnxOfflineSourceSeparationProcess\0")?),
                std::mem::transmute(symbol(b"SherpaOnnxDestroySourceSeparationOutput\0")?),
            )
        };
        let model = |name| {
            CString::new(root.join(name).to_str().ok_or("Invalid model path.")?)
                .map_err(|_| "Invalid model path.")
        };
        let vocals = model(component::VOCALS)?;
        let accompaniment = model(component::INSTRUMENTAL)?;
        let config = Config {
            vocals: vocals.as_ptr(),
            accompaniment: accompaniment.as_ptr(),
            uvr: ptr::null(),
            threads: 1,
            debug: 0,
            provider: c"cpu".as_ptr(),
        };
        let handle = unsafe { create(&config) };
        if handle.is_null() {
            return Err("Spleeter could not open the verified models.".into());
        }
        Ok(Self {
            handle,
            destroy,
            process,
            destroy_output,
            _library: library,
        })
    }

    fn separate(&self, stereo: &[Vec<f32>; 2]) -> Result<Separated, String> {
        let input = [stereo[0].as_ptr(), stereo[1].as_ptr()];
        let output = unsafe {
            (self.process)(
                self.handle,
                input.as_ptr(),
                2,
                stereo[0].len() as i32,
                SAMPLE_RATE as i32,
            )
        };
        if output.is_null() {
            return Err("Spleeter returned no audio.".into());
        }
        Ok(Separated {
            output,
            destroy: self.destroy_output,
        })
    }
}

struct Separated {
    output: *const Output,
    destroy: DestroyOutput,
}

impl Drop for Separated {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.output) };
    }
}

impl Separated {
    fn tracks(&self, frames: usize) -> Result<[[&[f32]; 2]; 2], String> {
        let output = unsafe { &*self.output };
        if output.sample_rate != SAMPLE_RATE as i32
            || output.count != STEMS.len() as i32
            || output.stems.is_null()
        {
            return Err("Spleeter returned an unexpected track layout.".into());
        }
        let stems = unsafe { slice::from_raw_parts(output.stems, STEMS.len()) };
        let mut tracks = [[&[][..]; 2]; 2];
        for (track, stem) in tracks.iter_mut().zip(stems) {
            if stem.channels != 2 || stem.frames < frames as i32 || stem.samples.is_null() {
                return Err("Spleeter did not preserve the segment alignment.".into());
            }
            for (index, channel) in track.iter_mut().enumerate() {
                let samples = unsafe { *stem.samples.add(index) };
                if samples.is_null() {
                    return Err("Spleeter returned an empty audio channel.".into());
                }
                *channel = unsafe { slice::from_raw_parts(samples, frames) };
            }
        }
        Ok(tracks)
    }
}

const BLOCK: usize = 20 * SAMPLE_RATE;
const CONTEXT: usize = SAMPLE_RATE;
const BLEND: usize = 2_048;
const SPECTRAL_TAIL: usize = 4_096;
const FRAME_BYTES: usize = 8;

pub fn run(args: &[OsString]) -> Result<(), String> {
    let [runtime, mix, directory] = args else {
        return Err("Invalid separation worker arguments.".into());
    };
    let runtime = crate::library::authorized_path(Path::new(runtime))?;
    let mix = crate::library::authorized_path(Path::new(mix))?;
    let directory = crate::library::authorized_path(Path::new(directory))?;
    let length = crate::portable_zip::regular_file(&mix)?;
    if length == 0
        || length % FRAME_BYTES as u64 != 0
        || length / FRAME_BYTES as u64 > super::MAX_FRAMES as u64
    {
        return Err("Invalid separation input length.".into());
    }
    let frames = length as usize / FRAME_BYTES;
    component::verify(&runtime)?;
    let engine = Engine::load(&runtime)?;
    let mut input = File::open(mix).map_err(|error| error.to_string())?;
    let mut outputs = STEMS
        .iter()
        .map(|stem| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(format!("{stem}.f32")))
                .map(BufWriter::new)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut previous = [[Vec::new(), Vec::new()], [Vec::new(), Vec::new()]];
    for start in (0..frames).step_by(BLOCK) {
        let end = (start + BLOCK).min(frames);
        let left = start.saturating_sub(CONTEXT);
        let right = (end + CONTEXT).min(frames);
        let mut bytes = vec![0; (right - left) * FRAME_BYTES];
        input
            .seek(SeekFrom::Start((left * FRAME_BYTES) as u64))
            .and_then(|_| input.read_exact(&mut bytes))
            .map_err(|error| error.to_string())?;
        let mut stereo = [
            Vec::with_capacity(right - left),
            Vec::with_capacity(right - left),
        ];
        for frame in bytes.chunks_exact(FRAME_BYTES) {
            for (channel, raw) in stereo.iter_mut().zip(frame.chunks_exact(4)) {
                let value = f32::from_le_bytes(raw.try_into().unwrap());
                channel.push(if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                });
            }
        }
        // Inverse STFT emits complete hops. Give it a silent tail before cropping to the input.
        for channel in &mut stereo {
            channel.resize(channel.len() + SPECTRAL_TAIL, 0.0);
        }
        let separated = engine.separate(&stereo)?;
        let tracks = separated.tracks(right - left)?;
        for ((output, channels), tails) in outputs.iter_mut().zip(tracks).zip(&mut previous) {
            for frame in start..end {
                for (samples, tail) in channels.iter().zip(tails.iter()) {
                    let value = samples[frame - left];
                    let value = if let Some(previous) = tail.get(frame - start) {
                        let blend = (frame - start + 1) as f32 / (tail.len() + 1) as f32;
                        previous * (1.0 - blend) + value * blend
                    } else {
                        value
                    };
                    if !value.is_finite() {
                        return Err("Spleeter returned invalid audio samples.".into());
                    }
                    output
                        .write_all(&value.to_le_bytes())
                        .map_err(|error| error.to_string())?;
                }
            }
            for (samples, tail) in channels.iter().zip(tails) {
                tail.clear();
                tail.extend_from_slice(&samples[end - left..(end + BLEND).min(right) - left]);
            }
        }
    }
    for mut output in outputs {
        output.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}
