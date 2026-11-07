use super::*;
use std::{
    io::{Seek, SeekFrom},
    thread,
};
#[derive(Clone, Debug)]
pub(super) struct Device {
    pub id: String,
    pub label: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub kind: String,
    pub start_us: i64,
    pub files: Vec<String>,
}
#[derive(Default)]
pub(super) struct Log {
    pub start_us: i64,
    pub files: Vec<String>,
    pub active: Option<String>,
    pub error: Option<String>,
    pub initialized: bool,
}
pub(super) struct Recorder {
    pub log: Arc<Mutex<Log>>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Recorder {
    pub fn start(device: Device, dir: PathBuf, stop: watch::Receiver<bool>) -> Self {
        let log = Arc::new(Mutex::new(Log::default()));
        let output = log.clone();
        let handle = thread::spawn(move || {
            if let Err(e) = capture(device, dir, stop, output.clone()) {
                output.lock().unwrap().error = Some(e);
            }
        });
        Self {
            log,
            handle: Some(handle),
        }
    }
    pub fn finish(&mut self) -> Result<(), String> {
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| "The audio capture thread stopped unexpectedly.".to_string())?;
        }
        Ok(())
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
pub(super) fn snapshot(log: &Arc<Mutex<Log>>, kind: &str, include_active: bool) -> Track {
    let l = log.lock().unwrap();
    let mut files = l.files.clone();
    if include_active {
        if let Some(name) = &l.active {
            files.push(name.clone());
        }
    }
    let first = files.first().and_then(|n| segment_number(n)).unwrap_or(0);
    Track {
        kind: kind.into(),
        start_us: l.start_us + first as i64 * 2_000_000,
        files,
    }
}
pub(super) fn segment_number(name: &str) -> Option<u64> {
    name.rsplit_once('-')?.1.split('.').next()?.parse().ok()
}
pub(super) fn trim(log: &Arc<Mutex<Log>>, dir: &Path, earliest_us: i64) -> Result<(), String> {
    let mut l = log.lock().unwrap();
    let start = l.start_us;
    let mut keep = Vec::new();
    for name in &l.files {
        let index = segment_number(name).ok_or("Invalid audio segment.")?;
        if start + (index as i64 + 1) * 2_000_000 <= earliest_us {
            let p = dir.join(name);
            regular(&p)?;
            fs::remove_file(p).map_err(|e| e.to_string())?;
        } else {
            keep.push(name.clone());
        }
    }
    l.files = keep;
    Ok(())
}
struct Wav {
    file: fs::File,
    bytes: u32,
}
impl Wav {
    fn new(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .read(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        let mut wav = Self { file, bytes: 0 };
        wav.checkpoint()?;
        Ok(wav)
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.file.write_all(bytes).map_err(|e| e.to_string())?;
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u32)
            .ok_or("Audio exceeded WAV bounds.")?;
        Ok(())
    }
    fn checkpoint(&mut self) -> Result<(), String> {
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        let mut h = Vec::new();
        h.extend(b"RIFF");
        h.extend((36 + self.bytes).to_le_bytes());
        h.extend(b"WAVEfmt ");
        h.extend(16u32.to_le_bytes());
        h.extend(1u16.to_le_bytes());
        h.extend(2u16.to_le_bytes());
        h.extend(48000u32.to_le_bytes());
        h.extend(192000u32.to_le_bytes());
        h.extend(4u16.to_le_bytes());
        h.extend(16u16.to_le_bytes());
        h.extend(b"data");
        h.extend(self.bytes.to_le_bytes());
        self.file.write_all(&h).map_err(|e| e.to_string())?;
        self.file
            .seek(SeekFrom::End(0))
            .map_err(|e| e.to_string())?;
        self.file.flush().map_err(|e| e.to_string())?;
        Ok(())
    }
}
struct Segments {
    dir: PathBuf,
    kind: String,
    index: u64,
    frames: u64,
    wav: Option<Wav>,
    log: Arc<Mutex<Log>>,
}
impl Segments {
    fn new(dir: PathBuf, kind: String, log: Arc<Mutex<Log>>) -> Self {
        Self {
            dir,
            kind,
            index: 0,
            frames: 0,
            wav: None,
            log,
        }
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() % 4 != 0 {
            return Err("Invalid audio frame alignment.".into());
        }
        let mut offset = 0;
        while offset < bytes.len() {
            if self.wav.is_none() {
                let name = format!("{}-{:08}.wav", self.kind, self.index);
                self.wav = Some(Wav::new(&self.dir.join(&name))?);
                self.log.lock().unwrap().active = Some(name);
            }
            let available = (96000 - (self.frames % 96000)) as usize * 4;
            let take = available.min(bytes.len() - offset);
            self.wav
                .as_mut()
                .unwrap()
                .write(&bytes[offset..offset + take])?;
            self.frames += (take / 4) as u64;
            offset += take;
            if self.frames % 96000 == 0 {
                self.close()?;
                self.index += 1;
            }
        }
        Ok(())
    }
    fn silence(&mut self, frames: u64) -> Result<(), String> {
        let silence = [0u8; 4096];
        let mut bytes = frames * 4;
        while bytes > 0 {
            let n = bytes.min(silence.len() as u64) as usize;
            self.append(&silence[..n])?;
            bytes -= n as u64;
        }
        Ok(())
    }
    fn close(&mut self) -> Result<(), String> {
        if let Some(mut wav) = self.wav.take() {
            wav.checkpoint()?;
            wav.file.sync_all().map_err(|e| e.to_string())?;
            let mut log = self.log.lock().unwrap();
            if let Some(name) = log.active.take() {
                log.files.push(name);
            }
        }
        Ok(())
    }
}
impl Drop for Segments {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(windows)]
pub(super) fn devices() -> Result<Vec<Device>, String> {
    use wasapi::{deinitialize, initialize_mta, DeviceEnumerator, DeviceState, Direction};
    initialize_mta().ok().map_err(|e| e.to_string())?;
    let result = (|| {
        let e = DeviceEnumerator::new().map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for (direction, kind) in [
            (Direction::Render, "system"),
            (Direction::Capture, "microphone"),
        ] {
            let collection = e
                .get_device_collection(&direction)
                .map_err(|e| e.to_string())?;
            for d in &collection {
                let d = d.map_err(|e| e.to_string())?;
                if matches!(d.get_state(), Ok(DeviceState::Active)) {
                    result.push(Device {
                        id: d.get_id().map_err(|e| e.to_string())?,
                        label: d.get_friendlyname().map_err(|e| e.to_string())?,
                        kind: kind.into(),
                    });
                }
            }
        }
        Ok(result)
    })();
    deinitialize();
    result
}
#[cfg(not(windows))]
pub(super) fn devices() -> Result<Vec<Device>, String> {
    Ok(vec![])
}
#[cfg(windows)]
fn capture(
    device: Device,
    dir: PathBuf,
    stop: watch::Receiver<bool>,
    log: Arc<Mutex<Log>>,
) -> Result<(), String> {
    use wasapi::*;
    initialize_mta().ok().map_err(|e| e.to_string())?;
    let result = (|| {
        let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
        let endpoint = enumerator
            .get_device(&device.id)
            .map_err(|e| e.to_string())?;
        let mut client = endpoint.get_iaudioclient().map_err(|e| e.to_string())?;
        let format = WaveFormat::new(16, 16, &SampleType::Int, 48000, 2, None);
        let mode = StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 2_000_000,
        };
        client
            .initialize_client(&format, &Direction::Capture, &mode)
            .map_err(|e| e.to_string())?;
        // wasapi detects loopback from the eRender endpoint and Direction::Capture initialization.
        let capture: windows::Win32::Media::Audio::IAudioCaptureClient =
            unsafe { client.as_iaudioclient().GetService() }.map_err(|e| e.to_string())?;
        let max = client.get_buffer_size().map_err(|e| e.to_string())? as usize;
        if max == 0 || max > 96000 {
            return Err("The selected audio device returned an unsupported buffer size.".into());
        }
        let mut bytes = vec![0u8; (max + 1) * 4];
        let (epoch, qpc) = native::clock()?;
        client.start_stream().map_err(|e| e.to_string())?;
        {
            let mut l = log.lock().unwrap();
            l.start_us = epoch;
            l.initialized = true;
        }
        let mut segments = Segments::new(dir, device.kind, log.clone());
        let mut checkpoint = Instant::now();
        let mut first = true;
        while !*stop.borrow() {
            if !matches!(endpoint.get_state(), Ok(DeviceState::Active)) {
                return Err("The selected audio device disconnected.".into());
            }
            while unsafe { capture.GetNextPacketSize() }.map_err(|e| e.to_string())? > 0 {
                let (frames, info) = read_packet(&capture, &mut bytes)?;
                if frames == 0 {
                    break;
                }
                if info.flags.timestamp_error || (!first && info.flags.data_discontinuity) {
                    return Err(
                        "Audio timing was interrupted; the available recording is partial.".into(),
                    );
                }
                first = false;
                let packet_us = epoch + (info.timestamp / 10) as i64 - qpc;
                let target = ((packet_us - epoch).max(0) as u64 * 48000) / 1_000_000;
                if target > segments.frames {
                    let gap = target - segments.frames;
                    if gap > 48000 * 10 {
                        return Err("Audio capture fell behind by more than ten seconds.".into());
                    }
                    segments.silence(gap)?;
                }
                let skip = segments.frames.saturating_sub(target).min(frames as u64) as usize;
                segments.append(&bytes[skip * 4..frames as usize * 4])?;
            }
            // A silent render endpoint may deliver no packets. Keep its real timeline without treating silence as device loss.
            let (_, now_qpc) = native::clock()?;
            let target = ((now_qpc - qpc - 200_000).max(0) as u64 * 48000) / 1_000_000;
            if target > segments.frames {
                if target - segments.frames > 48000 * 2 {
                    return Err("Audio capture paused for more than two seconds. Recover the available recording.".into());
                }
                segments.silence(target - segments.frames)?;
            }
            if checkpoint.elapsed() >= Duration::from_millis(200) {
                if let Some(wav) = &mut segments.wav {
                    wav.checkpoint()?;
                }
                checkpoint = Instant::now();
            }
            thread::sleep(Duration::from_millis(10));
        }
        client.stop_stream().map_err(|e| e.to_string())?;
        segments.close()?;
        Ok(())
    })();
    deinitialize();
    result
}
#[cfg(windows)]
fn read_packet(
    capture: &windows::Win32::Media::Audio::IAudioCaptureClient,
    bytes: &mut [u8],
) -> Result<(u32, wasapi::BufferInfo), String> {
    let mut ptr = std::ptr::null_mut();
    let mut frames = 0;
    let mut flags = 0;
    let mut index = 0;
    let mut timestamp = 0;
    unsafe {
        capture.GetBuffer(
            &mut ptr,
            &mut frames,
            &mut flags,
            Some(&mut index),
            Some(&mut timestamp),
        )
    }
    .map_err(|e| e.to_string())?;
    let result = unsafe { copy_packet(ptr, frames, flags & 2 != 0, bytes) };
    let released = unsafe { capture.ReleaseBuffer(frames) }.map_err(|e| e.to_string());
    result?;
    released?;
    Ok((frames, wasapi::BufferInfo::new(flags, index, timestamp)))
}
unsafe fn copy_packet(
    ptr: *const u8,
    frames: u32,
    silent: bool,
    bytes: &mut [u8],
) -> Result<(), String> {
    let size = frames as usize * 4;
    if size > bytes.len() {
        return Err("The audio device returned an oversized packet.".into());
    }
    if silent {
        bytes[..size].fill(0);
    } else if size > 0 {
        if ptr.is_null() {
            return Err("The audio device returned an invalid packet.".into());
        }
        bytes[..size].copy_from_slice(std::slice::from_raw_parts(ptr, size));
    }
    Ok(())
}
#[cfg(not(windows))]
fn capture(
    _: Device,
    _: PathBuf,
    _: watch::Receiver<bool>,
    _: Arc<Mutex<Log>>,
) -> Result<(), String> {
    Err("WASAPI capture requires Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silent_audio_packets_can_have_null_data_and_are_zeroed() {
        let mut bytes = [7u8; 32];
        unsafe {
            copy_packet(std::ptr::null(), 4, true, &mut bytes).unwrap();
            assert_eq!(&bytes[..16], &[0u8; 16]);
            assert_eq!(&bytes[16..], &[7u8; 16]);
            assert!(copy_packet(std::ptr::null(), 2, false, &mut bytes).is_err());
            assert!(copy_packet(std::ptr::null(), 20, true, &mut bytes).is_err());
        }
    }
    #[test]
    fn audio_segments_are_recoverable_stereo_pcm_and_trimmed_as_a_ring() {
        let dir = std::env::temp_dir().join(super::super::id("c"));
        directory(&dir).unwrap();
        let log = Arc::new(Mutex::new(Log {
            start_us: 10_000_000,
            ..Log::default()
        }));
        {
            let mut segments = Segments::new(dir.clone(), "system".into(), log.clone());
            segments.append(&vec![7u8; 192000 * 5]).unwrap();
            segments.close().unwrap();
        }
        let track = snapshot(&log, "system", false);
        assert_eq!(track.files.len(), 3);
        let first = fs::read(dir.join(&track.files[0])).unwrap();
        assert_eq!(&first[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(first[40..44].try_into().unwrap()),
            384000
        );
        assert_eq!(first.len(), 384044);
        trim(&log, &dir, 12_000_000).unwrap();
        let t = snapshot(&log, "system", false);
        assert_eq!(t.files.len(), 2);
        assert_eq!(t.start_us, 12_000_000);
        assert!(!dir.join(&track.files[0]).exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
