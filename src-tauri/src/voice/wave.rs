use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
pub const SAMPLE_RATE: u32 = 22050;
pub const MAX_SEGMENT: u64 = 16 * 1024 * 1024;
pub const MAX_OUTPUT: u64 = 3 * 1024 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct Wave {
    pub offset: u64,
    pub length: u64,
    pub bytes: u64,
}
impl Wave {
    pub fn duration(&self) -> f64 {
        self.length as f64 / (SAMPLE_RATE as f64 * 2.0)
    }
}
pub fn inspect(path: &Path) -> Result<Wave, String> {
    let path = crate::library::authorized_path(path)?;
    let bytes = fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if !(44..=MAX_SEGMENT).contains(&bytes) {
        return Err("The local speech segment is empty or exceeds 16 MiB.".into());
    }
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"RIFF"
        || &header[8..] != b"WAVE"
        || u32::from_le_bytes(header[4..8].try_into().unwrap()) as u64 + 8 != bytes
    {
        return Err("The speech engine returned an incomplete or unsupported WAV file.".into());
    }
    let mut cursor = 12u64;
    let mut format = false;
    let mut data = None;
    let mut chunks = 0;
    while cursor + 8 <= bytes {
        chunks += 1;
        if chunks > 128 {
            return Err("The speech WAV has excessive metadata chunks.".into());
        }
        file.seek(SeekFrom::Start(cursor))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
        let length = u32::from_le_bytes(chunk[4..].try_into().unwrap()) as u64;
        let start = cursor + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes)
            .ok_or("The speech WAV has a truncated chunk.")?;
        if &chunk[..4] == b"fmt " {
            if format || length < 16 || length > 64 {
                return Err("The speech WAV has an invalid format chunk.".into());
            }
            let mut fmt = [0u8; 16];
            file.read_exact(&mut fmt).map_err(|e| e.to_string())?;
            if u16::from_le_bytes(fmt[0..2].try_into().unwrap()) != 1
                || u16::from_le_bytes(fmt[2..4].try_into().unwrap()) != 1
                || u32::from_le_bytes(fmt[4..8].try_into().unwrap()) != SAMPLE_RATE
                || u32::from_le_bytes(fmt[8..12].try_into().unwrap()) != SAMPLE_RATE * 2
                || u16::from_le_bytes(fmt[12..14].try_into().unwrap()) != 2
                || u16::from_le_bytes(fmt[14..16].try_into().unwrap()) != 16
            {
                return Err("Speech segments must use 22050 Hz, mono, 16-bit PCM.".into());
            }
            format = true;
        }
        if &chunk[..4] == b"data" {
            if data.is_some() || length == 0 || length % 2 != 0 {
                return Err(
                    "The speech WAV contains empty, repeated or unaligned audio data.".into(),
                );
            }
            data = Some((start, length));
        }
        cursor = end + (length % 2);
        if cursor > bytes {
            return Err("The speech WAV is missing chunk padding.".into());
        }
    }
    if !format || cursor != bytes {
        return Err("The speech WAV format or trailing data is invalid.".into());
    }
    let (offset, length) = data.ok_or("The speech WAV contains no audio samples.")?;
    Ok(Wave {
        offset,
        length,
        bytes,
    })
}
pub fn header(length: u64) -> Result<Vec<u8>, String> {
    if length == 0 || length > MAX_OUTPUT || length % 2 != 0 {
        return Err("The narration exceeds its 3 GiB audio limit or is empty.".into());
    }
    let length = u32::try_from(length).map_err(|_| "The narration is too long for WAV.")?;
    let mut output = Vec::with_capacity(44);
    output.extend(b"RIFF");
    output.extend((length + 36).to_le_bytes());
    output.extend(b"WAVEfmt ");
    output.extend(16u32.to_le_bytes());
    output.extend(1u16.to_le_bytes());
    output.extend(1u16.to_le_bytes());
    output.extend(SAMPLE_RATE.to_le_bytes());
    output.extend((SAMPLE_RATE * 2).to_le_bytes());
    output.extend(2u16.to_le_bytes());
    output.extend(16u16.to_le_bytes());
    output.extend(b"data");
    output.extend(length.to_le_bytes());
    Ok(output)
}
pub fn sample_bytes(path: &Path, max_seconds: u32) -> Result<(Vec<u8>, f64, bool), String> {
    let wave = inspect(path)?;
    let limit = u64::from(max_seconds) * u64::from(SAMPLE_RATE) * 2;
    let length = wave.length.min(limit);
    let mut bytes = header(length)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(wave.offset))
        .map_err(|e| e.to_string())?;
    file.take(length)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() != length as usize + 44 {
        return Err("The speech sample changed while being read.".into());
    }
    Ok((
        bytes,
        length as f64 / (f64::from(SAMPLE_RATE) * 2.0),
        length < wave.length,
    ))
}
pub fn copy_samples(
    path: &Path,
    wave: &Wave,
    output: &mut impl Write,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(wave.offset))
        .map_err(|e| e.to_string())?;
    let mut remaining = wave.length;
    let mut buffer = [0u8; 65536];
    while remaining > 0 {
        super::cancelled(cancel)?;
        let wanted = remaining.min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..wanted])
            .map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("A cached speech segment ended before its declared sample count.".into());
        }
        output
            .write_all(&buffer[..read])
            .map_err(|e| e.to_string())?;
        remaining -= read as u64;
    }
    Ok(())
}
