use crate::model::Progress;
use reqwest::{header, redirect::Policy, Method, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

const CALLBACK: &str = "http://127.0.0.1:53687/callback";
const MAX_JSON: usize = 8 * 1024 * 1024;
const MAX_DOWNLOAD: u64 = 128 * 1024 * 1024 * 1024;
static AUTH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Drive,
    Onedrive,
    Dropbox,
}
impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Drive => "drive",
            Self::Onedrive => "onedrive",
            Self::Dropbox => "dropbox",
        }
    }
    fn secret(self, kind: &str) -> String {
        format!("cloud-{}-{kind}", self.name())
    }
    fn scope(self) -> &'static str {
        match self {
            Self::Drive => "https://www.googleapis.com/auth/drive.readonly",
            Self::Onedrive => "Files.Read offline_access",
            Self::Dropbox => {
                "files.metadata.read files.content.read sharing.read account_info.read"
            }
        }
    }
    fn authorize(self) -> &'static str {
        match self {
            Self::Drive => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::Onedrive => "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            Self::Dropbox => "https://www.dropbox.com/oauth2/authorize",
        }
    }
    fn token(self) -> &'static str {
        match self {
            Self::Drive => "https://oauth2.googleapis.com/token",
            Self::Onedrive => "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            Self::Dropbox => "https://api.dropboxapi.com/oauth2/token",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub provider: Provider,
    pub account_id: String,
    pub id: String,
    pub export_mime: Option<String>,
    pub version: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFormat {
    pub mime: String,
    pub extension: String,
    pub label: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub selection: Selection,
    pub name: String,
    pub kind: String,
    pub mime: String,
    pub size: Option<u64>,
    pub location: String,
    pub exports: Vec<ExportFormat>,
    pub downloadable: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub entries: Vec<Entry>,
    pub next_cursor: Option<String>,
    pub partial: bool,
    pub warnings: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub provider: Provider,
    pub configured: bool,
    pub connected: bool,
    pub account_id: Option<String>,
    pub scope: &'static str,
    pub redirect_uri: &'static str,
}

#[derive(Serialize, Deserialize)]
struct Config {
    client_id: String,
    client_secret: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Credentials {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
    account_id: String,
}
#[derive(Serialize, Deserialize)]
struct SharedFile {
    account_id: String,
    url: String,
    item_id: String,
}
fn save_shared_file(root: &Path, account: &str, url: &str, id: &str) -> Result<String, String> {
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("{account}\n{url}\n{id}").as_bytes())
    );
    let reference = format!("cloud-dropbox-link-{}", &hash[..40]);
    let mut bytes = serde_json::to_vec(&SharedFile {
        account_id: account.into(),
        url: url.into(),
        item_id: id.into(),
    })
    .map_err(|_| "Could not protect the shared file reference.")?;
    let saved = crate::sessions::save_module_secret(root, &reference, &bytes);
    bytes.fill(0);
    saved?;
    Ok(format!("shared:{}", &hash[..40]))
}
fn shared_file(root: &Path, selection: &Selection) -> Result<Option<SharedFile>, String> {
    let Some(hash) = selection.id.strip_prefix("shared:") else {
        return Ok(None);
    };
    if selection.provider != Provider::Dropbox
        || hash.len() != 40
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid protected shared file reference.".into());
    }
    let mut bytes =
        crate::sessions::load_module_secret(root, &format!("cloud-dropbox-link-{hash}"))?;
    let result: Result<SharedFile, String> = serde_json::from_slice(&bytes)
        .map_err(|_| "The protected shared file reference is invalid.".into());
    bytes.fill(0);
    let result = result?;
    if result.account_id != selection.account_id {
        return Err("This shared file belongs to another authorized account.".into());
    }
    shared(Provider::Dropbox, &result.url)?;
    Ok(Some(result))
}
impl Drop for Credentials {
    fn drop(&mut self) {
        wipe(&mut self.access_token);
        if let Some(v) = &mut self.refresh_token {
            wipe(v);
        }
    }
}
impl Drop for Config {
    fn drop(&mut self) {
        if let Some(v) = &mut self.client_secret {
            wipe(v);
        }
    }
}
fn wipe(value: &mut String) {
    unsafe { value.as_bytes_mut().fill(0) };
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}
fn now() -> u64 {
    crate::queue::now() / 1000
}
fn load<T: serde::de::DeserializeOwned>(
    root: &Path,
    provider: Provider,
    kind: &str,
) -> Result<T, String> {
    let mut bytes = crate::sessions::load_module_secret(root, &provider.secret(kind))?;
    let parsed = serde_json::from_slice(&bytes)
        .map_err(|_| "The protected cloud connection is invalid. Reconnect this provider.".into());
    bytes.fill(0);
    parsed
}
fn save<T: Serialize>(
    root: &Path,
    provider: Provider,
    kind: &str,
    value: &T,
) -> Result<(), String> {
    let mut bytes =
        serde_json::to_vec(value).map_err(|_| "Could not encode the cloud connection.")?;
    let result = crate::sessions::save_module_secret(root, &provider.secret(kind), &bytes);
    bytes.fill(0);
    result
}
fn exists(root: &Path, provider: Provider, kind: &str) -> bool {
    root.join("module-secrets")
        .join(format!("{}.bin", provider.secret(kind)))
        .is_file()
}
fn connection(root: &Path, provider: Provider) -> Result<ConnectionStatus, String> {
    let configured = exists(root, provider, "config");
    let account_id = if exists(root, provider, "tokens") {
        Some(
            load::<Credentials>(root, provider, "tokens")?
                .account_id
                .clone(),
        )
    } else {
        None
    };
    Ok(ConnectionStatus {
        provider,
        configured,
        connected: account_id.is_some(),
        account_id,
        scope: provider.scope(),
        redirect_uri: CALLBACK,
    })
}
pub fn status(root: &Path) -> Result<Vec<ConnectionStatus>, String> {
    [Provider::Drive, Provider::Onedrive, Provider::Dropbox]
        .into_iter()
        .map(|p| connection(root, p))
        .collect()
}
pub fn configure(
    root: &Path,
    provider: Provider,
    client_id: String,
    client_secret: Option<String>,
) -> Result<ConnectionStatus, String> {
    let _guard = AUTH_LOCK
        .try_lock()
        .map_err(|_| "Wait for the active cloud authorization or token refresh to finish.")?;
    if client_id.is_empty()
        || client_id.len() > 512
        || !client_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
    {
        return Err("Enter the client ID registered for your desktop application.".into());
    }
    if client_secret
        .as_ref()
        .is_some_and(|s| s.len() > 1024 || !s.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err("Invalid desktop client metadata.".into());
    }
    if provider != Provider::Drive && client_secret.as_ref().is_some_and(|s| !s.is_empty()) {
        return Err("This provider uses a public PKCE client without a client secret.".into());
    }
    if exists(root, provider, "tokens") {
        return Err("Disconnect this account before changing its OAuth application.".into());
    }
    save(
        root,
        provider,
        "config",
        &Config {
            client_id,
            client_secret: client_secret.filter(|s| !s.is_empty()),
        },
    )?;
    connection(root, provider)
}
pub fn revoke(root: &Path, provider: Provider) -> Result<ConnectionStatus, String> {
    let _guard = AUTH_LOCK
        .try_lock()
        .map_err(|_| "Cancel the active cloud authorization before disconnecting.")?;
    let directory = crate::sessions::secure_module_directory(root, "module-secrets")?;
    let file = directory.join(format!("{}.bin", provider.secret("tokens")));
    match fs::remove_file(file) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Could not remove this local cloud connection.".into()),
    };
    connection(root, provider)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | c.get(2).copied().unwrap_or(0) as u32;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if c.len() > 1 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        }
        if c.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        }
    }
    out
}
fn random() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        if unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        } < 0
        {
            return Err("Windows could not create a secure OAuth challenge.".into());
        }
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|_| "Could not create a secure OAuth challenge.")?;
    }
    Ok(base64(&bytes))
}
fn browser(input: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let value: Vec<_> = std::ffi::OsStr::new(input)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let op: Vec<_> = "open\0".encode_utf16().collect();
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                op.as_ptr(),
                value.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result as isize <= 32 {
            return Err("Could not open the system browser for authorization.".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = input;
        Err("Desktop OAuth is available in the Windows build.".into())
    }
}
async fn cancelled(mut cancel: watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("nabnab/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "Could not prepare the cloud API client.".into())
}
fn endpoint(input: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(input).map_err(|_| "Invalid cloud API endpoint.")?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(
            url.host_str(),
            Some(
                "www.googleapis.com"
                    | "oauth2.googleapis.com"
                    | "login.microsoftonline.com"
                    | "graph.microsoft.com"
                    | "api.dropboxapi.com"
                    | "content.dropboxapi.com"
            )
        )
    {
        return Err("The cloud request is outside the provider's authorized API endpoints.".into());
    }
    Ok(url)
}
async fn read_json(mut response: Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 => "The cloud authorization expired or was revoked. Reconnect this account.",
            403 => "The provider denied access or download permission for this item.",
            404 => "This cloud item is unavailable to the connected account.",
            429 => "The provider rate limit was reached. Wait before retrying.",
            _ => "The cloud provider rejected the request.",
        }
        .into());
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_JSON as u64)
    {
        return Err("Cloud metadata exceeds the 8 MiB limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The cloud metadata transfer failed.")?
    {
        if bytes.len() + chunk.len() > MAX_JSON {
            return Err("Cloud metadata exceeds the 8 MiB limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "The cloud provider returned invalid JSON metadata.".into())
}
async fn token_request(
    provider: Provider,
    config: &Config,
    mut pairs: Vec<(&str, String)>,
    cancel: watch::Receiver<bool>,
) -> Result<Value, String> {
    pairs.push(("client_id", config.client_id.clone()));
    if let Some(secret) = &config.client_secret {
        pairs.push(("client_secret", secret.clone()));
    }
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    let request = client()?
        .post(endpoint(provider.token())?)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(encoded);
    tokio::select! {_=cancelled(cancel)=>Err(crate::engine::CANCELLED.into()),r=async{read_json(request.send().await.map_err(|_|"OAuth token exchange failed.")?).await}=>r}
}
fn credentials(value: &Value, old: Option<&Credentials>) -> Result<Credentials, String> {
    let access_token = value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 16384 && s.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or("The provider returned an invalid access token.")?
        .to_owned();
    let refresh_token = value["refresh_token"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| old.and_then(|o| o.refresh_token.clone()));
    if refresh_token
        .as_ref()
        .is_some_and(|s| s.len() > 16384 || !s.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err("The provider returned an invalid refresh token.".into());
    }
    Ok(Credentials {
        access_token,
        refresh_token,
        expires_at: now().saturating_add(value["expires_in"].as_u64().unwrap_or(3600).min(86400)),
        account_id: old.map(|o| o.account_id.clone()).unwrap_or_default(),
    })
}
async fn access(
    root: &Path,
    provider: Provider,
    cancel: watch::Receiver<bool>,
) -> Result<Credentials, String> {
    let _guard = tokio::select! {
        _ = cancelled(cancel.clone()) => return Err(crate::engine::CANCELLED.into()),
        guard = AUTH_LOCK.lock() => guard,
    };
    let current: Credentials = load(root, provider, "tokens")
        .map_err(|_| "Connect this cloud account before browsing its files.")?;
    if current.expires_at > now() + 60 {
        return Ok(current);
    }
    let refresh = current
        .refresh_token
        .clone()
        .ok_or("Reconnect this account to renew its authorization.")?;
    let config: Config = load(root, provider, "config")?;
    let value = token_request(
        provider,
        &config,
        vec![
            ("grant_type", "refresh_token".into()),
            ("refresh_token", refresh),
        ],
        cancel,
    )
    .await?;
    let updated = credentials(&value, Some(&current))?;
    save(root, provider, "tokens", &updated)?;
    Ok(updated)
}
async fn api(
    token: &str,
    method: Method,
    input: &str,
    body: Option<Value>,
    cancel: watch::Receiver<bool>,
) -> Result<Value, String> {
    let url = endpoint(input)?;
    let mut header = header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "Invalid protected access token.")?;
    header.set_sensitive(true);
    let mut request = client()?
        .request(method, url)
        .header(header::AUTHORIZATION, header);
    if let Some(body) = body {
        request = request
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.to_string());
    }
    tokio::select! {_=cancelled(cancel)=>Err(crate::engine::CANCELLED.into()),r=async{read_json(request.send().await.map_err(|_|"The cloud API request failed.")?).await}=>r}
}

fn callback_code(request: &str, state: &str) -> Result<Option<String>, String> {
    let mut lines = request.split("\r\n");
    let line = lines.next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    if parts.next() != Some("GET") {
        return Ok(None);
    }
    let Some(path) = parts.next() else {
        return Ok(None);
    };
    let url = url::Url::parse(&format!("http://127.0.0.1:53687{path}"))
        .map_err(|_| "Invalid OAuth callback.")?;
    if url.path() != "/callback" || url.host_str() != Some("127.0.0.1") {
        return Ok(None);
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    if pairs.iter().filter(|(k, _)| k == "state").count() != 1
        || pairs
            .iter()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.as_ref())
            != Some(state)
    {
        return Ok(None);
    }
    if pairs.iter().any(|(k, _)| k == "error") {
        return Err("Authorization was denied in the system browser.".into());
    }
    if pairs.iter().filter(|(k, _)| k == "code").count() != 1 {
        return Err("The provider did not return a unique authorization code.".into());
    }
    Ok(pairs
        .into_iter()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .filter(|s| s.len() <= 8192 && !s.chars().any(char::is_control)))
}
pub async fn connect(
    root: PathBuf,
    provider: Provider,
    cancel: watch::Receiver<bool>,
) -> Result<ConnectionStatus, String> {
    let _guard = tokio::select! {
        _ = cancelled(cancel.clone()) => return Err(crate::engine::CANCELLED.into()),
        guard = AUTH_LOCK.lock() => guard,
    };
    let config: Config = load(&root, provider, "config")
        .map_err(|_| "Configure a legitimately registered desktop OAuth client first.")?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:53687")
        .await
        .map_err(|_| {
            "The OAuth loopback port 53687 is busy. Close another authorization attempt and retry."
        })?;
    let state = random()?;
    let verifier = random()?;
    let challenge = base64(&Sha256::digest(verifier.as_bytes()));
    let mut auth =
        url::Url::parse(provider.authorize()).map_err(|_| "Invalid authorization endpoint.")?;
    auth.query_pairs_mut().extend_pairs([
        ("client_id", config.client_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", CALLBACK),
        ("scope", provider.scope()),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    match provider {
        Provider::Drive => {
            auth.query_pairs_mut()
                .extend_pairs([("access_type", "offline"), ("prompt", "consent")]);
        }
        Provider::Dropbox => {
            auth.query_pairs_mut()
                .append_pair("token_access_type", "offline");
        }
        Provider::Onedrive => {
            auth.query_pairs_mut().append_pair("response_mode", "query");
        }
    }
    browser(auth.as_str())?;
    let receive = async {
        loop {
            let (mut stream, peer) = listener
                .accept()
                .await
                .map_err(|_| "The OAuth callback listener failed.")?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let mut request = Vec::new();
            loop {
                let mut buffer = [0u8; 1024];
                let read = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buffer))
                    .await
                    .map_err(|_| "The OAuth callback timed out.")?
                    .map_err(|_| "Could not read the OAuth callback.")?;
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.len() > 16384 {
                    break;
                }
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let code = std::str::from_utf8(&request)
                .ok()
                .map(|s| callback_code(s, &state))
                .transpose()?
                .flatten();
            let message = if code.is_some() {
                "Authorization received. You can close this tab and return to nabnab."
            } else {
                "Invalid authorization callback. Return to nabnab and retry."
            };
            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Security-Policy: default-src 'none'\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",message.len());
            let _ = stream.write_all(response.as_bytes()).await;
            if let Some(code) = code {
                break Ok::<_, String>(code);
            }
        }
    };
    let code = tokio::select! {_=cancelled(cancel.clone())=>return Err(crate::engine::CANCELLED.into()),r=tokio::time::timeout(Duration::from_secs(180),receive)=>r.map_err(|_|"Authorization timed out after three minutes. Retry when ready.")??};
    let value = token_request(
        provider,
        &config,
        vec![
            ("grant_type", "authorization_code".into()),
            ("code", code),
            ("redirect_uri", CALLBACK.into()),
            ("code_verifier", verifier),
        ],
        cancel.clone(),
    )
    .await?;
    let mut tokens = credentials(&value, None)?;
    tokens.account_id = match provider {
        Provider::Drive => {
            let v = api(
                &tokens.access_token,
                Method::GET,
                "https://www.googleapis.com/drive/v3/about?fields=user(permissionId)",
                None,
                cancel,
            )
            .await?;
            required(&v["user"], "permissionId")?
        }
        Provider::Onedrive => {
            let v = api(
                &tokens.access_token,
                Method::GET,
                "https://graph.microsoft.com/v1.0/me/drive?$select=id",
                None,
                cancel,
            )
            .await?;
            required(&v, "id")?
        }
        Provider::Dropbox => {
            let v = api(
                &tokens.access_token,
                Method::POST,
                "https://api.dropboxapi.com/2/users/get_current_account",
                Some(Value::Null),
                cancel,
            )
            .await?;
            required(&v, "account_id")?
        }
    };
    save(&root, provider, "tokens", &tokens)?;
    connection(&root, provider)
}

fn required(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| format!("Cloud metadata has no valid {key}."))
}
fn component(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.len() > 512
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.!:".contains(&b))
    {
        return Err("Invalid cloud item identity.".into());
    }
    Ok(value)
}
fn text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(512)
        .collect()
}
fn export_formats(value: &Value) -> Vec<ExportFormat> {
    value
        .as_object()
        .into_iter()
        .flat_map(|v| v.keys())
        .filter_map(|mime| {
            let (extension, label) = match mime.as_str() {
                "application/pdf" => ("pdf", "PDF"),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                    ("docx", "Word")
                }
                "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
                    ("pptx", "PowerPoint")
                }
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
                    ("xlsx", "Excel")
                }
                "text/plain" => ("txt", "Text"),
                "text/csv" => ("csv", "CSV"),
                "application/epub+zip" => ("epub", "EPUB"),
                "image/png" => ("png", "PNG"),
                "image/jpeg" => ("jpg", "JPEG"),
                "image/svg+xml" => ("svg", "SVG"),
                "application/zip" => ("zip", "HTML ZIP"),
                "text/markdown" => ("md", "Markdown"),
                _ => return None,
            };
            Some(ExportFormat {
                mime: mime.clone(),
                extension: extension.into(),
                label: label.into(),
            })
        })
        .collect()
}
fn drive_entry(value: &Value, account: &str, location: &str) -> Result<Entry, String> {
    let id = required(value, "id")?;
    component(&id)?;
    let mime = required(value, "mimeType")?;
    let kind = if mime == "application/vnd.google-apps.folder" {
        "folder"
    } else if mime == "application/vnd.google-apps.shortcut" {
        "shortcut"
    } else if mime.starts_with("application/vnd.google-apps.") {
        "document"
    } else {
        "file"
    };
    let exports = export_formats(&value["exportLinks"]);
    Ok(Entry {
        selection: Selection {
            provider: Provider::Drive,
            account_id: account.into(),
            id,
            export_mime: None,
            version: value["version"]
                .as_str()
                .or_else(|| value["modifiedTime"].as_str())
                .map(str::to_owned),
        },
        name: text(&required(value, "name")?),
        kind: kind.into(),
        mime,
        size: value["size"].as_str().and_then(|s| s.parse().ok()),
        location: location.into(),
        downloadable: value["capabilities"]["canDownload"].as_bool() == Some(true)
            && match kind {
                "file" => true,
                "document" => !exports.is_empty(),
                _ => false,
            },
        exports,
    })
}
fn onedrive_entry(value: &Value, account: &str, location: &str) -> Result<Entry, String> {
    let actual = if value["remoteItem"].is_object() {
        &value["remoteItem"]
    } else {
        value
    };
    let item = required(actual, "id")?;
    component(&item)?;
    let drive = actual["parentReference"]["driveId"]
        .as_str()
        .unwrap_or(account);
    component(drive)?;
    let folder = actual["folder"].is_object();
    Ok(Entry {
        selection: Selection {
            provider: Provider::Onedrive,
            account_id: account.into(),
            id: format!("{drive}:{item}"),
            export_mime: None,
            version: actual["eTag"].as_str().map(str::to_owned),
        },
        name: text(
            value["name"]
                .as_str()
                .or_else(|| actual["name"].as_str())
                .unwrap_or("Cloud item"),
        ),
        kind: if folder {
            "folder"
        } else if actual["file"].is_object() {
            "file"
        } else {
            "shortcut"
        }
        .into(),
        mime: actual["file"]["mimeType"]
            .as_str()
            .unwrap_or("application/octet-stream")
            .into(),
        size: actual["size"].as_u64(),
        location: location.into(),
        exports: vec![],
        downloadable: !folder && actual["file"].is_object(),
    })
}
fn dropbox_entry(value: &Value, account: &str, location: &str) -> Result<Entry, String> {
    let tag = value[".tag"].as_str().unwrap_or_default();
    let id = required(value, "id")?;
    component(&id)?;
    Ok(Entry {
        selection: Selection {
            provider: Provider::Dropbox,
            account_id: account.into(),
            id,
            export_mime: None,
            version: value["rev"]
                .as_str()
                .or_else(|| value["server_modified"].as_str())
                .map(str::to_owned),
        },
        name: text(&required(value, "name")?),
        kind: if tag == "folder" { "folder" } else { "file" }.into(),
        mime: "application/octet-stream".into(),
        size: value["size"].as_u64(),
        location: value["path_display"].as_str().unwrap_or(location).into(),
        exports: vec![],
        downloadable: tag == "file" && value["is_downloadable"].as_bool() != Some(false),
    })
}
fn drive_metadata_url(id: &str) -> Result<String, String> {
    component(id)?;
    Ok(format!("https://www.googleapis.com/drive/v3/files/{id}?fields=id,name,mimeType,size,version,modifiedTime,capabilities(canDownload),exportLinks&supportsAllDrives=true"))
}
fn graph_item(id: &str, account: &str) -> Result<String, String> {
    if id == "root" {
        component(account)?;
        return Ok(format!(
            "https://graph.microsoft.com/v1.0/drives/{account}/root"
        ));
    }
    let (drive, item) = id
        .split_once(':')
        .ok_or("Choose an item from this account's file browser.")?;
    component(drive)?;
    component(item)?;
    Ok(format!(
        "https://graph.microsoft.com/v1.0/drives/{drive}/items/{item}"
    ))
}
fn shared(provider: Provider, input: &str) -> Result<String, String> {
    let normalized = crate::model::validate_url(input)?;
    let url = url::Url::parse(&normalized).map_err(|_| "Invalid shared cloud link.")?;
    if url.scheme() != "https" {
        return Err("Use the official HTTPS shared link.".into());
    }
    match provider {
        Provider::Drive => {
            if !matches!(url.host_str(), Some("drive.google.com" | "docs.google.com")) {
                return Err("Use an official Google Drive or Docs shared link.".into());
            }
            if url.query_pairs().any(|(k, _)| k == "resourcekey") {
                return Err("This shared link requires a resource key. Add it to the authorized account and select it in the file browser.".into());
            }
            let segments: Vec<_> = url.path_segments().into_iter().flatten().collect();
            let id = segments
                .iter()
                .position(|s| *s == "d" || *s == "folders")
                .and_then(|i| segments.get(i + 1).copied())
                .map(str::to_owned)
                .or_else(|| {
                    url.query_pairs()
                        .find(|(k, _)| k == "id")
                        .map(|(_, v)| v.into_owned())
                })
                .ok_or("Use a Drive file, folder or document shared link.")?;
            component(&id)?;
            Ok(id)
        }
        Provider::Onedrive => {
            if !matches!(url.host_str(), Some("1drv.ms" | "onedrive.live.com"))
                && !url
                    .host_str()
                    .is_some_and(|h| h.ends_with(".sharepoint.com"))
            {
                return Err("Use an official OneDrive or SharePoint shared link.".into());
            }
            Ok(format!("u!{}", base64(normalized.as_bytes())))
        }
        Provider::Dropbox => {
            if !matches!(url.host_str(), Some("www.dropbox.com" | "dropbox.com")) {
                return Err("Use an official Dropbox shared link.".into());
            }
            Ok(normalized)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BrowseLocation {
    Folder(String),
    SharedLink(String),
}
fn browse_location(folder: Option<String>, shared_url: Option<String>) -> BrowseLocation {
    if let Some(folder) = folder {
        BrowseLocation::Folder(folder)
    } else if let Some(link) = shared_url.filter(|s| !s.trim().is_empty()) {
        BrowseLocation::SharedLink(link)
    } else {
        BrowseLocation::Folder("root".into())
    }
}
pub async fn browse(
    root: &Path,
    provider: Provider,
    folder: Option<String>,
    cursor: Option<String>,
    shared_url: Option<String>,
    cancel: watch::Receiver<bool>,
) -> Result<Page, String> {
    if cursor
        .as_ref()
        .is_some_and(|s| s.len() > 16384 || s.chars().any(char::is_control))
    {
        return Err("The cloud pagination cursor is invalid.".into());
    }
    let token = access(root, provider, cancel.clone()).await?;
    let mut warnings = vec![];
    let (mut target, shared_link) = match browse_location(folder, shared_url) {
        BrowseLocation::Folder(folder) => (folder, None),
        BrowseLocation::SharedLink(link) => ("root".into(), Some(link)),
    };
    if let Some(link) = shared_link {
        let reference = shared(provider, &link)?;
        let value = match provider {
            Provider::Drive => {
                api(
                    &token.access_token,
                    Method::GET,
                    &drive_metadata_url(&reference)?,
                    None,
                    cancel.clone(),
                )
                .await?
            }
            Provider::Onedrive => {
                api(
                    &token.access_token,
                    Method::GET,
                    &format!("https://graph.microsoft.com/v1.0/shares/{reference}/driveItem"),
                    None,
                    cancel.clone(),
                )
                .await?
            }
            Provider::Dropbox => {
                api(
                    &token.access_token,
                    Method::POST,
                    "https://api.dropboxapi.com/2/sharing/get_shared_link_metadata",
                    Some(json!({"url":reference})),
                    cancel.clone(),
                )
                .await?
            }
        };
        let mut entry = match provider {
            Provider::Drive => drive_entry(&value, &token.account_id, "Shared link")?,
            Provider::Onedrive => onedrive_entry(&value, &token.account_id, "Shared link")?,
            Provider::Dropbox => dropbox_entry(&value, &token.account_id, "Shared link")?,
        };
        if provider == Provider::Dropbox && entry.kind == "file" {
            entry.selection.id =
                save_shared_file(root, &token.account_id, &link, &entry.selection.id)?;
        } else if provider == Provider::Dropbox {
            warnings.push("Browse shared Dropbox folders after adding them to the connected account. Password-protected shared links require a route that is not available here.".into());
        }
        if entry.kind != "folder" {
            return Ok(Page {
                entries: vec![entry],
                next_cursor: None,
                partial: false,
                warnings,
            });
        }
        target = entry.selection.id;
    }
    let (value, entries, next_cursor) = match provider {
        Provider::Drive => {
            component(&target)?;
            let mut url = url::Url::parse("https://www.googleapis.com/drive/v3/files").unwrap();
            url.query_pairs_mut().extend_pairs([("q",format!("'{target}' in parents and trashed = false")),("pageSize","100".into()),("orderBy","folder,name".into()),("fields","nextPageToken,incompleteSearch,files(id,name,mimeType,size,version,modifiedTime,capabilities(canDownload),exportLinks)".into()),("supportsAllDrives","true".into()),("includeItemsFromAllDrives","true".into())]);
            if let Some(cursor) = cursor {
                url.query_pairs_mut().append_pair("pageToken", &cursor);
            }
            let value = api(&token.access_token, Method::GET, url.as_str(), None, cancel).await?;
            let entries = value["files"]
                .as_array()
                .ok_or("Google Drive did not return a file list.")?
                .iter()
                .take(100)
                .map(|v| drive_entry(v, &token.account_id, &target))
                .collect::<Result<Vec<_>, _>>()?;
            let next = value["nextPageToken"].as_str().map(str::to_owned);
            (value, entries, next)
        }
        Provider::Onedrive => {
            let base = format!("{}/children", graph_item(&target, &token.account_id)?);
            let url = if let Some(cursor) = cursor {
                let parsed = endpoint(&cursor)?;
                if parsed.host_str() != Some("graph.microsoft.com")
                    || parsed.path() != url::Url::parse(&base).unwrap().path()
                {
                    return Err("The OneDrive pagination cursor belongs to another folder.".into());
                }
                cursor
            } else {
                format!("{base}?$top=100&$select=id,name,file,folder,remoteItem,parentReference,size,eTag")
            };
            let value = api(&token.access_token, Method::GET, &url, None, cancel).await?;
            let entries = value["value"]
                .as_array()
                .ok_or("OneDrive did not return a file list.")?
                .iter()
                .take(100)
                .map(|v| onedrive_entry(v, &token.account_id, &target))
                .collect::<Result<Vec<_>, _>>()?;
            let next = value["@odata.nextLink"].as_str().map(str::to_owned);
            (value, entries, next)
        }
        Provider::Dropbox => {
            let (url, body) = if let Some(cursor) = cursor {
                (
                    "https://api.dropboxapi.com/2/files/list_folder/continue",
                    json!({"cursor":cursor}),
                )
            } else {
                if target != "root" {
                    component(&target)?;
                }
                (
                    "https://api.dropboxapi.com/2/files/list_folder",
                    json!({"path":if target=="root"{""}else{target.as_str()},"recursive":false,"include_deleted":false,"include_non_downloadable_files":true,"limit":100}),
                )
            };
            let value = api(&token.access_token, Method::POST, url, Some(body), cancel).await?;
            let entries = value["entries"]
                .as_array()
                .ok_or("Dropbox did not return a file list.")?
                .iter()
                .take(100)
                .filter(|v| v[".tag"] != "deleted")
                .map(|v| dropbox_entry(v, &token.account_id, &target))
                .collect::<Result<Vec<_>, _>>()?;
            let next = if value["has_more"].as_bool() == Some(true) {
                Some(required(&value, "cursor")?)
            } else {
                None
            };
            (value, entries, next)
        }
    };
    let partial = next_cursor.is_some() || value["incompleteSearch"].as_bool() == Some(true);
    if value["incompleteSearch"].as_bool() == Some(true) {
        warnings.push(
            "Google Drive reported an incomplete search. Narrow the folder selection.".into(),
        );
    }
    Ok(Page {
        entries,
        next_cursor,
        partial,
        warnings,
    })
}

pub async fn preview_folder(
    root: &Path,
    selection: &Selection,
    cancel: watch::Receiver<bool>,
) -> Result<Page, String> {
    let tokens = access(root, selection.provider, cancel.clone()).await?;
    if tokens.account_id != selection.account_id {
        return Err("This folder belongs to a different cloud account. Select it again.".into());
    }
    let mut pending = vec![(selection.id.clone(), 0usize)];
    let mut seen = HashSet::new();
    let mut entries = vec![];
    let mut partial = false;
    let mut warnings = vec![];
    let mut pages = 0;
    while let Some((folder, depth)) = pending.pop() {
        if !seen.insert(folder.clone()) {
            warnings.push("A repeated folder reference was skipped.".into());
            continue;
        }
        if depth >= 12 {
            partial = true;
            continue;
        }
        let mut cursor = None;
        loop {
            if pages >= 20 || entries.len() >= 500 {
                partial = true;
                break;
            }
            let page = browse(
                root,
                selection.provider,
                Some(folder.clone()),
                cursor,
                None,
                cancel.clone(),
            )
            .await?;
            pages += 1;
            warnings.extend(page.warnings);
            for entry in page.entries {
                if entry.kind == "folder" {
                    pending.push((entry.selection.id.clone(), depth + 1));
                } else if entry.kind == "shortcut" {
                    warnings.push("A shortcut was not followed automatically.".into());
                } else if entry.downloadable {
                    entries.push(entry);
                    if entries.len() >= 500 {
                        partial = true;
                        break;
                    }
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        if pages >= 20 || entries.len() >= 500 {
            partial = true;
            break;
        }
    }
    Ok(Page {
        entries,
        next_cursor: None,
        partial,
        warnings,
    })
}

pub fn validate_selection(selection: &Selection) -> Result<(), String> {
    component(&selection.id)?;
    if selection.account_id.is_empty()
        || selection.account_id.len() > 512
        || selection.account_id.chars().any(char::is_control)
    {
        return Err("Invalid cloud account identity.".into());
    }
    if selection
        .export_mime
        .as_ref()
        .is_some_and(|s| s.len() > 256 || s.chars().any(char::is_control))
        || selection
            .version
            .as_ref()
            .is_some_and(|s| s.len() > 512 || s.chars().any(char::is_control))
    {
        return Err("Invalid cloud selection metadata.".into());
    }
    Ok(())
}
fn filename(name: &str, extension: Option<&str>) -> String {
    let mut safe: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(160)
        .collect();
    safe = safe.trim_matches([' ', '.']).to_owned();
    if safe.is_empty() {
        safe = "cloud-file".into();
    }
    let stem = safe.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        safe.insert(0, '_');
    }
    if let Some(extension) = extension {
        if !safe
            .to_ascii_lowercase()
            .ends_with(&format!(".{extension}"))
        {
            safe.push('.');
            safe.push_str(extension);
        }
    }
    safe
}
async fn transfer(
    mut response: Response,
    staging: &Path,
    name: &str,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    if !response.status().is_success() {
        return Err(
            "The cloud provider denied the file transfer. Check permission and retry.".into(),
        );
    }
    let expected = response.content_length();
    if expected.is_some_and(|n| n > MAX_DOWNLOAD) {
        return Err("This file exceeds the 128 GiB cloud transfer limit.".into());
    }
    let name = filename(name, None);
    crate::direct::validate_filename(&name)?;
    let path = staging.join(name);
    if fs::symlink_metadata(&path).is_ok() {
        return Err(
            "The cloud staging output already exists. Existing files were preserved.".into(),
        );
    }
    let partial = staging.join(".cloud-source.part");
    let mut file = crate::direct::partial_file(&partial, 0)?;
    let observed_cancel = cancel.clone();
    let operation = async {
        let mut received = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The cloud file transfer was interrupted.")?
        {
            if *observed_cancel.borrow() {
                return Err(crate::engine::CANCELLED.into());
            }
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or("The cloud file is too large.")?;
            if received > expected.unwrap_or(MAX_DOWNLOAD) {
                return Err("The cloud transfer exceeded its declared size or limit.".into());
            }
            file.write_all(&chunk)
                .map_err(|_| "Could not write the downloaded cloud file.")?;
            emit(Progress {
                request_id: run_id,
                phase: "downloading".into(),
                percent: expected
                    .filter(|n| *n > 0)
                    .map(|n| received as f64 / n as f64 * 100.0),
                speed: None,
                downloaded: Some(received as f64),
                total: expected.map(|n| n as f64),
            });
        }
        if expected.is_some_and(|n| n != received) {
            return Err("The cloud file ended before its declared size.".into());
        }
        if *observed_cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        file.sync_all()
            .map_err(|_| "Could not flush the downloaded cloud file.")?;
        Ok::<(), String>(())
    };
    let result =
        tokio::select! {_=cancelled(cancel)=>Err(crate::engine::CANCELLED.into()),r=operation=>r};
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    crate::direct::move_completed(&partial, &path)?;
    Ok(path)
}
pub async fn execute(
    root: &Path,
    selection: &Selection,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<Vec<PathBuf>, String> {
    validate_selection(selection)?;
    let staging = crate::library::authorized_path(staging)?;
    if !staging.is_dir() {
        return Err("Choose a regular cloud staging directory.".into());
    }
    let token = access(root, selection.provider, cancel.clone()).await?;
    if token.account_id != selection.account_id {
        return Err("This job was authorized for another account. Select the file again.".into());
    }
    let (entry, url, method, argument) = match selection.provider {
        Provider::Drive => {
            let metadata = api(
                &token.access_token,
                Method::GET,
                &drive_metadata_url(&selection.id)?,
                None,
                cancel.clone(),
            )
            .await?;
            let mut entry = drive_entry(&metadata, &token.account_id, "")?;
            let url = if entry.kind == "document" {
                let mime = selection
                    .export_mime
                    .as_deref()
                    .ok_or("Choose an export format offered for this native document.")?;
                let format = entry
                    .exports
                    .iter()
                    .find(|f| f.mime == mime)
                    .ok_or("This export format is no longer offered for the document.")?;
                entry.name = filename(&entry.name, Some(&format.extension));
                let mut url = url::Url::parse(&format!(
                    "https://www.googleapis.com/drive/v3/files/{}/export",
                    selection.id
                ))
                .unwrap();
                url.query_pairs_mut().append_pair("mimeType", mime);
                url.to_string()
            } else {
                if selection.export_mime.is_some() {
                    return Err(
                        "A binary Drive file must download its original, not use document export."
                            .into(),
                    );
                }
                format!(
                    "https://www.googleapis.com/drive/v3/files/{}?alt=media&supportsAllDrives=true",
                    selection.id
                )
            };
            (entry, url, Method::GET, None)
        }
        Provider::Onedrive => {
            if selection.export_mime.is_some() {
                return Err(
                    "OneDrive file downloads do not use Google document export formats.".into(),
                );
            }
            let base = graph_item(&selection.id, &token.account_id)?;
            let metadata = api(
                &token.access_token,
                Method::GET,
                &base,
                None,
                cancel.clone(),
            )
            .await?;
            let entry = onedrive_entry(&metadata, &token.account_id, "")?;
            (entry, format!("{base}/content"), Method::GET, None)
        }
        Provider::Dropbox => {
            if selection.export_mime.is_some() {
                return Err(
                    "Dropbox file downloads do not use Google document export formats.".into(),
                );
            }
            let shared = shared_file(root, selection)?;
            let (metadata_url, body, transfer_url, argument) = if let Some(reference) = &shared {
                (
                    "https://api.dropboxapi.com/2/sharing/get_shared_link_metadata",
                    json!({"url":reference.url}),
                    "https://content.dropboxapi.com/2/sharing/get_shared_link_file",
                    json!({"url":reference.url}).to_string(),
                )
            } else {
                (
                    "https://api.dropboxapi.com/2/files/get_metadata",
                    json!({"path":selection.id}),
                    "https://content.dropboxapi.com/2/files/download",
                    json!({"path":selection.id}).to_string(),
                )
            };
            let metadata = api(
                &token.access_token,
                Method::POST,
                metadata_url,
                Some(body),
                cancel.clone(),
            )
            .await?;
            let mut entry = dropbox_entry(&metadata, &token.account_id, "")?;
            if let Some(reference) = shared {
                if entry.selection.id != reference.item_id {
                    return Err("The shared link now points to a different item. Inspect it again before downloading.".into());
                }
                entry.selection.id = selection.id.clone();
            }
            (entry, transfer_url.into(), Method::POST, Some(argument))
        }
    };
    if !entry.downloadable {
        return Err("This item is a folder, shortcut or does not allow downloading. Review its contents first.".into());
    }
    if selection.version.is_some() && selection.version != entry.selection.version {
        return Err(
            "The source changed after selection. Refresh the browser and approve the new version."
                .into(),
        );
    }
    let mut bearer = header::HeaderValue::from_str(&format!("Bearer {}", token.access_token))
        .map_err(|_| "Invalid protected cloud token.")?;
    bearer.set_sensitive(true);
    let mut request = client()?
        .request(method, endpoint(&url)?)
        .timeout(Duration::from_secs(24 * 60 * 60))
        .header(header::AUTHORIZATION, bearer)
        .header(header::ACCEPT_ENCODING, "identity");
    if let Some(argument) = argument {
        request = request.header("Dropbox-API-Arg", argument);
    }
    let response = tokio::select! {_=cancelled(cancel.clone())=>return Err(crate::engine::CANCELLED.into()),r=request.send()=>r.map_err(|_|"The authorized cloud transfer could not start.")?};
    let path = if response.status().is_redirection() {
        if selection.provider != Provider::Onedrive {
            return Err("The provider redirected an authenticated transfer. No token was forwarded; retry or use its official account route.".into());
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or("OneDrive did not return a valid download location.")?;
        let parsed = url::Url::parse(location)
            .map_err(|_| "OneDrive returned an invalid download location.")?;
        if parsed.scheme() != "https" {
            return Err("OneDrive returned an insecure download location.".into());
        }
        crate::direct::download_file(
            location,
            &staging,
            &filename(&entry.name, None),
            cancel,
            run_id,
            emit,
        )
        .await?
    } else {
        transfer(response, &staging, &entry.name, cancel, run_id, emit).await?
    };
    Ok(vec![path])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_folder_navigation_uses_the_chosen_child_and_back_resolves_the_shared_root() {
        let shared = "https://onedrive.live.com/?id=shared-root";
        assert_eq!(
            browse_location(None, Some(shared.into())),
            BrowseLocation::SharedLink(shared.into())
        );
        assert_eq!(
            browse_location(Some("drive-id:child-id".into()), Some(shared.into())),
            BrowseLocation::Folder("drive-id:child-id".into())
        );
        assert_eq!(
            browse_location(Some("drive-id:nested-id".into()), Some(shared.into())),
            BrowseLocation::Folder("drive-id:nested-id".into())
        );
        assert_eq!(
            browse_location(None, Some(shared.into())),
            BrowseLocation::SharedLink(shared.into())
        );
        assert_eq!(
            browse_location(None, None),
            BrowseLocation::Folder("root".into())
        );
        assert_eq!(
            browse_location(None, Some("  ".into())),
            BrowseLocation::Folder("root".into())
        );
    }
    #[test]
    fn pkce_uses_base64url_without_padding() {
        assert_eq!(
            base64(&Sha256::digest(
                b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
            )),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(base64(b"f"), "Zg");
        assert_eq!(base64(b"fo"), "Zm8");
        assert_eq!(base64(b"foo"), "Zm9v");
    }
    #[test]
    fn callbacks_require_state_and_unique_code() {
        assert_eq!(
            callback_code(
                "GET /callback?code=code&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53687\r\n\r\n",
                "expected"
            )
            .unwrap(),
            Some("code".into())
        );
        assert!(callback_code(
            "GET /callback?code=private&state=wrong HTTP/1.1\r\n\r\n",
            "expected"
        )
        .unwrap()
        .is_none());
        assert!(callback_code(
            "GET /callback?code=a&code=b&state=expected HTTP/1.1\r\n\r\n",
            "expected"
        )
        .is_err());
        assert!(callback_code(
            "GET /callback?error=access_denied&state=expected HTTP/1.1\r\n\r\n",
            "expected"
        )
        .unwrap_err()
        .contains("denied"));
    }
    #[test]
    fn api_tokens_cannot_be_forwarded_to_other_origins() {
        for id in [".", "..", "file/path", "file?query=1", "file#fragment"] {
            assert!(component(id).is_err());
        }
        for url in [
            "https://graph.microsoft.com.evil.test/v1.0/me",
            "http://graph.microsoft.com/v1.0/me",
            "https://graph.microsoft.com:8443/v1.0/me",
            "https://token@graph.microsoft.com/v1.0/me",
            "https://127.0.0.1/",
        ] {
            assert!(endpoint(url).is_err());
        }
        assert!(endpoint("https://graph.microsoft.com/v1.0/me/drive").is_ok());
    }
    #[test]
    fn drive_exports_are_only_api_offered_supported_formats() {
        let entry=drive_entry(&json!({"id":"doc-id","name":"Report","mimeType":"application/vnd.google-apps.document","capabilities":{"canDownload":true},"exportLinks":{"application/pdf":"https://secret.invalid/","application/vnd.openxmlformats-officedocument.wordprocessingml.document":"secret","application/x-unknown":"secret"},"version":"4"}),"account","root").unwrap();
        assert_eq!(entry.exports.len(), 2);
        assert!(entry.exports.iter().any(|f| f.extension == "docx"));
        assert!(!entry.exports.iter().any(|f| f.extension == "xlsx"));
        let encoded = serde_json::to_string(&entry).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(entry.downloadable);
    }
    #[test]
    fn shortcuts_and_restricted_files_are_not_downloadable() {
        assert!(!drive_entry(&json!({"id":"shortcut","name":"Loop","mimeType":"application/vnd.google-apps.shortcut","capabilities":{"canDownload":true}}),"account","root").unwrap().downloadable);
        assert!(
            !dropbox_entry(
                &json!({".tag":"file","id":"id:file","name":"restricted","is_downloadable":false}),
                "account",
                "root"
            )
            .unwrap()
            .downloadable
        );
    }
    #[test]
    fn shared_links_use_official_identity_not_parameter_guessing() {
        assert_eq!(
            shared(
                Provider::Drive,
                "https://docs.google.com/document/d/doc-id/edit"
            )
            .unwrap(),
            "doc-id"
        );
        assert!(shared(
            Provider::Drive,
            "https://drive.google.com/file/d/doc-id/view?resourcekey=secret"
        )
        .is_err());
        assert!(shared(Provider::Dropbox, "https://evil.test/file").is_err());
        assert!(shared(Provider::Onedrive, "https://1drv.ms/u/some-share")
            .unwrap()
            .starts_with("u!"));
    }
    #[test]
    fn filenames_cannot_escape_staging_or_use_device_names() {
        assert_eq!(filename("../CON", None), "_CON");
        assert_eq!(filename("CON.txt", None), "_CON.txt");
        assert_eq!(filename("Report", Some("pdf")), "Report.pdf");
        assert_eq!(filename("a.pdf", Some("pdf")), "a.pdf");
    }
    fn stage() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "nabnab-cloud-{}-{}",
            crate::queue::now(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }
    async fn fixture(raw: &'static [u8]) -> Response {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::Read;
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(raw);
        });
        client()
            .unwrap()
            .get(format!("http://{address}/fixture"))
            .send()
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn transfer_preserves_bytes_and_reports_real_progress() {
        let response =
            fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npackage")
                .await;
        let directory = stage();
        let (_sender, cancel) = watch::channel(false);
        let progress = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = progress.clone();
        let path = transfer(response, &directory, "archive.zip", cancel, 17, move |p| {
            captured.lock().unwrap().push(p)
        })
        .await
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"package");
        assert_eq!(path.file_name().unwrap(), "archive.zip");
        let progress = progress.lock().unwrap();
        assert_eq!(progress.last().unwrap().downloaded, Some(7.0));
        assert_eq!(progress.last().unwrap().percent, Some(100.0));
        fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn transfer_cancellation_and_truncation_leave_no_partial_file() {
        let directory = stage();
        let response =
            fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npackage")
                .await;
        let (_sender, cancel) = watch::channel(true);
        assert_eq!(
            transfer(response, &directory, "cancelled.zip", cancel, 1, |_| {})
                .await
                .unwrap_err(),
            crate::engine::CANCELLED
        );
        assert!(!directory.join("cancelled.zip").exists());
        let response =
            fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort")
                .await;
        let (_sender, cancel) = watch::channel(false);
        assert!(
            transfer(response, &directory, "truncated.zip", cancel, 1, |_| {})
                .await
                .is_err()
        );
        assert!(!directory.join("truncated.zip").exists());
        fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn oversized_transfer_and_collisions_preserve_existing_files() {
        let directory = stage();
        fs::write(directory.join("existing.zip"), b"original").unwrap();
        let response =
            fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npackage")
                .await;
        let (_sender, cancel) = watch::channel(false);
        assert!(
            transfer(response, &directory, "existing.zip", cancel, 1, |_| {})
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(directory.join("existing.zip")).unwrap(),
            b"original"
        );
        let response = fixture(
            b"HTTP/1.1 200 OK\r\nContent-Length: 137438953473\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (_sender, cancel) = watch::channel(false);
        assert!(
            transfer(response, &directory, "too-large.zip", cancel, 1, |_| {})
                .await
                .is_err()
        );
        assert!(!directory.join("too-large.zip").exists());
        fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn transfer_retries_owned_partial_and_rejects_queue_control_filenames() {
        let directory = stage();
        fs::write(directory.join(".cloud-source.part"), b"interrupted").unwrap();
        let response =
            fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npackage")
                .await;
        let (_sender, cancel) = watch::channel(false);
        let output = transfer(response, &directory, "retried.zip", cancel, 1, |_| {})
            .await
            .unwrap();
        assert_eq!(fs::read(output).unwrap(), b"package");
        assert!(!directory.join(".cloud-source.part").exists());
        for name in ["owner.json", "publication.json", "publication.new"] {
            let response = fixture(
                b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npackage",
            )
            .await;
            let (_sender, cancel) = watch::channel(false);
            assert!(transfer(response, &directory, name, cancel, 1, |_| {})
                .await
                .is_err());
            assert!(!directory.join(name).exists());
        }
        fs::remove_dir_all(directory).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn shared_references_are_protected_and_bound_to_the_authorized_account() {
        let directory = stage();
        let id = save_shared_file(
            &directory,
            "account",
            "https://www.dropbox.com/scl/fi/private/notes.txt?rlkey=private",
            "id:file",
        )
        .unwrap();
        let selection = Selection {
            provider: Provider::Dropbox,
            account_id: "account".into(),
            id,
            export_mime: None,
            version: Some("revision".into()),
        };
        let encoded = serde_json::to_string(&selection).unwrap();
        assert!(!encoded.contains("private"));
        assert!(!encoded.contains("dropbox.com"));
        let loaded = shared_file(&directory, &selection).unwrap().unwrap();
        assert_eq!(loaded.item_id, "id:file");
        let token_file = fs::read_dir(directory.join("module-secrets"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let bytes = fs::read(token_file).unwrap();
        assert!(!bytes.windows(7).any(|w| w == b"private"));
        let wrong = Selection {
            account_id: "different".into(),
            ..selection
        };
        assert!(shared_file(&directory, &wrong).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
