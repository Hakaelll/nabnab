use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != 64 || right.len() != 64 {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.bytes().zip(right.bytes()) {
        difference |= a ^ b;
    }
    unsafe { std::ptr::read_volatile(&difference) == 0 }
}
pub fn checked_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Use an absolute local application-data directory.".into());
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|_| "Open the nabnab desktop application before using local automation.")?;
        let mut linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            linked |= metadata.file_attributes() & 0x400 != 0;
        }
        if linked {
            return Err("Automation data cannot use symbolic links or junctions.".into());
        }
    }
    fs::canonicalize(path)
        .map_err(|_| "The local application-data directory is unavailable.".into())
}
pub fn data_root() -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("NABNAB_TEST_DATA_DIR") {
        return checked_path(Path::new(&path));
    }
    if let Some(path) = std::env::var_os("NABNAB_DATA_DIR") {
        return checked_path(Path::new(&path));
    }
    let appdata = std::env::var_os("APPDATA")
        .ok_or("Windows APPDATA is unavailable. Open the desktop application first.")?;
    checked_path(&PathBuf::from(appdata).join("com.ytnabber.desktop"))
}
pub fn pipe_name(root: &Path) -> Result<String, String> {
    let root = checked_path(root)?;
    let identity = format!(
        "{}\0{}",
        root.to_string_lossy().to_lowercase(),
        current_sid()?
    );
    Ok(format!(
        r"\\.\pipe\nabnab-automation-v1-{:x}",
        Sha256::digest(identity.as_bytes())
    ))
}
#[cfg(windows)]
pub fn current_sid() -> Result<String, String> {
    token_sid(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() })
}
#[cfg(windows)]
fn token_sid(process: windows_sys::Win32::Foundation::HANDLE) -> Result<String, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::Threading::OpenProcessToken,
    };
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err("Windows could not identify the automation owner.".into());
    }
    let result = (|| {
        let mut length = 0;
        unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut length) };
        if length == 0 || length > 65536 {
            return Err("Windows could not identify the automation owner.".into());
        }
        let mut bytes = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            return Err("Windows could not identify the automation owner.".into());
        }
        let user = unsafe { &*bytes.as_ptr().cast::<TOKEN_USER>() };
        let mut sid = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
            return Err("Windows could not identify the automation owner.".into());
        }
        let mut length = 0;
        unsafe {
            while *sid.add(length) != 0 {
                length += 1;
            }
        }
        let text = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(sid, length)) };
        unsafe {
            LocalFree(sid.cast());
        }
        Ok(text)
    })();
    unsafe {
        CloseHandle(token);
    }
    result
}
#[cfg(windows)]
pub fn verify_pipe_server(handle: windows_sys::Win32::Foundation::HANDLE) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            Pipes::GetNamedPipeServerProcessId,
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
    };
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(handle, &mut pid) } == 0 {
        return Err("Could not verify the local desktop owner.".into());
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err("Could not verify the local desktop owner.".into());
    }
    let sid = token_sid(process);
    unsafe {
        CloseHandle(process);
    }
    if sid? != current_sid()? {
        return Err("The local automation pipe belongs to another Windows user.".into());
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn current_sid() -> Result<String, String> {
    Err("Local automation currently requires Windows.".into())
}
#[cfg(windows)]
pub struct SecurityDescriptor(*mut std::ffi::c_void);
#[cfg(windows)]
impl SecurityDescriptor {
    pub fn owner_only(inherit: bool) -> Result<Self, String> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        let flags = if inherit { "OICI" } else { "" };
        let sddl: Vec<u16> = format!("D:P(A;{flags};GA;;;{})", current_sid()?)
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
            return Err("Windows could not construct private automation permissions.".into());
        }
        Ok(Self(descriptor))
    }
    pub fn as_ptr(&self) -> *mut std::ffi::c_void {
        self.0
    }
}
#[cfg(windows)]
impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::LocalFree(self.0);
        }
    }
}
#[cfg(windows)]
fn secure_directory(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    let descriptor = SecurityDescriptor::owner_only(true)?;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor.as_ptr(),
        )
    } == 0
    {
        Err("Windows could not secure automation credentials for this user.".into())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn secure_directory(_: &Path) -> Result<(), String> {
    Err("Private automation credentials currently require Windows.".into())
}
#[cfg(windows)]
pub fn random_id() -> Result<String, String> {
    random_hex(16)
}
#[cfg(not(windows))]
pub fn random_id() -> Result<String, String> {
    Err("Local automation currently requires Windows.".into())
}
#[cfg(windows)]
fn random_hex(length: usize) -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut bytes = vec![0u8; length];
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            length as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } != 0
    {
        return Err("Windows could not generate local automation credentials.".into());
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
#[cfg(windows)]
fn crypt(bytes: &[u8], id: &str, decrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    let entropy = format!("nabnab-automation-v1:{id}");
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
        return Err(
            "Windows could not protect or unlock this client's credentials for the current user."
                .into(),
        );
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        if decrypt {
            for index in 0..output.cbData as usize {
                std::ptr::write_volatile(output.pbData.add(index), 0);
            }
        }
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}
#[cfg(not(windows))]
fn crypt(_: &[u8], _: &str, _: bool) -> Result<Vec<u8>, String> {
    Err("Local automation credentials currently require Windows.".into())
}
fn directory(root: &Path, create: bool) -> Result<PathBuf, String> {
    let root = checked_path(root)?;
    let path = root.join("automation-credentials");
    if create && !path.exists() {
        fs::create_dir(&path).map_err(|_| "Could not create private automation credentials.")?;
    }
    let path = checked_path(&path)?;
    if create {
        secure_directory(&path)?;
    }
    Ok(path)
}
fn record(root: &Path, id: &str, create: bool) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Choose a valid local automation client identifier.".into());
    }
    Ok(directory(root, create)?.join(format!("{id}.bin")))
}
pub fn create(root: &Path, id: &str) -> Result<(), String> {
    #[cfg(windows)]
    let secret = random_hex(32)?;
    #[cfg(not(windows))]
    let secret = String::new();
    let path = record(root, id, true)?;
    let bytes = crypt(secret.as_bytes(), id, false)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| {
            "Could not create new private client credentials. Existing credentials were preserved."
        })?;
    if file
        .write_all(b"nabnab-automation-v1\n")
        .and_then(|_| file.write_all(&bytes))
        .and_then(|_| file.sync_all())
        .is_err()
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err("Could not commit private client credentials.".into());
    }
    Ok(())
}
pub fn load(root: &Path, id: &str) -> Result<String, String> {
    let path = checked_path(&record(root, id, false)?)?;
    let metadata = fs::metadata(&path).map_err(|_| {
        "This client's credentials are unavailable. Connect again and approve it in the desktop."
    })?;
    if !metadata.is_file() || metadata.len() > 65536 {
        return Err("This client's private credentials are invalid.".into());
    }
    let bytes =
        fs::read(path).map_err(|_| "This client's private credentials could not be read.")?;
    let bytes = bytes
        .strip_prefix(b"nabnab-automation-v1\n")
        .ok_or("This client's private credentials have an unsupported version.")?;
    let secret = String::from_utf8(crypt(bytes, id, true)?)
        .map_err(|_| "This client's private credentials are invalid.")?;
    if secret.len() != 64
        || !secret
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("This client's private credentials are invalid.".into());
    }
    Ok(secret)
}
pub fn revoke(root: &Path, id: &str) -> Result<(), String> {
    let path = record(root, id, false)?;
    if path.exists() {
        fs::remove_file(checked_path(&path)?)
            .map_err(|_| "Could not remove this client's private credentials.")?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_and_credentials_compare_without_accepting_invalid_lengths() {
        assert!(valid_id(&"a".repeat(32)));
        assert!(!valid_id("../secret"));
        assert!(constant_time_eq(&"b".repeat(64), &"b".repeat(64)));
        assert!(!constant_time_eq(&"a".repeat(64), &"b".repeat(64)));
        assert!(!constant_time_eq("", ""));
    }
    #[cfg(windows)]
    #[test]
    fn private_dpapi_credentials_roundtrip_and_cannot_change_identity() {
        let root = std::env::temp_dir().join(format!("nabnab-automation-{}", random_id().unwrap()));
        fs::create_dir(&root).unwrap();
        let id = random_id().unwrap();
        create(&root, &id).unwrap();
        let secret = load(&root, &id).unwrap();
        assert_eq!(secret.len(), 64);
        let path = record(&root, &id, false).unwrap();
        let encrypted = fs::read(&path).unwrap();
        assert!(!encrypted
            .windows(64)
            .any(|window| window == secret.as_bytes()));
        assert!(create(&root, &id).is_err());
        assert_eq!(load(&root, &id).unwrap(), secret);
        let second = random_id().unwrap();
        fs::copy(&path, record(&root, &second, false).unwrap()).unwrap();
        assert!(load(&root, &second).is_err());
        assert_eq!(
            pipe_name(&root).unwrap(),
            pipe_name(&fs::canonicalize(&root).unwrap()).unwrap()
        );
        revoke(&root, &id).unwrap();
        assert!(load(&root, &id).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
