use std::{collections::HashSet, fs::{self, File, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}};
use tokio::sync::watch;

const MAX_ENTRIES: usize = 10_000;
const MAX_BYTES: u64 = 128 * 1024 * 1024 * 1024;

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 1024 || name.contains(['\\', ':', '<', '>', '"', '|', '?', '*']) || name.chars().any(char::is_control) {
        return Err("The package contains an unsafe relative filename.".into());
    }
    for part in name.split('/') {
        let stem = part.split('.').next().unwrap_or_default().to_ascii_uppercase();
        if part.is_empty() || matches!(part, "." | "..") || part.ends_with(['.', ' ']) || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit()) {
            return Err("The package contains traversal, an empty component or a reserved filename.".into());
        }
    }
    Ok(())
}
pub fn regular_file(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 { return Err("Packages cannot follow symbolic links or reparse points.".into()); }
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() { return Err("Packages accept regular selected files only.".into()); }
    if metadata.len() > MAX_BYTES { return Err("A package file exceeds the 128 GiB limit.".into()); }
    Ok(metadata.len())
}
fn canceled(cancel: &watch::Receiver<bool>) -> Result<(), String> { if *cancel.borrow() { Err(crate::engine::CANCELLED.into()) } else { Ok(()) } }
const fn crc_table() -> [u32; 256] { let mut table = [0u32; 256]; let mut index = 0; while index < 256 { let mut value = index as u32; let mut bit = 0; while bit < 8 { value = (value >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(value & 1)); bit += 1; } table[index] = value; index += 1; } table }
const CRC_TABLE: [u32; 256] = crc_table();
fn crc_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes { crc = (crc >> 8) ^ CRC_TABLE[((crc ^ u32::from(*byte)) & 255) as usize]; }
    crc
}
fn checksum(path: &Path, expected: u64, cancel: &watch::Receiver<bool>) -> Result<u32, String> {
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 64 * 1024]; let mut total = 0u64; let mut crc = !0u32;
    loop { canceled(cancel)?; let received = input.read(&mut buffer).map_err(|e| e.to_string())?; if received == 0 { break; } total += received as u64; if total > expected { return Err("A selected package file changed while being read.".into()); } crc = crc_update(crc, &buffer[..received]); }
    if total != expected { return Err("A selected package file changed while being read.".into()); }
    Ok(!crc)
}
fn u16_to(bytes: &mut Vec<u8>, value: u16) { bytes.extend_from_slice(&value.to_le_bytes()); }
fn u32_to(bytes: &mut Vec<u8>, value: u32) { bytes.extend_from_slice(&value.to_le_bytes()); }
fn u64_to(bytes: &mut Vec<u8>, value: u64) { bytes.extend_from_slice(&value.to_le_bytes()); }
struct Entry { name: String, size: u64, crc: u32, offset: u64 }
fn local(entry: &Entry) -> Vec<u8> {
    let large = entry.size >= u64::from(u32::MAX); let mut bytes = Vec::new();
    u32_to(&mut bytes, 0x04034b50); u16_to(&mut bytes, if large {45} else {20}); u16_to(&mut bytes, 0x800); u16_to(&mut bytes, 0); u32_to(&mut bytes, 0); u32_to(&mut bytes, entry.crc);
    for _ in 0..2 { u32_to(&mut bytes, if large {u32::MAX} else {entry.size as u32}); }
    u16_to(&mut bytes, entry.name.len() as u16); u16_to(&mut bytes, if large {20} else {0}); bytes.extend_from_slice(entry.name.as_bytes());
    if large { u16_to(&mut bytes, 1); u16_to(&mut bytes, 16); u64_to(&mut bytes, entry.size); u64_to(&mut bytes, entry.size); }
    bytes
}
fn central(entry: &Entry) -> Vec<u8> {
    let large = entry.size >= u64::from(u32::MAX); let far = entry.offset >= u64::from(u32::MAX); let mut extra = Vec::new();
    if large { u64_to(&mut extra, entry.size); u64_to(&mut extra, entry.size); } if far { u64_to(&mut extra, entry.offset); }
    let mut bytes = Vec::new(); u32_to(&mut bytes, 0x02014b50); u16_to(&mut bytes, 45); u16_to(&mut bytes, if large || far {45} else {20}); u16_to(&mut bytes, 0x800); u16_to(&mut bytes, 0); u32_to(&mut bytes, 0); u32_to(&mut bytes, entry.crc);
    for _ in 0..2 { u32_to(&mut bytes, if large {u32::MAX} else {entry.size as u32}); }
    u16_to(&mut bytes, entry.name.len() as u16); u16_to(&mut bytes, if extra.is_empty() {0} else {extra.len() as u16 + 4}); u16_to(&mut bytes, 0); u16_to(&mut bytes, 0); u16_to(&mut bytes, 0); u32_to(&mut bytes, 0); u32_to(&mut bytes, if far {u32::MAX} else {entry.offset as u32}); bytes.extend_from_slice(entry.name.as_bytes());
    if !extra.is_empty() { u16_to(&mut bytes, 1); u16_to(&mut bytes, extra.len() as u16); bytes.extend_from_slice(&extra); } bytes
}
fn write(output: &Path, entries: &[(String, PathBuf)], cancel: watch::Receiver<bool>) -> Result<(), String> {
    if entries.is_empty() || entries.len() > MAX_ENTRIES { return Err("A package must contain 1–10,000 selected files.".into()); }
    let mut names = HashSet::new(); let mut sizes = Vec::new(); let mut total = 0u64;
    for (name, path) in entries { validate_name(name)?; if !names.insert(name.to_lowercase()) { return Err("The package contains colliding relative names.".into()); } let size = regular_file(path)?; total = total.checked_add(size).filter(|value| *value <= MAX_BYTES).ok_or("The package exceeds its 128 GiB budget.")?; sizes.push(size); }
    canceled(&cancel)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(output).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut records = Vec::new(); let mut offset = 0u64;
        for ((name, path), size) in entries.iter().zip(sizes) {
            canceled(&cancel)?; let crc = checksum(path, size, &cancel)?;
            let entry = Entry { name: name.clone(), size, crc, offset }; let header = local(&entry); file.write_all(&header).map_err(|e| e.to_string())?; offset += header.len() as u64;
            let mut input = File::open(path).map_err(|e| e.to_string())?; let mut buffer = [0u8; 64 * 1024]; let mut copied = 0u64; let mut actual = !0u32;
            loop { canceled(&cancel)?; let count = input.read(&mut buffer).map_err(|e| e.to_string())?; if count == 0 {break;} copied += count as u64; if copied > size {return Err("A package input changed. No completed package was published.".into());} actual = crc_update(actual, &buffer[..count]); file.write_all(&buffer[..count]).map_err(|e| e.to_string())?; }
            if copied != size || !actual != crc { return Err("A package input changed. No completed package was published.".into()); }
            offset += copied; records.push(entry);
        }
        let central_offset = offset;
        for entry in &records { let record = central(entry); file.write_all(&record).map_err(|e| e.to_string())?; offset += record.len() as u64; }
        let central_size = offset - central_offset; let large = central_offset >= u64::from(u32::MAX) || central_size >= u64::from(u32::MAX);
        let mut end = Vec::new();
        if large {
            u32_to(&mut end, 0x06064b50); u64_to(&mut end, 44); u16_to(&mut end, 45); u16_to(&mut end, 45); u32_to(&mut end, 0); u32_to(&mut end, 0); u64_to(&mut end, records.len() as u64); u64_to(&mut end, records.len() as u64); u64_to(&mut end, central_size); u64_to(&mut end, central_offset);
            u32_to(&mut end, 0x07064b50); u32_to(&mut end, 0); u64_to(&mut end, offset); u32_to(&mut end, 1);
        }
        u32_to(&mut end, 0x06054b50); u32_to(&mut end, 0); u16_to(&mut end, if large {u16::MAX} else {records.len() as u16}); u16_to(&mut end, if large {u16::MAX} else {records.len() as u16}); u32_to(&mut end, if large {u32::MAX} else {central_size as u32}); u32_to(&mut end, if large {u32::MAX} else {central_offset as u32}); u16_to(&mut end, 0);
        file.write_all(&end).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?; canceled(&cancel)
    })();
    drop(file); if result.is_err() { let _ = fs::remove_file(output); } result
}
pub async fn write_generated(output: &Path, entries: &[(String, PathBuf)], cancel: watch::Receiver<bool>) -> Result<(), String> {
    let output = output.to_owned(); let entries = entries.to_owned(); tokio::task::spawn_blocking(move || write(&output, &entries, cancel)).await.map_err(|_| "The ZIP worker failed.".to_string())?
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn names_reject_traversal_and_windows_collisions() { for name in ["../x", "/root", "a//b", "a\\b", "C:/x", "con.txt", "a/../b", "x.", "x ", "a\0b"] { assert!(validate_name(name).is_err(), "{name}"); } assert!(validate_name("files/á data.png").is_ok()); }
    #[test] fn crc_matches_standard_vector() { assert_eq!(!crc_update(!0u32, b"123456789"), 0xcbf43926); }
    #[test] fn zip64_headers_keep_sizes_and_offsets_above_four_gib() { let entry = Entry { name: "data.bin".into(), size: 4_294_967_300, crc: 0, offset: 4_294_967_400 }; let header = local(&entry); assert_eq!(&header[18..26], &[255; 8]); assert_eq!(&header[header.len()-16..header.len()-8], &4_294_967_300u64.to_le_bytes()); let directory = central(&entry); assert_eq!(&directory[42..46], &[255; 4]); assert_eq!(&directory[directory.len()-8..], &4_294_967_400u64.to_le_bytes()); }
    #[tokio::test] async fn stored_zip_preserves_selected_bytes_and_existing_output() {
        let root = std::env::temp_dir().join(format!("nab-portable-zip-{}-{}", std::process::id(), crate::queue::now())); fs::create_dir(&root).unwrap(); let input = root.join("input"); fs::write(&input, b"123456789").unwrap(); let output = root.join("bundle.zip"); let (_sender, cancel) = watch::channel(false);
        write_generated(&output, &[("files/data.txt".into(), input.clone())], cancel.clone()).await.unwrap(); let bytes = fs::read(&output).unwrap(); assert_eq!(&bytes[..4], &0x04034b50u32.to_le_bytes()); assert_eq!(&bytes[14..18], &0xcbf43926u32.to_le_bytes()); assert!(bytes.windows(9).any(|part| part == b"123456789")); assert!(write_generated(&output, &[("files/data.txt".into(), input.clone())], cancel).await.is_err()); assert_eq!(fs::read(output).unwrap(), bytes); assert_eq!(fs::read(input).unwrap(), b"123456789"); fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test] async fn canceled_zip_is_not_created() { let root = std::env::temp_dir().join(format!("nab-portable-cancel-{}-{}", std::process::id(), crate::queue::now())); fs::create_dir(&root).unwrap(); let input = root.join("input"); fs::write(&input, b"data").unwrap(); let (sender, cancel) = watch::channel(false); sender.send(true).unwrap(); let output = root.join("bundle.zip"); assert!(write_generated(&output, &[("data.txt".into(), input)], cancel).await.is_err()); assert!(!output.exists()); fs::remove_dir_all(root).unwrap(); }
}
