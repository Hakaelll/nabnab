use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

pub const PORT: u16 = 49587;
const HOST: &str = "127.0.0.1:49587";
const MAX_BODY: usize = 16 * 1024;
const MAX_HEADERS: usize = 8 * 1024;

#[derive(Default)]
pub struct Bridge {
    state: Arc<Shared>,
    worker: Mutex<Option<Worker>>,
}
#[derive(Default)]
struct Shared {
    clients: Mutex<Option<BTreeMap<String, String>>>,
    enabled: AtomicBool,
}
struct Worker {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    pub clients: Vec<String>,
    pub port: u16,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub token: String,
    pub port: u16,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserRequest {
    pub version: u32,
    pub request_id: String,
    pub url: String,
    pub profile: String,
    #[serde(default)]
    pub intent: BrowserIntent,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub preset_id: Option<String>,
    #[serde(skip)]
    pub client_id: String,
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserIntent {
    #[default]
    Download,
    Prepare,
    Save,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub version: u32,
    pub request_id: String,
    pub accepted_job_ids: Vec<String>,
    pub accepted_draft_id: Option<String>,
    pub intent: BrowserIntent,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedClients {
    schema_version: u32,
    clients: BTreeMap<String, String>,
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| (b'a'..=b'p').contains(&c))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}
fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        let result = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if result < 0 {
            return Err("Windows could not generate a secure pairing token.".into());
        }
    }
    #[cfg(not(windows))]
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| "The operating system could not generate a secure pairing token.")?;
    Ok(hex(&bytes))
}
fn clients_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(root) = crate::test_data_dir() {
        return Ok(root.join("browser-clients.json"));
    }
    app.path()
        .app_config_dir()
        .map(|p| p.join("browser-clients.json"))
        .map_err(|e| e.to_string())
}
fn read_clients(path: &std::path::Path) -> Result<BTreeMap<String, String>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("Could not read paired browser clients: {error}")),
    };
    if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 {
        return Err("The paired browser client settings are too large.".into());
    }
    let saved: SavedClients = serde_json::from_reader(file).map_err(|_| "Paired browser client settings are invalid. Restore their backup or remove this settings file to pair again.")?;
    if saved.schema_version != 1
        || saved.clients.len() > 32
        || saved.clients.iter().any(|(id, hash)| {
            !valid_id(id) || hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit())
        })
    {
        return Err(
            "Paired browser client settings have an unsupported schema or invalid client.".into(),
        );
    }
    Ok(saved.clients)
}
fn load(app: &AppHandle, bridge: &Bridge) -> Result<(), String> {
    let mut clients = bridge
        .state
        .clients
        .lock()
        .map_err(|_| "Browser pairing state is unavailable.")?;
    if clients.is_none() {
        *clients = Some(read_clients(&clients_path(app)?)?);
    }
    Ok(())
}
fn save_clients(app: &AppHandle, clients: &BTreeMap<String, String>) -> Result<(), String> {
    let path = clients_path(app)?;
    fs::create_dir_all(
        path.parent()
            .ok_or("The browser settings location is invalid.")?,
    )
    .map_err(|e| e.to_string())?;
    let temporary = path.with_extension("new");
    let saved = SavedClients {
        schema_version: 1,
        clients: clients.clone(),
    };
    let bytes = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
    let mut file = File::create(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    read_clients(&temporary)?;
    fs::rename(&temporary, &path).map_err(|e| format!("Could not save paired browser clients: {e}"))
}

pub fn status(bridge: &Bridge) -> Status {
    Status {
        enabled: bridge.state.enabled.load(Ordering::SeqCst),
        clients: bridge
            .state
            .clients
            .lock()
            .ok()
            .and_then(|c| c.as_ref().map(|m| m.keys().cloned().collect()))
            .unwrap_or_default(),
        port: PORT,
    }
}
pub fn pair(app: &AppHandle, bridge: &Bridge, extension_id: &str) -> Result<Pairing, String> {
    if !valid_id(extension_id) {
        return Err(
            "Copy the extension ID from its settings. It must contain 32 letters from a to p."
                .into(),
        );
    }
    load(app, bridge)?;
    let token = new_token()?;
    {
        let mut guard = bridge
            .state
            .clients
            .lock()
            .map_err(|_| "Browser pairing state is unavailable.")?;
        let mut clients = guard.clone().unwrap_or_default();
        if clients.len() >= 32 && !clients.contains_key(extension_id) {
            return Err("Revoke an existing browser client before pairing another.".into());
        }
        clients.insert(extension_id.into(), token_hash(&token));
        save_clients(app, &clients)?;
        *guard = Some(clients);
    }
    start(app, bridge)?;
    Ok(Pairing { token, port: PORT })
}
pub fn revoke(app: &AppHandle, bridge: &Bridge, extension_id: &str) -> Result<(), String> {
    if !valid_id(extension_id) {
        return Err("Invalid browser extension ID.".into());
    }
    load(app, bridge)?;
    let empty = {
        let mut guard = bridge
            .state
            .clients
            .lock()
            .map_err(|_| "Browser pairing state is unavailable.")?;
        let mut clients = guard.clone().unwrap_or_default();
        clients.remove(extension_id);
        save_clients(app, &clients)?;
        let empty = clients.is_empty();
        *guard = Some(clients);
        empty
    };
    if empty {
        stop(bridge);
    }
    Ok(())
}
pub fn start(app: &AppHandle, bridge: &Bridge) -> Result<(), String> {
    load(app, bridge)?;
    if bridge
        .state
        .clients
        .lock()
        .map_err(|_| "Browser pairing state is unavailable.")?
        .as_ref()
        .is_none_or(|c| c.is_empty())
    {
        return Ok(());
    }
    let mut worker = bridge
        .worker
        .lock()
        .map_err(|_| "The browser bridge is unavailable.")?;
    if worker
        .as_ref()
        .is_some_and(|worker| worker.thread.is_finished())
    {
        *worker = None;
    }
    if worker.is_some() {
        return Ok(());
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, PORT))
        .map_err(|e| format!("Could not start the local browser bridge on port {PORT}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let state = bridge.state.clone();
    let app = app.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    state.enabled.store(true, Ordering::SeqCst);
    let thread = thread::Builder::new()
        .name("nabnab-browser".into())
        .spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, address)) if address.ip().is_loopback() => {
                        if stopping.load(Ordering::SeqCst) {
                            break;
                        }
                        if prepare_stream(&stream).is_err() {
                            continue;
                        }
                        serve(&app, &state, &mut stream);
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50))
                    }
                    Err(_) => break,
                }
            }
            state.enabled.store(false, Ordering::SeqCst);
        })
        .map_err(|e| {
            bridge.state.enabled.store(false, Ordering::SeqCst);
            e.to_string()
        })?;
    *worker = Some(Worker { stop, thread });
    Ok(())
}
pub fn stop(bridge: &Bridge) {
    if let Ok(mut guard) = bridge.worker.lock() {
        if let Some(worker) = guard.take() {
            worker.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect_timeout(
                &std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, PORT)),
                Duration::from_millis(100),
            );
            let _ = worker.thread.join();
        }
    }
    bridge.state.enabled.store(false, Ordering::SeqCst);
}

fn prepare_stream(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))
}

struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
fn read_request(stream: &mut impl Read) -> Result<HttpRequest, u16> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        if Instant::now() >= deadline {
            return Err(400);
        }
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        if bytes.len() >= MAX_HEADERS {
            return Err(431);
        }
        let count = stream.read(&mut chunk).map_err(|_| 400u16)?;
        if count == 0 {
            return Err(400);
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    if header_end > MAX_HEADERS {
        return Err(431);
    }
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|_| 400u16)?;
    if !header.is_ascii() {
        return Err(400);
    }
    let mut lines = header.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let method = first.next().ok_or(400u16)?.to_owned();
    let path = first.next().ok_or(400u16)?.to_owned();
    if first.next() != Some("HTTP/1.1") || first.next().is_some() {
        return Err(400);
    }
    let mut headers = BTreeMap::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(400u16)?;
        if name.is_empty()
            || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || value.bytes().any(|c| c < 0x20 && c != b'\t')
        {
            return Err(400);
        }
        if headers
            .insert(name.to_ascii_lowercase(), value.trim().to_owned())
            .is_some()
        {
            return Err(400);
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(400);
    }
    let length = match headers.get("content-length") {
        Some(value) if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
            value.parse::<usize>().map_err(|_| 413u16)?
        }
        Some(_) => return Err(400),
        None => 0,
    };
    if length > MAX_BODY {
        return Err(413);
    }
    let mut body = bytes[header_end..].to_vec();
    if body.len() > length {
        return Err(400);
    }
    while body.len() < length {
        if Instant::now() >= deadline {
            return Err(400);
        }
        let remaining = (length - body.len()).min(chunk.len());
        let count = stream.read(&mut chunk[..remaining]).map_err(|_| 400u16)?;
        if count == 0 {
            return Err(400);
        }
        body.extend_from_slice(&chunk[..count]);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}
fn allowed_origin(
    request: &HttpRequest,
    clients: &BTreeMap<String, String>,
) -> Result<(String, String), u16> {
    if request.headers.get("host").map(String::as_str) != Some(HOST) {
        return Err(403);
    }
    let origin = request.headers.get("origin").ok_or(403u16)?;
    let id = origin
        .strip_prefix("chrome-extension://")
        .filter(|id| valid_id(id))
        .ok_or(403u16)?;
    if !clients.contains_key(id) {
        return Err(403);
    }
    Ok((origin.clone(), id.to_owned()))
}
fn authenticated(request: &HttpRequest, expected: &str) -> bool {
    let Some(token) = request
        .headers
        .get("authorization")
        .and_then(|s| s.strip_prefix("Bearer "))
    else {
        return false;
    };
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let actual = token_hash(token);
    actual.len() == expected.len()
        && actual
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |different, (a, b)| different | (a ^ b))
            == 0
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HealthRequest {
    version: u32,
}
fn validate_health(request: &HttpRequest) -> Result<(), u16> {
    match request.method.as_str() {
        "GET" if request.body.is_empty() => Ok(()),
        "POST" => {
            if request.headers.get("content-type").map(String::as_str) != Some("application/json") {
                return Err(415);
            }
            let body: HealthRequest = serde_json::from_slice(&request.body).map_err(|_| 400u16)?;
            if body.version != 1 {
                return Err(400);
            }
            Ok(())
        }
        _ => Err(405),
    }
}
fn validate_message(mut request: BrowserRequest, client: &str) -> Result<BrowserRequest, String> {
    if request.version != 1 {
        return Err("This extension protocol version is unsupported.".into());
    }
    if request.request_id.len() < 8
        || request.request_id.len() > 128
        || !request
            .request_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    {
        return Err("The browser request ID is invalid.".into());
    }
    if !matches!(request.profile.as_str(), "mp4" | "mp3" | "source" | "audio") {
        return Err("The browser download profile is invalid.".into());
    }
    if request.url.len() > 8192 {
        return Err("The browser URL is too long.".into());
    }
    request.url = crate::model::validate_url(&request.url)?;
    if request.urls.len() > 200 {
        return Err("Send at most 200 selected links.".into());
    }
    request.urls = request
        .urls
        .iter()
        .map(|url| crate::model::validate_url(url))
        .collect::<Result<_, _>>()?;
    if request
        .preset_id
        .as_ref()
        .is_some_and(|id| crate::queue::validate_request_id(id).is_err())
    {
        return Err("Choose a valid local preset.".into());
    }
    if request.intent == BrowserIntent::Download && !request.urls.is_empty() {
        return Err("Send selections to the preparer before downloading.".into());
    }
    request.client_id = client.into();
    Ok(request)
}
fn write_response(
    stream: &mut impl Write,
    status: u16,
    origin: Option<&str>,
    body: &serde_json::Value,
) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        _ => "Internal Server Error",
    };
    let body = if status == 204 {
        Vec::new()
    } else {
        serde_json::to_vec(body).unwrap_or_default()
    };
    let mut header = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n", body.len());
    if let Some(origin) = origin {
        header.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\n"
        ));
        if status == 204 {
            header.push_str("Access-Control-Allow-Methods: GET, POST\r\nAccess-Control-Allow-Headers: Authorization, Content-Type\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Max-Age: 300\r\n");
        }
    }
    header.push_str("\r\n");
    let _ = stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(&body));
}
fn serve(app: &AppHandle, state: &Shared, stream: &mut TcpStream) {
    use serde_json::json;
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(status) => {
            write_response(
                stream,
                status,
                None,
                &json!({"error":"Invalid HTTP request."}),
            );
            return;
        }
    };
    let clients = match state.clients.lock() {
        Ok(clients) => clients.clone().unwrap_or_default(),
        Err(_) => {
            write_response(
                stream,
                500,
                None,
                &json!({"error":"Browser pairing state is unavailable."}),
            );
            return;
        }
    };
    let (origin, client) = match allowed_origin(&request, &clients) {
        Ok(allowed) => allowed,
        Err(status) => {
            write_response(
                stream,
                status,
                None,
                &json!({"error":"The browser client is not paired."}),
            );
            return;
        }
    };
    if !matches!(request.path.as_str(), "/health" | "/enqueue" | "/presets") {
        write_response(
            stream,
            404,
            Some(&origin),
            &json!({"error":"Unknown endpoint."}),
        );
        return;
    }
    if request.method == "OPTIONS" {
        let allowed_method = matches!(
            (
                request.path.as_str(),
                request
                    .headers
                    .get("access-control-request-method")
                    .map(String::as_str)
            ),
            ("/health" | "/presets", Some("GET" | "POST")) | ("/enqueue", Some("POST"))
        );
        let valid = request.body.is_empty()
            && allowed_method
            && request
                .headers
                .get("access-control-request-headers")
                .is_some_and(|value| {
                    value.split(',').all(|header| {
                        matches!(
                            header.trim().to_ascii_lowercase().as_str(),
                            "authorization" | "content-type"
                        )
                    })
                });
        write_response(
            stream,
            if valid { 204 } else { 403 },
            Some(&origin),
            &json!({"error":"Invalid browser preflight."}),
        );
        return;
    }
    if !clients
        .get(&client)
        .is_some_and(|hash| authenticated(&request, hash))
    {
        write_response(
            stream,
            401,
            Some(&origin),
            &json!({"error":"Pair nabnab again. The browser token was rejected."}),
        );
        return;
    }
    if request.path == "/health" || request.path == "/presets" {
        if let Err(status) = validate_health(&request) {
            write_response(
                stream,
                status,
                Some(&origin),
                &json!({"error":"Use POST with the supported JSON protocol version for connection status."}),
            );
            return;
        }
        if request.path == "/presets" {
            let shared = app.state::<Arc<crate::AppState>>();
            let guard = shared.repository.lock().unwrap();
            if let Some(repo) = guard.as_ref() {
                let presets: Vec<_> = repo
                    .data
                    .presets
                    .iter()
                    .map(|p| json!({"id":p.id,"name":p.name,"revision":p.revision}))
                    .collect();
                write_response(
                    stream,
                    200,
                    Some(&origin),
                    &json!({"version":1,"presets":presets}),
                );
            } else {
                write_response(
                    stream,
                    503,
                    Some(&origin),
                    &json!({"error":"Open nabnab first."}),
                );
            }
            return;
        }
        write_response(
            stream,
            200,
            Some(&origin),
            &json!({"version":1,"product":"nabnab","connected":true}),
        );
        return;
    }
    if request.method != "POST" {
        write_response(
            stream,
            405,
            Some(&origin),
            &json!({"error":"Use POST to send a link."}),
        );
        return;
    }
    if request.headers.get("content-type").map(String::as_str) != Some("application/json") {
        write_response(
            stream,
            415,
            Some(&origin),
            &json!({"error":"Use application/json."}),
        );
        return;
    }
    let message = serde_json::from_slice::<BrowserRequest>(&request.body)
        .map_err(|_| "The browser request does not match the protocol.".to_owned())
        .and_then(|message| validate_message(message, &client));
    let message = match message {
        Ok(message) => message,
        Err(error) => {
            write_response(stream, 400, Some(&origin), &json!({"error":error}));
            return;
        }
    };
    match tauri::async_runtime::block_on(crate::accept_browser(app.clone(), message)) {
        Ok(receipt) => write_response(
            stream,
            200,
            Some(&origin),
            &serde_json::to_value(receipt)
                .unwrap_or_else(|_| json!({"error":"Could not acknowledge the request."})),
        ),
        Err(error) => write_response(
            stream,
            400,
            Some(&origin),
            &json!({"error":crate::failure::sanitize(&error)}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const ID: &str = "abcdefghijklmnopabcdefghijklmnop";
    const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    fn accepted_connections_wait_for_a_delayed_request_instead_of_returning_bad_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            thread::sleep(Duration::from_millis(50));
            stream
                .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1:49587\r\n\r\n")
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(true).unwrap();
        prepare_stream(&stream).unwrap();
        let response = read_request(&mut stream);
        client.join().unwrap();
        assert_eq!(response.unwrap().path, "/health");
    }
    fn request(host: &str, origin: &str, token: &str) -> HttpRequest {
        HttpRequest {
            method: "GET".into(),
            path: "/health".into(),
            body: Vec::new(),
            headers: BTreeMap::from([
                ("host".into(), host.into()),
                ("origin".into(), origin.into()),
                ("authorization".into(), format!("Bearer {token}")),
            ]),
        }
    }
    #[test]
    fn only_exact_paired_extension_origins_and_loopback_hosts_are_allowed() {
        let clients = BTreeMap::from([(ID.into(), token_hash(TOKEN))]);
        let origin = format!("chrome-extension://{ID}");
        assert_eq!(
            allowed_origin(&request(HOST, &origin, TOKEN), &clients).unwrap(),
            (origin.clone(), ID.into())
        );
        for bad in [
            format!("{origin}.evil"),
            format!("{origin}/"),
            "https://example.com".into(),
            "null".into(),
        ] {
            assert!(allowed_origin(&request(HOST, &bad, TOKEN), &clients).is_err());
        }
        for host in [
            "localhost:49587",
            "127.0.0.1:49587.evil",
            "evil.test:49587",
            "[::1]:49587",
        ] {
            assert!(allowed_origin(&request(host, &origin, TOKEN), &clients).is_err());
        }
        assert!(!authenticated(
            &request(HOST, &origin, "wrong"),
            &token_hash(TOKEN)
        ));
        assert!(authenticated(
            &request(HOST, &origin, TOKEN),
            &token_hash(TOKEN)
        ));
        assert!(!authenticated(
            &request(HOST, &origin, TOKEN),
            &token_hash(&"b".repeat(64))
        ));
    }
    #[test]
    fn bounded_http_parser_rejects_smuggling_and_oversized_messages() {
        let raw = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1:49587\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(read_request(&mut &raw[..]).unwrap().path, "/health");
        for raw in [
            "POST /enqueue HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nxx",
            "POST /enqueue HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n",
            "POST /enqueue HTTP/1.1\r\nHost: x\r\nContent-Length: 16385\r\n\r\n",
            "POST /enqueue HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\n\r\nxx",
            "POST /enqueue HTTP/1.1\r\nHost: x\r\nContent-Length: +1\r\n\r\nx",
            "GET /health HTTP/1.0\r\nHost: x\r\n\r\n",
        ] {
            assert!(read_request(&mut raw.as_bytes()).is_err(), "{raw}");
        }
        let huge = format!(
            "GET /health HTTP/1.1\r\nX-Test: {}\r\n\r\n",
            "a".repeat(MAX_HEADERS)
        );
        assert!(read_request(&mut huge.as_bytes()).is_err());
    }
    #[test]
    fn browser_protocol_rejects_untrusted_fields_versions_profiles_and_urls() {
        let value = json!({"version":1,"requestId":"request-123","url":"https://youtube.com/watch?v=public","profile":"mp4"});
        let parsed = validate_message(serde_json::from_value(value.clone()).unwrap(), ID).unwrap();
        assert_eq!(parsed.client_id, ID);
        assert_eq!(parsed.request_id, "request-123");
        for (field, bad) in [
            ("version", json!(2)),
            ("requestId", json!("x:y")),
            ("url", json!("http://127.0.0.1/secret")),
            ("profile", json!("--exec")),
        ] {
            let mut invalid = value.clone();
            invalid[field] = bad;
            assert!(validate_message(serde_json::from_value(invalid).unwrap(), ID).is_err());
        }
        let mut invalid = value;
        invalid["token"] = json!("secret");
        assert!(serde_json::from_value::<BrowserRequest>(invalid).is_err());
    }
    #[test]
    fn responses_have_exact_cors_no_discovery_and_no_wildcard() {
        let mut bytes = Vec::new();
        let origin = format!("chrome-extension://{ID}");
        write_response(&mut bytes, 204, Some(&origin), &json!({}));
        let response = String::from_utf8(bytes).unwrap();
        assert!(response.contains(&format!("Access-Control-Allow-Origin: {origin}\r\n")));
        assert!(!response.contains("Allow-Origin: *"));
        assert!(!response.contains(TOKEN));
        let secret = new_token().unwrap();
        assert_eq!(secret.len(), 64);
        assert_ne!(new_token().unwrap(), secret);
        assert!(!serde_json::to_string(&SavedClients {
            schema_version: 1,
            clients: BTreeMap::from([(ID.into(), token_hash(&secret))])
        })
        .unwrap()
        .contains(&secret));
    }
    #[test]
    fn browser_health_post_is_versioned_and_get_compatibility_stays_strict() {
        let mut probe = request(HOST, &format!("chrome-extension://{ID}"), TOKEN);
        assert!(validate_health(&probe).is_ok());
        probe.method = "POST".into();
        probe
            .headers
            .insert("content-type".into(), "application/json".into());
        probe.body = br#"{"version":1}"#.to_vec();
        assert!(validate_health(&probe).is_ok());
        for body in [
            br#"{"version":2}"#.as_slice(),
            br#"{"version":1,"token":"untrusted"}"#.as_slice(),
            b"".as_slice(),
        ] {
            probe.body = body.to_vec();
            assert_eq!(validate_health(&probe), Err(400));
        }
        probe.body = br#"{"version":1}"#.to_vec();
        probe.headers.remove("content-type");
        assert_eq!(validate_health(&probe), Err(415));
        probe.method = "DELETE".into();
        assert_eq!(validate_health(&probe), Err(405));
    }
}
