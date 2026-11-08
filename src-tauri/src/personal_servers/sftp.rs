use super::{copy_progress, Entry, Stamp, SFTP_CHANGED as CHANGED, SFTP_DISCONNECTED};
use crate::model::Progress;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    future::Future,
    io::Write,
    net::IpAddr,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
    sync::watch,
    task::JoinHandle,
};

const MAX_PACKET: usize = 256 * 1024;
const MAX_READ: usize = 128 * 1024;
const MAX_LIST: usize = 500;
const MAX_DEPTH: usize = 24;
const MAX_PATH: usize = 4096;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const INVALID_PACKET: &str = "The SFTP server sent an invalid or unsupported response.";

const INIT: u8 = 1;
const VERSION: u8 = 2;
const OPEN: u8 = 3;
const CLOSE: u8 = 4;
const READ: u8 = 5;
const LSTAT: u8 = 7;
const FSTAT: u8 = 8;
const OPENDIR: u8 = 11;
const READDIR: u8 = 12;
const REALPATH: u8 = 16;
const STATUS: u8 = 101;
const HANDLE: u8 = 102;
const DATA: u8 = 103;
const NAME: u8 = 104;
const ATTRS: u8 = 105;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_regular_files_with_the_reviewed_size_and_time_can_be_copied() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&13u32.to_be_bytes());
        wire.extend_from_slice(&123u64.to_be_bytes());
        wire.extend_from_slice(&0o100644u32.to_be_bytes());
        wire.extend_from_slice(&0u32.to_be_bytes());
        wire.extend_from_slice(&99u32.to_be_bytes());
        let reviewed = Stamp {
            size: Some(123),
            modified: Some("99".into()),
            etag: None,
        };
        let mut reader = Decoder(&wire);
        assert!(matches_stamp(&reader.attrs().unwrap(), &reviewed));
        reader.finish().unwrap();
        wire[12..16].copy_from_slice(&0o120777u32.to_be_bytes());
        assert!(!matches_stamp(&Decoder(&wire).attrs().unwrap(), &reviewed));
        assert!(Decoder(&wire[..16]).attrs().is_err());
        assert!(scoped_path("/authorized", &["..".into(), "unrelated".into()]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn host_fingerprint_is_tied_to_the_public_key_bytes() {
        let blob = "AAAAC3NzaC1lZDI1NTE5AAAAIAcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcH";
        let key = host_key("ssh-ed25519", blob).unwrap();
        assert_eq!(
            key.fingerprint,
            "SHA256:gNSIRW+2Iyiuvsdp/bgjy38bvWHw6wQm3tuoXrl3WjQ"
        );
        assert!(host_key("ssh-rsa", blob).is_err());
        assert!(host_key("ssh-ed25519", &format!("{blob}\nextra")).is_err());
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Config {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub root: String,
    pub identity: PathBuf,
    pub host_key: Option<HostKey>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct HostKey {
    pub fingerprint: String,
    algorithm: String,
    blob: String,
}

fn hostname(value: &str) -> Result<String, String> {
    let value = value.trim();
    let address = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    if let Ok(ip) = address.parse::<IpAddr>() {
        if ip.is_unspecified() || ip.is_multicast() || address == "255.255.255.255" {
            return Err("Enter a single unicast SFTP host.".into());
        }
        return Ok(ip.to_string());
    }
    if value.len() > 253
        || !value.bytes().any(|byte| byte.is_ascii_alphabetic())
        || !value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err("Enter a single SFTP hostname or IP address without a URL, network range or zone identifier.".into());
    }
    Ok(value.to_ascii_lowercase())
}

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !matches!(value, "." | "..")
        && !value.contains(['/', '\\', ':'])
        && !value.chars().any(char::is_control)
}

fn scoped_path(root: &str, relative: &[String]) -> Result<Vec<String>, String> {
    let root = root
        .strip_prefix('/')
        .ok_or("Use an absolute SFTP root such as /media.")?;
    let parts: Vec<_> = if root.is_empty() {
        vec![]
    } else {
        root.split('/').collect()
    };
    if parts.len() + relative.len() > MAX_DEPTH
        || !parts
            .iter()
            .copied()
            .chain(relative.iter().map(String::as_str))
            .all(segment)
        || root.len() + relative.iter().map(|part| part.len() + 1).sum::<usize>() >= MAX_PATH
    {
        return Err("The SFTP path contains invalid segments or exceeds its scope limit.".into());
    }
    Ok(parts
        .into_iter()
        .map(str::to_owned)
        .chain(relative.iter().cloned())
        .collect())
}

fn local_path(path: &Path) -> Result<&str, String> {
    let value = path
        .to_str()
        .ok_or("Use a local Windows path supported by OpenSSH.")?;
    if !path.is_absolute()
        || value
            .chars()
            .any(|c| c.is_control() || matches!(c, '%' | '"' | '{' | '}'))
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("Use an absolute local path without OpenSSH expansion characters.".into());
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        let drive = match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
                _ => return Err("Choose a key on a local drive.".into()),
            },
            _ => return Err("Choose a key on a local drive.".into()),
        };
        let root = [u16::from(drive), b':' as u16, b'\\' as u16, 0];
        let drive_type =
            unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) };
        // Win32 DRIVE_REMOVABLE and DRIVE_FIXED.
        if !matches!(drive_type, 2 | 3) {
            return Err("Choose a key on a local drive.".into());
        }
    }
    Ok(value)
}

fn local_metadata(path: &Path) -> Result<fs::Metadata, String> {
    local_path(path)?;
    let mut checked = PathBuf::new();
    let mut last = None;
    for component in path.components() {
        checked.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&checked).map_err(|_| "The local OpenSSH path is unavailable.")?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err("Choose a local OpenSSH path without junctions or links.".into());
            }
        }
        last = Some(metadata);
    }
    last.ok_or_else(|| "The local OpenSSH path is unavailable.".into())
}

pub(super) fn normalize(mut config: Config) -> Result<Config, String> {
    config.host = hostname(&config.host)?;
    if config.port == 0 {
        return Err("Choose an SFTP port from 1 to 65535.".into());
    }
    config.username = config.username.trim().to_owned();
    if config.username.is_empty()
        || config.username.len() > 128
        || config.username.starts_with('-')
        || !config
            .username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-@".contains(&byte))
    {
        return Err("Use a supported SFTP login name.".into());
    }
    let root = config.root.trim().trim_end_matches('/');
    config.root = if root.is_empty() && config.root.trim() == "/" {
        "/".into()
    } else {
        root.into()
    };
    scoped_path(&config.root, &[])?;
    local_path(&config.identity)?;
    if let Some(key) = &config.host_key {
        if host_key(&key.algorithm, &key.blob)? != *key {
            return Err(
                "The saved SFTP host key fingerprint does not match its public key.".into(),
            );
        }
    }
    Ok(config)
}

#[cfg(windows)]
fn executable(name: &str) -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 32_768];
    let len = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if len == 0 || len >= buffer.len() {
        return Err("Windows could not locate the system OpenSSH client.".into());
    }
    let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len]))
        .join("OpenSSH")
        .join(name);
    path.is_file()
        .then_some(path)
        .ok_or_else(|| "SFTP needs the optional Windows OpenSSH Client.".into())
}

#[cfg(not(windows))]
fn executable(_name: &str) -> Result<PathBuf, String> {
    Err("This SFTP connector uses the Windows OpenSSH Client.".into())
}

fn command(program: &str) -> Result<Command, String> {
    let mut command = Command::new(executable(program)?);
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    Ok(command)
}

fn host_algorithms(algorithm: &str) -> Option<&str> {
    match algorithm {
        "ssh-ed25519" | "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => {
            Some(algorithm)
        }
        "ssh-rsa" => Some("rsa-sha2-512,rsa-sha2-256"),
        _ => None,
    }
}

#[cfg(windows)]
fn fingerprint(algorithm: &str, blob: &str) -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{
        CryptBinaryToStringA, CryptStringToBinaryA, CRYPT_STRING_BASE64, CRYPT_STRING_NOCRLF,
        CRYPT_STRING_STRICT,
    };
    let invalid = "The SFTP server returned an invalid public host key.";
    let mut key = vec![0; blob.len()];
    let mut length = key.len() as u32;
    let decoded = unsafe {
        CryptStringToBinaryA(
            blob.as_ptr(),
            blob.len() as u32,
            CRYPT_STRING_BASE64 | CRYPT_STRING_STRICT,
            key.as_mut_ptr(),
            &mut length,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if decoded == 0 {
        return Err(invalid.into());
    }
    key.truncate(length as usize);
    let mut wire = Decoder(&key);
    if wire.string()? != algorithm.as_bytes() || wire.0.is_empty() {
        return Err(invalid.into());
    }
    let hash = Sha256::digest(&key);
    let mut encoded = [0u8; 64];
    let mut length = encoded.len() as u32;
    let ok = unsafe {
        CryptBinaryToStringA(
            hash.as_ptr(),
            hash.len() as u32,
            CRYPT_STRING_BASE64 | CRYPT_STRING_NOCRLF,
            encoded.as_mut_ptr(),
            &mut length,
        )
    };
    if ok == 0 {
        return Err("Windows could not calculate the SFTP host fingerprint.".into());
    }
    let text = std::str::from_utf8(&encoded[..length as usize]).map_err(|_| invalid)?;
    Ok(format!("SHA256:{}", text.trim_end_matches(['\0', '='])))
}

#[cfg(not(windows))]
fn fingerprint(_algorithm: &str, _blob: &str) -> Result<String, String> {
    Err("This SFTP connector uses the Windows OpenSSH Client.".into())
}

fn host_key(algorithm: &str, blob: &str) -> Result<HostKey, String> {
    if host_algorithms(algorithm).is_none()
        || blob.is_empty()
        || blob.len() > 8192
        || !blob
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+/=".contains(&byte))
    {
        return Err("The server did not offer a supported SFTP host key.".into());
    }
    Ok(HostKey {
        fingerprint: fingerprint(algorithm, blob)?,
        algorithm: algorithm.into(),
        blob: blob.into(),
    })
}

async fn cancelled(mut cancel: watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn bounded<T>(
    cancel: watch::Receiver<bool>,
    timeout: Duration,
    operation: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = cancelled(cancel) => Err(crate::engine::CANCELLED.into()),
        result = tokio::time::timeout(timeout, operation) =>
            result.map_err(|_| "The SFTP connection or operation timed out.")?,
    }
}

pub(super) async fn probe(
    config: &Config,
    cancel: watch::Receiver<bool>,
) -> Result<HostKey, String> {
    let config = normalize(config.clone())?;
    bounded(cancel, REQUEST_TIMEOUT, async move {
        let mut child = command("ssh-keyscan.exe")?
            .args(["-T", "6", "-p", &config.port.to_string(), "-t", "ed25519,ecdsa,rsa", "--", &config.host])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|_| "Windows could not start the SFTP host key check.")?;
        let stdout = child.stdout.take().ok_or("The host key check did not open its output.")?;
        let mut bytes = Vec::new();
        stdout.take(32_769).read_to_end(&mut bytes).await.map_err(|_| "The host key check was interrupted.")?;
        if bytes.len() > 32_768 {
            return Err("The host key check exceeded its output limit.".into());
        }
        let status = child.wait().await.map_err(|_| "The SFTP host key check failed.")?;
        if !status.success() {
            return Err("The SFTP server did not return a supported key on the configured port.".into());
        }
        let keys = std::str::from_utf8(&bytes).map_err(|_| "The host key check returned invalid text.")?
            .lines().filter(|line| !line.starts_with('#')).filter_map(|line| {
                let mut fields = line.split_ascii_whitespace();
                fields.next()?;
                host_key(fields.next()?, fields.next()?).ok()
            }).collect::<Vec<_>>();
        if let Some(key) = config.host_key {
            return keys.into_iter().find(|candidate| *candidate == key)
                .ok_or_else(|| "The SFTP host key changed. Verify the new fingerprint with the server administrator.".into());
        }
        ["ssh-ed25519", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521", "ssh-rsa"]
            .into_iter().find_map(|algorithm| keys.iter().find(|key| key.algorithm == algorithm).cloned())
            .ok_or_else(|| "The SFTP server did not return a supported key on the configured port.".into())
    }).await
}

struct KnownHosts(PathBuf);

impl KnownHosts {
    fn create(key: &HostKey) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir();
        if !local_metadata(&dir)?.is_dir() {
            return Err("The local temporary folder is unavailable for SFTP.".into());
        }
        let path = dir.join(format!(
            "nabnab-known-host-{}-{}-{}",
            std::process::id(),
            crate::queue::now(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|_| "Could not prepare a private SFTP known-hosts file.")?;
        let guard = Self(path);
        let written = writeln!(file, "nabnab-pinned {} {}", key.algorithm, key.blob);
        drop(file);
        written.map_err(|_| "Could not write the pinned SFTP host key.")?;
        Ok(guard)
    }
}

impl Drop for KnownHosts {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Directory,
}

struct Attrs {
    size: Option<u64>,
    modified: Option<u32>,
    mode: Option<u32>,
}

impl Attrs {
    fn kind(&self) -> Option<Kind> {
        match self.mode? & 0o170000 {
            0o100000 => Some(Kind::File),
            0o040000 => Some(Kind::Directory),
            _ => None,
        }
    }

    fn stamp(&self) -> Stamp {
        Stamp {
            size: self.size,
            modified: self.modified.map(|stamp| stamp.to_string()),
            etag: None,
        }
    }
}

struct Decoder<'a>(&'a [u8]);

impl<'a> Decoder<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let (value, rest) = self.0.split_at_checked(len).ok_or(INVALID_PACKET)?;
        self.0 = rest;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("eight bytes"),
        ))
    }

    fn string(&mut self) -> Result<&'a [u8], String> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    fn attrs(&mut self) -> Result<Attrs, String> {
        let flags = self.u32()?;
        if flags & !0x8000_000f != 0 {
            return Err(INVALID_PACKET.into());
        }
        let size = if flags & 1 != 0 {
            Some(self.u64()?)
        } else {
            None
        };
        if flags & 2 != 0 {
            self.take(8)?;
        }
        let mode = if flags & 4 != 0 {
            Some(self.u32()?)
        } else {
            None
        };
        let modified = if flags & 8 != 0 {
            self.u32()?;
            Some(self.u32()?)
        } else {
            None
        };
        if flags & 0x8000_0000 != 0 {
            let count = self.u32()?;
            if count > 32 {
                return Err(INVALID_PACKET.into());
            }
            for _ in 0..count {
                self.string()?;
                self.string()?;
            }
        }
        Ok(Attrs {
            size,
            modified,
            mode,
        })
    }

    fn finish(&self) -> Result<(), String> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(INVALID_PACKET.into())
        }
    }
}

fn push_string(payload: &mut Vec<u8>, value: &[u8]) {
    payload.extend_from_slice(&(value.len() as u32).to_be_bytes());
    payload.extend_from_slice(value);
}

fn string_payload(value: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(value.len() + 4);
    push_string(&mut payload, value);
    payload
}

struct Response {
    kind: u8,
    data: Vec<u8>,
}

impl Response {
    fn status(&self) -> Result<u32, String> {
        if self.kind != STATUS {
            return Err(INVALID_PACKET.into());
        }
        Decoder(&self.data).u32()
    }

    fn body(&self, kind: u8) -> Result<Decoder<'_>, String> {
        if self.kind == kind {
            return Ok(Decoder(&self.data));
        }
        match self.status()? {
            2 => Err("The SFTP item is no longer available.".into()),
            3 => Err("The SFTP account is not allowed to read this item.".into()),
            _ => Err("The SFTP server could not read the selected item.".into()),
        }
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    next_id: u32,
    cancel: watch::Receiver<bool>,
    stderr: JoinHandle<Vec<u8>>,
    _known_hosts: KnownHosts,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stderr.abort();
    }
}

async fn drain_stderr(mut stderr: ChildStderr) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    while let Ok(count) = stderr.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count.min(8192 - bytes.len())]);
    }
    bytes
}

impl Session {
    async fn connect(config: &Config, cancel: watch::Receiver<bool>) -> Result<Self, String> {
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        let key = config
            .host_key
            .as_ref()
            .ok_or("Verify the SFTP server fingerprint before connecting.")?;
        if !local_metadata(&config.identity)?.is_file() {
            return Err("Choose a local SFTP private key file.".into());
        }
        let identity = local_path(&config.identity)?;
        #[cfg(windows)]
        let identity = identity.strip_prefix(r"\\?\").unwrap_or(identity);
        let known_hosts = KnownHosts::create(key)?;
        let known_hosts_path = local_path(&known_hosts.0)?;
        let mut child = command("ssh.exe")?
            .args([
                "-F",
                "none",
                "-T",
                "-a",
                "-x",
                "-s",
                "-o",
                "BatchMode=yes",
                "-o",
                "IdentitiesOnly=yes",
                "-o",
                "IdentityAgent=none",
                "-o",
                "PreferredAuthentications=publickey",
                "-o",
                "CertificateFile=none",
                "-o",
                "SecurityKeyProvider=none",
                "-o",
                "PasswordAuthentication=no",
                "-o",
                "KbdInteractiveAuthentication=no",
                "-o",
                "ClearAllForwardings=yes",
                "-o",
                "ProxyCommand=none",
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                "CheckHostIP=no",
                "-o",
                "HostKeyAlias=nabnab-pinned",
                "-o",
                "GlobalKnownHostsFile=none",
                "-o",
                "UpdateHostKeys=no",
                "-o",
                "VerifyHostKeyDNS=no",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ConnectionAttempts=1",
                "-o",
                &format!(
                    "HostKeyAlgorithms={}",
                    host_algorithms(&key.algorithm).ok_or(INVALID_PACKET)?
                ),
                "-o",
                &format!(
                    "UserKnownHostsFile=\"{}\"",
                    known_hosts_path.replace('\\', "/")
                ),
                "-i",
                identity,
                "-p",
                &config.port.to_string(),
                "-l",
                &config.username,
                "--",
                &config.host,
                "sftp",
            ])
            .env_remove("SSH_AUTH_SOCK")
            .env_remove("SSH_SK_PROVIDER")
            .env_remove("SSH_ASKPASS")
            .env("SSH_ASKPASS_REQUIRE", "never")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "Windows could not start OpenSSH for this SFTP connection.")?;
        let stdin = child.stdin.take().ok_or(INVALID_PACKET)?;
        let stdout = child.stdout.take().ok_or(INVALID_PACKET)?;
        let stderr = child.stderr.take().ok_or(INVALID_PACKET)?;
        let mut session = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            cancel,
            stderr: tokio::spawn(drain_stderr(stderr)),
            _known_hosts: known_hosts,
        };
        let response = session.exchange(&[INIT, 0, 0, 0, 3]).await;
        match response {
            Ok(response)
                if response.first() == Some(&VERSION) && Decoder(&response[1..]).u32()? == 3 =>
            {
                Ok(session)
            }
            Err(error) if error == crate::engine::CANCELLED => Err(error),
            _ => {
                let _ = session.child.start_kill();
                let bytes = tokio::time::timeout(Duration::from_millis(500), &mut session.stderr)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();
                let error = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
                if error.contains("host key verification failed")
                    || error.contains("remote host identification has changed")
                    || error.contains("no matching host key type")
                {
                    Err("The SFTP host key differs from the trusted key. Verify the fingerprint before reconnecting.".into())
                } else if error.contains("permission denied") || error.contains("bad permissions") {
                    Err("The SFTP server rejected the selected key. Check the login, private key and its permissions.".into())
                } else {
                    Err("Could not open an SFTP v3 session. Check that the server is online and SFTP is available for this login.".into())
                }
            }
        }
    }

    async fn exchange(&mut self, packet: &[u8]) -> Result<Vec<u8>, String> {
        bounded(self.cancel.clone(), REQUEST_TIMEOUT, async {
            self.stdin
                .write_all(&(packet.len() as u32).to_be_bytes())
                .await
                .map_err(|_| SFTP_DISCONNECTED)?;
            self.stdin
                .write_all(packet)
                .await
                .map_err(|_| SFTP_DISCONNECTED)?;
            self.stdin.flush().await.map_err(|_| SFTP_DISCONNECTED)?;
            let len = self
                .stdout
                .read_u32()
                .await
                .map_err(|_| SFTP_DISCONNECTED)? as usize;
            if len == 0 || len > MAX_PACKET {
                return Err(INVALID_PACKET.into());
            }
            let mut response = vec![0; len];
            self.stdout
                .read_exact(&mut response)
                .await
                .map_err(|_| SFTP_DISCONNECTED)?;
            Ok(response)
        })
        .await
    }

    async fn request(&mut self, kind: u8, payload: Vec<u8>) -> Result<Response, String> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("The SFTP session exceeded its request limit.")?;
        let mut packet = Vec::with_capacity(payload.len() + 5);
        packet.push(kind);
        packet.extend_from_slice(&id.to_be_bytes());
        packet.extend(payload);
        let mut result = self.exchange(&packet).await?;
        if result.len() < 5 || Decoder(&result[1..]).u32()? != id {
            return Err(INVALID_PACKET.into());
        }
        let kind = result[0];
        result.drain(..5);
        Ok(Response { kind, data: result })
    }

    async fn attrs(&mut self, request: u8, value: &[u8]) -> Result<Attrs, String> {
        let response = self.request(request, string_payload(value)).await?;
        let mut body = response.body(ATTRS)?;
        let attrs = body.attrs()?;
        body.finish()?;
        Ok(attrs)
    }

    async fn realpath(&mut self, path: &str) -> Result<(), String> {
        let response = self
            .request(REALPATH, string_payload(path.as_bytes()))
            .await?;
        let mut body = response.body(NAME)?;
        if body.u32()? != 1 || body.string()? != path.as_bytes() {
            return Err("The SFTP server resolved a path outside the selected canonical root or through a link.".into());
        }
        body.string()?;
        body.attrs()?;
        body.finish()
    }

    async fn checked_path(
        &mut self,
        parts: &[String],
        kind: Kind,
    ) -> Result<(String, Attrs), String> {
        let mut path = String::from("/");
        let mut attrs = self.attrs(LSTAT, path.as_bytes()).await?;
        if attrs.kind() != Some(Kind::Directory) {
            return Err("The SFTP root is not a real directory.".into());
        }
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                path.push('/');
            }
            path.push_str(part);
            attrs = self.attrs(LSTAT, path.as_bytes()).await?;
            let expected = if index + 1 == parts.len() {
                kind
            } else {
                Kind::Directory
            };
            if attrs.kind() != Some(expected) {
                return Err(
                    "The SFTP path contains a link, special file or unexpected type.".into(),
                );
            }
        }
        if attrs.kind() != Some(kind) {
            return Err("Select an SFTP file to copy.".into());
        }
        self.realpath(&path).await?;
        Ok((path, attrs))
    }

    async fn handle(&mut self, request: u8, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let response = self.request(request, payload).await?;
        let mut body = response.body(HANDLE)?;
        let handle = body.string()?;
        if handle.is_empty() || handle.len() > 256 {
            return Err(INVALID_PACKET.into());
        }
        body.finish()?;
        Ok(handle.to_vec())
    }

    async fn close(&mut self, handle: &[u8]) -> Result<(), String> {
        let response = self.request(CLOSE, string_payload(handle)).await?;
        if response.status()? != 0 {
            return Err("The SFTP server could not close the selected item.".into());
        }
        Ok(())
    }

    async fn list(&mut self, path: &str, relative: &[String]) -> Result<Vec<Entry>, String> {
        let handle = self
            .handle(OPENDIR, string_payload(path.as_bytes()))
            .await?;
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        let mut inspected = 0;
        for _ in 0..=MAX_LIST {
            let response = self.request(READDIR, string_payload(&handle)).await?;
            if response.kind == STATUS && response.status()? == 1 {
                self.close(&handle).await?;
                return Ok(entries);
            }
            let mut body = response.body(NAME)?;
            let count = body.u32()? as usize;
            inspected += count;
            if count == 0 || inspected > MAX_LIST + 2 {
                return Err("The SFTP folder exceeds its 500-entry listing limit. Select a narrower folder.".into());
            }
            for _ in 0..count {
                let name = body.string()?;
                body.string()?;
                let attrs = body.attrs()?;
                let Ok(name) = std::str::from_utf8(name) else {
                    continue;
                };
                if !segment(name) || !seen.insert(name.to_owned()) {
                    continue;
                }
                if seen.len() > MAX_LIST {
                    return Err(
                        "The SFTP folder exceeds 500 entries. Select a narrower folder.".into(),
                    );
                }
                let Some(kind) = attrs.kind() else { continue };
                let mut path = relative.to_vec();
                path.push(name.to_owned());
                entries.push(Entry {
                    path,
                    directory: kind == Kind::Directory,
                    stamp: attrs.stamp(),
                    mime: None,
                });
            }
            body.finish()?;
        }
        Err("The SFTP folder exceeded its bounded listing limit.".into())
    }

    async fn read(
        &mut self,
        handle: &[u8],
        offset: u64,
        count: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut payload = string_payload(handle);
        payload.extend_from_slice(&offset.to_be_bytes());
        payload.extend_from_slice(&(count as u32).to_be_bytes());
        let response = self.request(READ, payload).await?;
        if response.kind == STATUS && response.status()? == 1 {
            return Ok(None);
        }
        let mut body = response.body(DATA)?;
        let bytes = body.string()?;
        if bytes.is_empty() || bytes.len() > count {
            return Err(INVALID_PACKET.into());
        }
        body.finish()?;
        Ok(Some(bytes.to_vec()))
    }
}

pub(super) async fn browse(
    config: &Config,
    relative: &[String],
    cancel: watch::Receiver<bool>,
) -> Result<Vec<Entry>, String> {
    let config = normalize(config.clone())?;
    let parts = scoped_path(&config.root, relative)?;
    bounded(cancel.clone(), CHECK_TIMEOUT, async {
        let mut session = Session::connect(&config, cancel).await?;
        let (path, _) = session.checked_path(&parts, Kind::Directory).await?;
        let entries = session.list(&path, relative).await?;
        session.checked_path(&parts, Kind::Directory).await?;
        Ok(entries)
    })
    .await
}

fn matches_stamp(attrs: &Attrs, expected: &Stamp) -> bool {
    let current = attrs.stamp();
    attrs.kind() == Some(Kind::File)
        && current.size.is_some()
        && current.size == expected.size
        && current.modified.is_some()
        && current.modified == expected.modified
        && expected.etag.is_none()
}

pub(super) async fn download(
    config: &Config,
    relative: &[String],
    expected: &Stamp,
    destination: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress),
) -> Result<(), String> {
    let config = normalize(config.clone())?;
    if relative.is_empty() {
        return Err("Select a file inside the SFTP root.".into());
    }
    let parts = scoped_path(&config.root, relative)?;
    let size = expected.size.ok_or(CHANGED)?;
    let mut session = Session::connect(&config, cancel.clone()).await?;
    let (path, attrs) = bounded(
        cancel.clone(),
        CHECK_TIMEOUT,
        session.checked_path(&parts, Kind::File),
    )
    .await?;
    if !matches_stamp(&attrs, expected) {
        return Err(CHANGED.into());
    }
    let mut open = string_payload(path.as_bytes());
    open.extend_from_slice(&1u32.to_be_bytes()); // SSH_FXF_READ
    open.extend_from_slice(&0u32.to_be_bytes()); // No create attributes.
    let handle = session.handle(OPEN, open).await?;
    if !matches_stamp(&session.attrs(FSTAT, &handle).await?, expected) {
        return Err(CHANGED.into());
    }
    let (_, opened) = bounded(
        cancel.clone(),
        CHECK_TIMEOUT,
        session.checked_path(&parts, Kind::File),
    )
    .await?;
    if !matches_stamp(&opened, expected) {
        return Err(CHANGED.into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "Could not create a new staged SFTP copy.")?;
    let progress = |written| emit(copy_progress(run_id, size, written));
    progress(0);
    let result = async {
        let mut offset = 0;
        let mut last_report = Instant::now();
        while offset < size {
            let count = (size - offset).min(MAX_READ as u64) as usize;
            let bytes = session.read(&handle, offset, count).await?.ok_or(CHANGED)?;
            output
                .write_all(&bytes)
                .map_err(|_| "Could not write the staged SFTP copy.")?;
            let first = offset == 0;
            offset += bytes.len() as u64;
            if offset < size && (first || last_report.elapsed() >= Duration::from_millis(200)) {
                progress(offset);
                last_report = Instant::now();
            }
        }
        if session.read(&handle, offset, 1).await?.is_some()
            || !matches_stamp(&session.attrs(FSTAT, &handle).await?, expected)
        {
            return Err(CHANGED.into());
        }
        session.close(&handle).await?;
        let (_, after) = bounded(
            cancel.clone(),
            CHECK_TIMEOUT,
            session.checked_path(&parts, Kind::File),
        )
        .await?;
        if !matches_stamp(&after, expected) {
            return Err(CHANGED.into());
        }
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        output
            .sync_all()
            .map_err(|_| "Could not verify the staged SFTP copy.".into())
    }
    .await;
    drop(output);
    if result.is_err() {
        let _ = fs::remove_file(destination);
    } else {
        progress(size);
    }
    result
}
