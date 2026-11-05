use crate::providers::{self, ProviderId};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_COOKIE_BYTES: usize = 2 * 1024 * 1024;
const HEADER: &[u8] = b"nabnab-cookie-v1\n";
pub const SITES: [&str; 8] = [
    "youtube",
    "twitter",
    "instagram",
    "reddit",
    "tiktok",
    "vimeo",
    "twitch",
    "pinterest",
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
        ProviderId::Generic => None,
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
}
