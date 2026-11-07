use crate::providers::{self, ProviderId};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_COOKIE_BYTES: usize = 2 * 1024 * 1024;
const HEADER: &[u8] = b"nabnab-cookie-v1\n";
const URL_HEADER: &[u8] = b"nabnab-operational-url-v1\n";
const BACKUP_HEADER: &[u8] = b"nabnab-repository-backup-v1\n";
const MAX_URL_BYTES: usize = 8192;
const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;
const ARCHIVE_HEADER: &[u8] = b"nabnab-personal-archive-v1\n";
const MAX_ARCHIVE_RECORD_BYTES: usize = 32 * 1024 * 1024;

fn module_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("Invalid protected module name.".into());
    }
    Ok(())
}
pub fn secure_module_directory(root: &Path, name: &str) -> Result<PathBuf, String> {
    module_name(name)?;
    private_directory(root, name)
}
pub fn save_module_secret(root: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    module_name(name)?;
    if bytes.is_empty() || bytes.len() > 65536 {
        return Err("Module secret exceeds its size limit.".into());
    }
    let directory = private_directory(root, "module-secrets")?;
    write_protected(
        &directory.join(format!("{name}.bin")),
        b"nabnab-module-secret-v1\n",
        &format!("nabnab-module:{name}"),
        bytes,
    )
}
pub fn load_module_secret(root: &Path, name: &str) -> Result<Vec<u8>, String> {
    module_name(name)?;
    let secret = read_protected(
        &root.join("module-secrets").join(format!("{name}.bin")),
        b"nabnab-module-secret-v1\n",
        &format!("nabnab-module:{name}"),
        65536,
    )?;
    Ok(secret.0.clone())
}

pub(crate) fn personal_archive_record_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    module_name(name)?;
    Ok(private_directory(root, "personal-archive-data")?.join(format!("{name}.bin")))
}

pub(crate) fn save_personal_archive_record(root: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > MAX_ARCHIVE_RECORD_BYTES {
        return Err("The selected personal archive index exceeds 32 MiB. Import fewer categories or a smaller date range.".into());
    }
    let path = personal_archive_record_path(root, name)?;
    write_protected(&path, ARCHIVE_HEADER, &format!("nabnab-personal-archive:{name}"), bytes)
}

pub(crate) fn load_personal_archive_record(root: &Path, name: &str) -> Result<Vec<u8>, String> {
    let path = personal_archive_record_path(root, name)?;
    read_protected(&path, ARCHIVE_HEADER, &format!("nabnab-personal-archive:{name}"), MAX_ARCHIVE_RECORD_BYTES)
        .map(|secret| secret.0.clone())
}
pub const SITES: [&str; 24] = [
    "youtube",
    "twitter",
    "instagram",
    "reddit",
    "tiktok",
    "vimeo",
    "twitch",
    "pinterest",
    "facebook",
    "bilibili",
    "dailymotion",
    "soundcloud",
    "bandcamp",
    "artstation",
    "deviantart",
    "pixiv",
    "flickr",
    "tumblr",
    "imgur",
    "bluesky",
    "threads",
    "udemy",
    "hotmart",
    "kiwify",
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub site: String,
}

struct Secret(Vec<u8>);
impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}
fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        // Cookie bytes must not be retained by an optimized-away clearing loop.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

pub struct CookieFile {
    pub path: PathBuf,
}
impl Drop for CookieFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn domains(site: &str) -> Result<&'static [&'static str], String> {
    match site {
        "youtube" => Ok(&["youtube.com", "youtube-nocookie.com", "google.com"]),
        "twitter" => Ok(&["x.com", "twitter.com"]),
        "instagram" => Ok(&["instagram.com"]),
        "reddit" => Ok(&["reddit.com", "redd.it"]),
        "tiktok" => Ok(&["tiktok.com"]),
        "vimeo" => Ok(&["vimeo.com"]),
        "twitch" => Ok(&["twitch.tv"]),
        "pinterest" => Ok(&["pinterest.com", "pin.it"]),
        "facebook" => Ok(&["facebook.com", "fb.watch"]),
        "bilibili" => Ok(&["bilibili.com", "b23.tv"]),
        "dailymotion" => Ok(&["dailymotion.com", "dai.ly"]),
        "soundcloud" => Ok(&["soundcloud.com"]),
        "bandcamp" => Ok(&["bandcamp.com"]),
        "artstation" => Ok(&["artstation.com"]),
        "deviantart" => Ok(&["deviantart.com"]),
        "pixiv" => Ok(&["pixiv.net"]),
        "flickr" => Ok(&["flickr.com"]),
        "tumblr" => Ok(&["tumblr.com"]),
        "imgur" => Ok(&["imgur.com"]),
        "bluesky" => Ok(&["bsky.app"]),
        "threads" => Ok(&["threads.net", "threads.com"]),
        "udemy" => Ok(&["udemy.com"]),
        "hotmart" => Ok(&["hotmart.com"]),
        "kiwify" => Ok(&["kiwify.com.br", "kiwify.com"]),
        _ => Err("Choose a supported session site.".into()),
    }
}

fn site_for(url: &str) -> Result<Option<&'static str>, String> {
    Ok(match providers::identify(url)?.id {
        ProviderId::Youtube => Some("youtube"),
        ProviderId::Twitter => Some("twitter"),
        ProviderId::Instagram => Some("instagram"),
        ProviderId::Reddit => Some("reddit"),
        ProviderId::Tiktok => Some("tiktok"),
        ProviderId::Vimeo => Some("vimeo"),
        ProviderId::Twitch => Some("twitch"),
        ProviderId::Pinterest => Some("pinterest"),
        _ => {
            let host = url::Url::parse(url)
                .map_err(|_| "Invalid session URL.")?
                .host_str()
                .unwrap_or_default()
                .to_owned();
            SITES.iter().copied().find(|site| {
                domains(site).is_ok_and(|ds| {
                    ds.iter()
                        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
                })
            })
        }
    })
}

fn validate_cookies(site: &str, bytes: &[u8]) -> Result<(), String> {
    let allowed = domains(site)?;
    if bytes.is_empty() || bytes.len() > MAX_COOKIE_BYTES {
        return Err("Choose a Netscape cookie file of at most 2 MiB.".into());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "The cookie file must contain UTF-8 Netscape cookie text.")?
        .trim_start_matches('\u{feff}');
    if !text.lines().take(3).any(|line| {
        matches!(
            line.trim(),
            "# Netscape HTTP Cookie File" | "# HTTP Cookie File"
        )
    }) {
        return Err("Choose an exported Netscape HTTP cookie file.".into());
    }
    let mut records = 0;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let line = if let Some(cookie) = line.strip_prefix("#HttpOnly_") {
            cookie
        } else if line.starts_with('#') || line.trim().is_empty() {
            continue;
        } else {
            line
        };
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 7 {
            return Err("The cookie file contains an invalid Netscape record.".into());
        }
        let domain = fields[0]
            .strip_prefix('.')
            .unwrap_or(fields[0])
            .to_ascii_lowercase();
        if domain.split('.').any(str::is_empty)
            || !domain
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'-' | b'.'))
        {
            return Err("The cookie file contains an invalid cookie domain.".into());
        }
        if !allowed
            .iter()
            .any(|allowed| domain == *allowed || domain.ends_with(&format!(".{allowed}")))
        {
            return Err("The cookie file contains cookies for another site. Export only the selected site's session.".into());
        }
        if !matches!(fields[1], "TRUE" | "FALSE")
            || !fields[2].starts_with('/')
            || !matches!(fields[3], "TRUE" | "FALSE")
            || fields[4].parse::<u64>().is_err()
            || fields[5].is_empty()
            || fields
                .iter()
                .any(|field| field.chars().any(char::is_control))
        {
            return Err("The cookie file contains an invalid Netscape record.".into());
        }
        records += 1;
    }
    if records == 0 {
        return Err("The cookie file contains no cookies for the selected site.".into());
    }
    Ok(())
}

#[cfg(windows)]
fn crypt(bytes: &[u8], site: &str, decrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    let entropy = format!("nabnab-cookie-v1:{site}");
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 || output.pbData.is_null() {
        return Err("Windows could not protect or unlock this session for the current user. Import it again in this Windows profile.".into());
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        if decrypt {
            wipe(std::slice::from_raw_parts_mut(
                output.pbData,
                output.cbData as usize,
            ));
        }
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

#[cfg(not(windows))]
fn crypt(_: &[u8], _: &str, _: bool) -> Result<Vec<u8>, String> {
    Err("Secure session storage is currently available on Windows only.".into())
}

#[cfg(windows)]
fn restrict_directory(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            GetTokenInformation, SetFileSecurityW, TokenUser, DACL_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err("Windows could not secure the session directory.".into());
    }
    let result = (|| {
        let mut length = 0;
        unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut length) };
        if length == 0 || length > 64 * 1024 {
            return Err("Windows could not identify the current session owner.".into());
        }
        let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            return Err("Windows could not identify the current session owner.".into());
        }
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let mut sid = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
            return Err("Windows could not identify the current session owner.".into());
        }
        let mut sid_length = 0;
        unsafe {
            while *sid.add(sid_length) != 0 {
                sid_length += 1;
            }
        }
        let sid_text =
            unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(sid, sid_length)) };
        unsafe {
            LocalFree(sid.cast());
        }
        let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;{sid_text})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err("Windows could not construct private session permissions.".into());
        }
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let ok = unsafe {
            SetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            )
        };
        unsafe {
            LocalFree(descriptor);
        }
        if ok == 0 {
            return Err(
                "Windows could not apply private session permissions. No cookies were written."
                    .into(),
            );
        }
        Ok(())
    })();
    unsafe {
        CloseHandle(token);
    }
    result
}

#[cfg(not(windows))]
fn restrict_directory(_: &Path) -> Result<(), String> {
    Err("Secure session storage is currently available on Windows only.".into())
}

fn private_directory(root: &Path, name: &str) -> Result<PathBuf, String> {
    let directory = root.join(name);
    fs::create_dir_all(&directory)
        .map_err(|_| "Could not create the private session directory.")?;
    crate::library::authorized_path(&directory)?;
    restrict_directory(&directory)?;
    Ok(directory)
}

fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn replace(from: &Path, to: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                0x1 | 0x8,
            )
        } == 0
        {
            return Err(
                "Could not commit the protected session. The previous session was kept.".into(),
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    fs::rename(from, to).map_err(|_| "Could not commit the protected session.".into())
}

pub struct OperationalUrl(Secret);
impl OperationalUrl {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0 .0).expect("operational URL validated when unlocked")
    }
}

pub fn valid_url_reference(reference: &str) -> bool {
    reference.len() == 64
        && reference
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn validate_operational_url(bytes: &[u8]) -> Result<&str, String> {
    if bytes.is_empty() || bytes.len() > MAX_URL_BYTES {
        return Err("Use a download URL of at most 8192 bytes.".into());
    }
    let value = std::str::from_utf8(bytes).map_err(|_| "The protected download URL is invalid.")?;
    crate::model::validate_url(value)?;
    Ok(value)
}

fn read_protected(
    path: &Path,
    header: &[u8],
    purpose: &str,
    limit: usize,
) -> Result<Secret, String> {
    let path = crate::library::authorized_path(path).map_err(|_| {
        "Protected download data is unavailable. Restore its backup or add the link again."
    })?;
    let metadata = fs::metadata(&path).map_err(|_| "Protected download data is unavailable.")?;
    if !metadata.is_file() || metadata.len() > (limit + 64 * 1024) as u64 {
        return Err("Protected download data is invalid. Its files were left untouched.".into());
    }
    let bytes = fs::read(path).map_err(|_| "Protected download data could not be read.")?;
    let ciphertext = bytes
        .strip_prefix(header)
        .ok_or("Protected download data has an unsupported format.")?;
    let secret = Secret(crypt(ciphertext, purpose, true)
        .map_err(|_| "Windows could not unlock protected download data for this user. Restore its backup or add the link again.")?);
    if secret.0.len() > limit {
        return Err("Protected download data exceeds its size limit.".into());
    }
    Ok(secret)
}

fn write_protected(path: &Path, header: &[u8], purpose: &str, bytes: &[u8]) -> Result<(), String> {
    let protected = crypt(bytes, purpose, false)
        .map_err(|_| "Windows could not protect download data. No plaintext record was written.")?;
    let directory = path.parent().ok_or("Invalid protected data directory.")?;
    let temporary = directory.join(format!("protected-{}.new", nonce()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "Could not create protected download data.")?;
    let written = file
        .write_all(header)
        .and_then(|_| file.write_all(&protected))
        .and_then(|_| file.sync_all());
    drop(file);
    let result = (|| {
        written.map_err(|_| "Could not flush protected download data.")?;
        if read_protected(&temporary, header, purpose, bytes.len())?.0 != bytes {
            return Err(
                "Protected download data did not validate. The prior record was kept.".into(),
            );
        }
        replace(&temporary, path).map_err(|_| {
            "Could not commit protected download data. The prior record was kept.".into()
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn seal_url(root: &Path, job_id: &str, url: &str) -> Result<String, String> {
    if job_id.is_empty() || job_id.len() > 160 {
        return Err("Invalid job identifier for protected download data.".into());
    }
    let value = validate_operational_url(url.trim().as_bytes())?;
    // The reference identifies the job, never a hash or fragment of its secret URL.
    let reference = format!(
        "{:x}",
        Sha256::digest(format!("nabnab-operational-url-v1\0{job_id}").as_bytes())
    );
    let directory = private_directory(root, "job-urls")?;
    let path = directory.join(format!("{reference}.bin"));
    if path.exists() {
        if resolve_url(root, &reference)?.as_str() != value {
            return Err("This job's protected URL differs from its original request. No record was replaced.".into());
        }
        return Ok(reference);
    }
    write_protected(
        &path,
        URL_HEADER,
        &format!("operational-url:{reference}"),
        value.as_bytes(),
    )?;
    Ok(reference)
}

pub fn resolve_url(root: &Path, reference: &str) -> Result<OperationalUrl, String> {
    if !valid_url_reference(reference) {
        return Err("The protected download reference is invalid.".into());
    }
    let path = root.join("job-urls").join(format!("{reference}.bin"));
    let secret = read_protected(
        &path,
        URL_HEADER,
        &format!("operational-url:{reference}"),
        MAX_URL_BYTES,
    )?;
    validate_operational_url(&secret.0)?;
    Ok(OperationalUrl(secret))
}

pub(crate) fn remove_uncommitted_url(root: &Path, reference: &str) -> Result<(), String> {
    if !valid_url_reference(reference) { return Err("Invalid protected URL reference.".into()); }
    let directory=private_directory(root,"job-urls")?;
    fs::remove_file(directory.join(format!("{reference}.bin"))).map_err(|_|"Could not remove an uncommitted source reference.".into())
}

pub fn protect_repository_backup(root: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(
            "The previous repository exceeds the supported backup size. It was not modified."
                .into(),
        );
    }
    let directory = private_directory(root, "migration-backups")?;
    let path = directory.join("state-v1.dpapi");
    if path.exists() {
        if read_protected(
            &path,
            BACKUP_HEADER,
            "repository-schema1-backup",
            MAX_BACKUP_BYTES,
        )?
        .0 != bytes
        {
            return Err("The protected schema-1 backup differs from the previous repository. Keep both records and resolve this before retrying migration.".into());
        }
        return Ok(());
    }
    write_protected(&path, BACKUP_HEADER, "repository-schema1-backup", bytes)
}

pub fn list(root: &Path) -> Result<Vec<SessionInfo>, String> {
    let directory = root.join("sessions");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    crate::library::authorized_path(&directory)?;
    Ok(SITES
        .into_iter()
        .filter(|site| directory.join(format!("{site}.bin")).is_file())
        .map(|site| SessionInfo { site: site.into() })
        .collect())
}

pub fn import(root: &Path, site: &str, file: &Path) -> Result<(), String> {
    domains(site)?;
    let file = crate::library::authorized_path(file)?;
    let mut bytes = Secret(Vec::new());
    fs::File::open(file)
        .map_err(|_| "Could not open the selected cookie file.")?
        .take((MAX_COOKIE_BYTES + 1) as u64)
        .read_to_end(&mut bytes.0)
        .map_err(|_| "Could not read the selected cookie file.")?;
    validate_cookies(site, &bytes.0)?;
    let protected = crypt(&bytes.0, site, false)?;
    let directory = private_directory(root, "sessions")?;
    let temporary = directory.join(format!("session-{}.new", nonce()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "Could not create the protected session.")?;
    let result = file
        .write_all(HEADER)
        .and_then(|_| file.write_all(&protected))
        .and_then(|_| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("Could not save the protected session. The previous session was kept.".into());
    }
    let result = replace(&temporary, &directory.join(format!("{site}.bin")));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn revoke(root: &Path, site: &str) -> Result<(), String> {
    domains(site)?;
    let file = root.join("sessions").join(format!("{site}.bin"));
    match fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not remove the protected session.".into()),
    }
}

pub fn materialize(root: &Path, url: &str) -> Result<Option<CookieFile>, String> {
    let Some(site) = site_for(url)? else {
        return Ok(None);
    };
    let file = root.join("sessions").join(format!("{site}.bin"));
    if !file.exists() {
        return Ok(None);
    }
    let file = crate::library::authorized_path(&file)?;
    let mut bytes = Vec::new();
    fs::File::open(file)
        .map_err(|_| "Could not open the protected session.")?
        .take((MAX_COOKIE_BYTES + 16 * 1024) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the protected session.")?;
    let protected = bytes
        .strip_prefix(HEADER)
        .ok_or("The protected session is invalid. Import it again.")?;
    let secret = Secret(crypt(protected, site, true)?);
    validate_cookies(site, &secret.0)?;
    let directory = private_directory(root, "session-temp")?;
    let path = directory.join(format!("cookie-{}.txt", nonce()));
    let temporary = CookieFile { path };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary.path)
        .map_err(|_| "Could not create a private temporary session.")?;
    file.write_all(&secret.0)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not prepare the private temporary session.")?;
    drop(file);
    Ok(Some(temporary))
}

pub fn cleanup(root: &Path) -> Result<(), String> {
    let directory = root.join("session-temp");
    if !directory.exists() {
        return Ok(());
    }
    crate::library::authorized_path(&directory)?;
    for entry in
        fs::read_dir(directory).map_err(|_| "Could not recover temporary session files.")?
    {
        let entry = entry.map_err(|_| "Could not recover a temporary session file.")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("cookie-")
            && name.ends_with(".txt")
            && entry.file_type().is_ok_and(|kind| kind.is_file())
        {
            fs::remove_file(entry.path())
                .map_err(|_| "Could not clean up an interrupted session file.")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const COOKIE: &[u8] = b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tnabnab_fixture\tfictional-session-value\n";

    #[test]
    fn cookie_domains_are_exact_and_provider_scoped() {
        assert!(validate_cookies("youtube", COOKIE).is_ok());
        assert!(validate_cookies("twitter", COOKIE).is_err());
        assert!(validate_cookies("../youtube", COOKIE).is_err());
        for domain in [
            "youtube.com.attacker.net",
            "notyoutube.com",
            "localhost",
            "127.0.0.1",
        ] {
            let bytes =
                format!("# Netscape HTTP Cookie File\n{domain}\tTRUE\t/\tTRUE\t0\tname\tvalue\n");
            assert!(validate_cookies("youtube", bytes.as_bytes()).is_err());
        }
        assert_eq!(
            site_for("https://youtube.com.attacker.net/a").unwrap(),
            None
        );
        assert_eq!(
            site_for("https://www.youtube.com/watch?v=test").unwrap(),
            Some("youtube")
        );
    }

    #[test]
    fn cookie_validation_rejects_incomplete_and_mixed_exports() {
        assert!(validate_cookies("youtube", b"not a cookie export").is_err());
        assert!(validate_cookies("youtube", b"# Netscape HTTP Cookie File\n").is_err());
        assert!(validate_cookies("youtube", &vec![b'x'; MAX_COOKIE_BYTES + 1]).is_err());
        let mixed = [COOKIE, b".example.com\tTRUE\t/\tTRUE\t0\tname\tvalue\n"].concat();
        assert!(validate_cookies("youtube", &mixed).is_err());
        let http_only =
            b"# Netscape HTTP Cookie File\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tname\tvalue\n";
        assert!(validate_cookies("youtube", http_only).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_storage_round_trip_scopes_and_revokes_session() {
        let root = std::env::temp_dir().join(format!("nabnab-session-test-{}", nonce()));
        fs::create_dir_all(&root).unwrap();
        let original = root.join("export.txt");
        fs::write(&original, COOKIE).unwrap();
        import(&root, "youtube", &original).unwrap();
        assert_eq!(list(&root).unwrap()[0].site, "youtube");
        let stored = fs::read(root.join("sessions/youtube.bin")).unwrap();
        assert!(stored.starts_with(HEADER));
        assert!(!stored
            .windows(b"fictional-session-value".len())
            .any(|part| part == b"fictional-session-value"));
        assert!(materialize(&root, "https://x.com/example/status/1")
            .unwrap()
            .is_none());
        let temporary = materialize(&root, "https://youtube.com/watch?v=test")
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(&temporary.path).unwrap(), COOKIE);
        let path = temporary.path.clone();
        drop(temporary);
        assert!(!path.exists());
        assert!(crypt(stored.strip_prefix(HEADER).unwrap(), "twitter", true).is_err());
        revoke(&root, "youtube").unwrap();
        revoke(&root, "youtube").unwrap();
        assert!(list(&root).unwrap().is_empty());
        assert_eq!(fs::read(original).unwrap(), COOKIE);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn startup_cleanup_removes_only_owned_temporary_cookie_files() {
        let root = std::env::temp_dir().join(format!("nabnab-session-cleanup-{}", nonce()));
        let directory = private_directory(&root, "session-temp").unwrap();
        fs::write(directory.join("cookie-previous-run.txt"), COOKIE).unwrap();
        fs::write(directory.join("keep.txt"), b"unrelated").unwrap();
        cleanup(&root).unwrap();
        assert!(!directory.join("cookie-previous-run.txt").exists());
        assert_eq!(fs::read(directory.join("keep.txt")).unwrap(), b"unrelated");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn operational_urls_round_trip_exactly_without_plaintext_or_duplicate_records() {
        let root = std::env::temp_dir().join(format!("nabnab-protected-url-{}", nonce()));
        let url = "https://example.org/private-path-credential?api_key=API-SECRET-UNIQUE&accessToken=ACCESS-SECRET-UNIQUE&signature=SIGNATURE-SECRET-UNIQUE&part=1%2F2#FRAGMENT-SECRET-UNIQUE";
        let reference = seal_url(&root, "1234-1", url).unwrap();
        assert!(valid_url_reference(&reference));
        assert_eq!(resolve_url(&root, &reference).unwrap().as_str(), url);
        let path = root.join("job-urls").join(format!("{reference}.bin"));
        let encrypted = fs::read(&path).unwrap();
        for secret in [
            "private-path-credential",
            "API-SECRET-UNIQUE",
            "ACCESS-SECRET-UNIQUE",
            "SIGNATURE-SECRET-UNIQUE",
            "FRAGMENT-SECRET-UNIQUE",
        ] {
            assert!(!encrypted
                .windows(secret.len())
                .any(|bytes| bytes == secret.as_bytes()));
            assert!(!reference.contains(secret));
        }
        assert_eq!(seal_url(&root, "1234-1", url).unwrap(), reference);
        assert_eq!(fs::read(&path).unwrap(), encrypted);
        assert_eq!(fs::read_dir(root.join("job-urls")).unwrap().count(), 1);
        assert!(seal_url(&root, "1234-1", "https://example.org/different").is_err());
        assert_eq!(fs::read(&path).unwrap(), encrypted);
        assert!(resolve_url(&root, "../outside").is_err());
        assert!(seal_url(
            &root,
            "1234-2",
            &format!("https://example.org/{}", "x".repeat(8192))
        )
        .is_err());
        fs::write(&path, b"corrupted protected record").unwrap();
        assert!(resolve_url(&root, &reference).is_err());
        assert!(seal_url(&root, "1234-1", url).is_err());
        assert_eq!(fs::read(path).unwrap(), b"corrupted protected record");
        fs::remove_dir_all(root).unwrap();
    }
}
