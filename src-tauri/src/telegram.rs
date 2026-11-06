use crate::model::Progress;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::sync::watch;

type SharedClient = Arc<Mutex<Native>>;
static CLIENTS: OnceLock<Mutex<HashMap<String, SharedClient>>> = OnceLock::new();

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Configuration {
    api_id: i32,
    api_hash: String,
    encryption_key: String,
}
struct SecretBytes(Vec<u8>);
impl Drop for SecretBytes {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Authorization {
    Disconnected,
    Initializing,
    Phone,
    Code,
    Password { hint: String },
    Qr { svg: String },
    Ready,
    Closing,
    Closed,
    Unsupported { state: String },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub authorization: Authorization,
    pub version: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    pub accounts: Vec<Account>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub protected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramAttachment {
    pub account_id: String,
    pub chat_id: String,
    pub message_id: String,
    pub file_id: i32,
    pub id: String,
    pub title: String,
    pub filename: String,
    pub kind: String,
    pub size: Option<u64>,
    pub date: i64,
    pub can_save: bool,
    pub reason: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPage {
    pub items: Vec<TelegramAttachment>,
    pub next_message_id: String,
    pub inspected: usize,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AuthAction {
    Phone { value: String },
    Code { value: String },
    Password { value: String },
    Qr,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaQuery {
    pub account_id: String,
    pub chat_id: String,
    pub from_message_id: String,
    pub limit: u32,
    pub kind: String,
    pub query: String,
    pub start_date: Option<i64>,
    pub end_date: Option<i64>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TdSchema {
    Legacy180,
    Current1869,
}
impl TdSchema {
    fn parse(version: &str) -> Result<Self, String> {
        match version {
            "1.8.0" => Ok(Self::Legacy180),
            "1.8.69" => Ok(Self::Current1869),
            _ if version.len() <= 32 && version.split('.').count() == 3 && version.split('.').all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())) => Err(format!("TDLib {version} has no verified API schema in nabnab. Use the documented 1.8.0 or 1.8.69 build.")),
            _ => Err("TDLib returned an unsupported version identifier. Use the documented 1.8.0 or 1.8.69 build.".into()),
        }
    }
    fn version(self) -> &'static str {
        match self {
            Self::Legacy180 => "1.8.0",
            Self::Current1869 => "1.8.69",
        }
    }
}
fn initialization(schema: TdSchema, directory: &Path, config: &Configuration) -> Value {
    let parameters = json!({"use_test_dc":false,"database_directory":directory.join("database"),"files_directory":directory.join("files"),"use_file_database":true,"use_chat_info_database":true,"use_message_database":true,"use_secret_chats":false,"api_id":config.api_id,"api_hash":config.api_hash,"system_language_code":"es","device_model":"nabnab","system_version":"Windows","application_version":env!("CARGO_PKG_VERSION")});
    match schema {
        TdSchema::Legacy180 => {
            let mut parameters = parameters;
            parameters["@type"] = json!("tdlibParameters");
            parameters["enable_storage_optimizer"] = json!(false);
            parameters["ignore_file_names"] = json!(false);
            json!({"@type":"setTdlibParameters","parameters":parameters,"@extra":"initialization"})
        }
        TdSchema::Current1869 => {
            let mut parameters = parameters;
            parameters["@type"] = json!("setTdlibParameters");
            parameters["database_encryption_key"] = json!(config.encryption_key);
            parameters["@extra"] = json!("initialization");
            parameters
        }
    }
}
fn closing_request(state: &str, remove: bool) -> Result<Option<&'static str>, String> {
    match state {
        "authorizationStateClosed" | "authorizationStateClosing" | "authorizationStateLoggingOut" => Ok(None),
        "authorizationStateReady" if remove => Ok(Some("logOut")),
        "authorizationStateWaitTdlibParameters" | "authorizationStateWaitEncryptionKey" if remove => Err("Wait for Telegram session initialization before deleting the connection. The protected key was kept.".into()),
        "authorizationStateWaitPhoneNumber" | "authorizationStateWaitCode" | "authorizationStateWaitPassword" | "authorizationStateWaitOtherDeviceConfirmation" | "authorizationStateWaitRegistration" | "authorizationStateWaitEmailAddress" | "authorizationStateWaitEmailCode" | "authorizationStateWaitPremiumPurchase" => Ok(Some("close")),
        _ if !remove => Ok(Some("close")),
        _ => Err("This authorization state cannot be forgotten safely. The protected key was kept.".into()),
    }
}
async fn schema(account: &str) -> Result<TdSchema, String> {
    let shared = client(account)?;
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .lock()
            .map(|client| client.schema)
            .map_err(|_| "Telegram connection state is unavailable.".to_owned())
    })
    .await
    .map_err(|_| "Telegram schema lookup stopped unexpectedly.")?
}

fn account_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 32
        || !value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(
            "Use a local account label of 1–32 lowercase letters, digits or hyphens.".into(),
        );
    }
    Ok(())
}
fn numeric_id(value: &str, allow_zero: bool) -> Result<i64, String> {
    let parsed: i64 = value.parse().map_err(|_| "Invalid Telegram identifier.")?;
    if parsed.unsigned_abs() > 9_007_199_254_740_991 || (!allow_zero && parsed == 0) {
        return Err("Invalid Telegram identifier.".into());
    }
    Ok(parsed)
}
fn root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = crate::components::root(app)?;
    crate::sessions::secure_module_directory(&root, "telegram")
}
fn account_root(app: &AppHandle, account: &str) -> Result<PathBuf, String> {
    account_id(account)?;
    let root = root(app)?;
    let encoded = SecretBytes(crate::sessions::load_module_secret(
        &root,
        &format!("telegram-{account}"),
    )?);
    let configuration: Configuration =
        serde_json::from_slice(&encoded.0).map_err(|_| "Telegram configuration is invalid.")?;
    let generation = format!(
        "session-{:x}",
        Sha256::digest(configuration.encryption_key.as_bytes())
    );
    let account_root = crate::sessions::secure_module_directory(&root, account)?;
    crate::sessions::secure_module_directory(&account_root, &generation)
}
fn configured(app: &AppHandle) -> Result<Vec<String>, String> {
    let path = root(app)?.join("accounts.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path).map_err(|_| "Could not read Telegram account labels.")?;
    if bytes.len() > 4096 {
        return Err("Telegram account labels exceed their limit.".into());
    }
    let ids: Vec<String> =
        serde_json::from_slice(&bytes).map_err(|_| "Telegram account labels are invalid.")?;
    if ids.len() > 5 {
        return Err("At most five Telegram accounts can be configured.".into());
    }
    for id in &ids {
        account_id(id)?;
    }
    Ok(ids)
}
fn save_index(app: &AppHandle, ids: &[String]) -> Result<(), String> {
    let directory = root(app)?;
    let pending = directory.join("accounts.pending");
    fs::write(
        &pending,
        serde_json::to_vec(ids).map_err(|_| "Could not save account labels.")?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(pending, directory.join("accounts.json")).map_err(|e| e.to_string())
}
fn registry() -> &'static Mutex<HashMap<String, SharedClient>> {
    CLIENTS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn client(account: &str) -> Result<SharedClient, String> {
    account_id(account)?;
    registry()
        .lock()
        .map_err(|_| "Telegram connection state is unavailable.")?
        .get(account)
        .cloned()
        .ok_or_else(|| "Connect the selected Telegram account first.".into())
}
async fn request(account: &str, query: Value) -> Result<Value, String> {
    let shared = client(account)?;
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .lock()
            .map_err(|_| "Telegram connection state is unavailable.")?
            .request(query)
    })
    .await
    .map_err(|_| "Telegram request stopped unexpectedly.")?
}
fn checked(value: Value) -> Result<Value, String> {
    if value["@type"] == "error" {
        let code = value["code"].as_i64().unwrap_or_default();
        return Err(match code {
            401 => "Telegram authorization is required. Reconnect this account.".into(),
            429 => "Telegram is limiting requests. Wait before trying again.".into(),
            400 => "Telegram rejected this request. Check the authorization step, access and supported TDLib version.".into(),
            404 => "The Telegram chat, message or file is unavailable to this account.".into(),
            _ => format!("Telegram request failed with code {code}. Check the connection and account access."),
        });
    }
    Ok(value)
}
fn configuration_valid(id: i32, hash: &str) -> Result<(), String> {
    if id <= 0 || hash.len() != 32 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Enter the legitimate api_id and 32-character api_hash issued for your application at my.telegram.org.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn telegram_status(app: AppHandle) -> Result<Status, String> {
    let accounts = configured(&app)?
        .into_iter()
        .map(|id| {
            let state = client(&id)
                .ok()
                .and_then(|client| {
                    client.try_lock().ok().map(|native| {
                        (
                            native.authorization.clone(),
                            Some(native.schema.version().to_owned()),
                        )
                    })
                })
                .unwrap_or((Authorization::Disconnected, None));
            Account {
                id,
                authorization: state.0,
                version: state.1,
            }
        })
        .collect();
    Ok(Status {
        installed: dll_path(&app).is_ok(),
        accounts,
    })
}

#[tauri::command]
pub async fn telegram_connect(
    app: AppHandle,
    account: String,
    api_id: Option<i32>,
    api_hash: Option<String>,
) -> Result<Account, String> {
    account_id(&account)?;
    let dll = dll_path(&app)?;
    if !crate::components::root(&app)?.join("telegram.enabled").is_file(){return Err("Enable Telegram in Settings → Components before connecting an account.".into());}
    let mut ids = configured(&app)?;
    if !ids.contains(&account) && ids.len() >= 5 {
        return Err("At most five Telegram accounts can be configured.".into());
    }
    let secret_name = format!("telegram-{account}");
    let config: Configuration = if let (Some(id), Some(hash)) = (api_id, api_hash) {
        configuration_valid(id, &hash)?;
        if ids.contains(&account) {
            return Err("This account already has credentials. Reconnect it without replacing its configuration.".into());
        }
        let config = Configuration {
            api_id: id,
            api_hash: hash,
            encryption_key: random_key()?,
        };
        let encoded = SecretBytes(
            serde_json::to_vec(&config).map_err(|_| "Could not protect Telegram configuration.")?,
        );
        crate::sessions::save_module_secret(&root(&app)?, &secret_name, &encoded.0)?;
        config
    } else {
        let encoded = SecretBytes(crate::sessions::load_module_secret(
            &root(&app)?,
            &secret_name,
        )?);
        serde_json::from_slice(&encoded.0).map_err(|_| {
            "Telegram configuration is invalid. Remove the connection and configure it again."
        })?
    };
    configuration_valid(config.api_id, &config.api_hash)?;
    if !ids.contains(&account) {
        ids.push(account.clone());
        save_index(&app, &ids)?;
    }
    let directory = account_root(&app, &account)?;
    fs::create_dir_all(directory.join("database")).map_err(|e| e.to_string())?;
    fs::create_dir_all(directory.join("files")).map_err(|e| e.to_string())?;
    let selected = account.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut registry = registry()
            .lock()
            .map_err(|_| "Telegram connection state is unavailable.")?;
        if !registry.contains_key(&selected) {
            registry.insert(
                selected,
                Arc::new(Mutex::new(Native::open(&dll, &directory, config)?)),
            );
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "Could not start the Telegram component.")??;
    telegram_authorization(account).await
}

#[tauri::command]
pub async fn telegram_authorization(account: String) -> Result<Account, String> {
    let value = checked(request(&account, json!({"@type":"getAuthorizationState"})).await?)?;
    Ok(Account {
        version: Some(schema(&account).await?.version().to_owned()),
        id: account,
        authorization: authorization(&value),
    })
}

#[tauri::command]
pub async fn telegram_authorize(account: String, action: AuthAction) -> Result<Account, String> {
    let query = match action {
        AuthAction::Phone { value }
            if (5..=24).contains(&value.len())
                && value
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-')) =>
        {
            json!({"@type":"setAuthenticationPhoneNumber","phone_number":value,"settings":null})
        }
        AuthAction::Code { value }
            if (1..=256).contains(&value.len()) && !value.chars().any(char::is_control) =>
        {
            json!({"@type":"checkAuthenticationCode","code":value})
        }
        AuthAction::Password { value } if !value.is_empty() && value.len() <= 1024 => {
            json!({"@type":"checkAuthenticationPassword","password":value})
        }
        AuthAction::Qr => json!({"@type":"requestQrCodeAuthentication","other_user_ids":[]}),
        _ => return Err("Check the value for the current Telegram authorization step.".into()),
    };
    checked(request(&account, query).await?)?;
    telegram_authorization(account).await
}

#[tauri::command]
pub async fn telegram_disconnect(
    app: AppHandle,
    account: String,
    remove: bool,
) -> Result<(), String> {
    account_id(&account)?;
    if remove && client(&account).is_err() {
        return Err("Reconnect this account before logging out and deleting its session. No files were removed.".into());
    }
    if client(&account).is_ok() {
        let shared = client(&account)?;
        tauri::async_runtime::spawn_blocking(move || {
            shared
                .lock()
                .map_err(|_| "Telegram connection state is unavailable.")?
                .finish(remove)
        })
        .await
        .map_err(|_| "Could not close the Telegram session.")??;
        let removed = registry()
            .lock()
            .map_err(|_| "Telegram connection state is unavailable.")?
            .remove(&account);
        tauri::async_runtime::spawn_blocking(move || drop(removed))
            .await
            .map_err(|_| "Could not close the Telegram client.")?;
    }
    if remove {
        let mut ids = configured(&app)?;
        ids.retain(|id| id != &account);
        save_index(&app, &ids)?;
        // Library copies are independent of TDLib's account data, which logOut may delete.
        let path = root(&app)?
            .join("module-secrets")
            .join(format!("telegram-{account}.bin"));
        if path.is_file() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn chat(value: &Value) -> Result<Chat, String> {
    let id = value["id"]
        .as_i64()
        .ok_or("Telegram returned an invalid chat.")?;
    Ok(Chat {
        id: id.to_string(),
        title: value["title"]
            .as_str()
            .unwrap_or("Telegram")
            .chars()
            .take(240)
            .collect(),
        protected: value["has_protected_content"].as_bool().unwrap_or(true),
    })
}

#[tauri::command]
pub async fn telegram_chats(account: String, limit: u32) -> Result<Vec<Chat>, String> {
    if !(1..=100).contains(&limit) {
        return Err("Load between 1 and 100 chats at once.".into());
    }
    let loaded = request(
        &account,
        json!({"@type":"loadChats","chat_list":{"@type":"chatListMain"},"limit":limit}),
    )
    .await?;
    if loaded["@type"] == "error" && loaded["code"] != 404 {
        checked(loaded)?;
    }
    let response = checked(
        request(
            &account,
            json!({"@type":"getChats","chat_list":{"@type":"chatListMain"},"limit":limit}),
        )
        .await?,
    )?;
    let mut result = Vec::new();
    for id in response["chat_ids"]
        .as_array()
        .ok_or("Telegram returned an invalid chat list.")?
        .iter()
        .take(limit as usize)
    {
        if let Some(id) = id.as_i64() {
            result.push(chat(&checked(
                request(&account, json!({"@type":"getChat","chat_id":id})).await?,
            )?)?);
        }
    }
    Ok(result)
}

pub fn attachments(
    account: &str,
    message: &Value,
    chat_protected: bool,
) -> Vec<TelegramAttachment> {
    let Some(chat_id) = message["chat_id"].as_i64() else {
        return vec![];
    };
    let Some(message_id) = message["id"].as_i64() else {
        return vec![];
    };
    let content = &message["content"];
    let content_type = content["@type"].as_str().unwrap_or_default();
    let descriptor = match content_type {
        "messagePhoto" => content["photo"]["sizes"]
            .as_array()
            .and_then(|sizes| {
                sizes.iter().max_by_key(|size| {
                    size["width"]
                        .as_u64()
                        .unwrap_or(0)
                        .saturating_mul(size["height"].as_u64().unwrap_or(0))
                })
            })
            .map(|size| (&size["photo"], "photo", "photo.jpg".to_owned())),
        "messageVideo" => Some((
            &content["video"]["video"],
            "video",
            content["video"]["file_name"]
                .as_str()
                .unwrap_or("video.mp4")
                .to_owned(),
        )),
        "messageAnimation" => Some((
            &content["animation"]["animation"],
            "video",
            content["animation"]["file_name"]
                .as_str()
                .unwrap_or("animation.mp4")
                .to_owned(),
        )),
        "messageAudio" => Some((
            &content["audio"]["audio"],
            "audio",
            content["audio"]["file_name"]
                .as_str()
                .unwrap_or("audio.m4a")
                .to_owned(),
        )),
        "messageVoiceNote" => Some((&content["voice_note"]["voice"], "audio", "voice.ogg".into())),
        "messageVideoNote" => Some((
            &content["video_note"]["video"],
            "video",
            "video-note.mp4".into(),
        )),
        "messageDocument" => Some((
            &content["document"]["document"],
            "document",
            content["document"]["file_name"]
                .as_str()
                .unwrap_or("document.bin")
                .to_owned(),
        )),
        _ => None,
    };
    let Some((file, kind, filename)) = descriptor else {
        return vec![];
    };
    let Some(file_id) = file["id"]
        .as_i64()
        .and_then(|id| i32::try_from(id).ok())
        .filter(|id| *id > 0)
    else {
        return vec![];
    };
    let ephemeral = !message["self_destruct_type"].is_null()
        || !message["ephemeral_content"].is_null()
        || !message["receiver_id"].is_null()
        || message["ephemeral_message_id"]
            .as_i64()
            .is_some_and(|value| value != 0)
        || message["ttl"].as_i64().is_some_and(|value| value > 0)
        || message["ttl_expires_in"]
            .as_f64()
            .is_some_and(|value| value > 0.0)
        || message["self_destruct_in"]
            .as_f64()
            .is_some_and(|v| v > 0.0)
        || content["is_secret"].as_bool() == Some(true);
    let restricted = !message["restriction_info"].is_null()
        || message["restriction_reason"]
            .as_str()
            .is_some_and(|value| !value.is_empty());
    let can_save = message["can_be_saved"].as_bool() == Some(true)
        && !chat_protected
        && !ephemeral
        && !restricted;
    let unique = file["remote"]["unique_id"]
        .as_str()
        .filter(|value| value.len() <= 160)
        .map(str::to_owned)
        .unwrap_or_else(|| file_id.to_string());
    let title = content["caption"]["text"]
        .as_str()
        .filter(|text| !text.is_empty())
        .unwrap_or(&filename)
        .chars()
        .take(240)
        .collect();
    vec![TelegramAttachment {
        account_id: account.into(),
        chat_id: chat_id.to_string(),
        message_id: message_id.to_string(),
        file_id,
        id: format!("{account}:{chat_id}:{message_id}:{unique}"),
        title,
        filename: crate::options::safe_segment(&filename),
        kind: kind.into(),
        size: file["size"].as_u64().filter(|size| *size > 0),
        date: message["date"].as_i64().unwrap_or(0),
        can_save,
        reason: (!can_save).then(|| {
            if ephemeral {
                "Ephemeral media cannot be saved."
            } else {
                "Telegram does not allow saving this message."
            }
            .into()
        }),
    }]
}

#[tauri::command]
pub async fn telegram_media(query: MediaQuery) -> Result<MediaPage, String> {
    account_id(&query.account_id)?;
    let chat_id = numeric_id(&query.chat_id, false)?;
    let from = numeric_id(&query.from_message_id, true)?;
    if !(1..=100).contains(&query.limit)
        || query.query.len() > 240
        || !matches!(
            query.kind.as_str(),
            "all" | "photo" | "video" | "audio" | "document"
        )
        || query
            .start_date
            .zip(query.end_date)
            .is_some_and(|(start, end)| start > end)
        || query
            .min_size
            .zip(query.max_size)
            .is_some_and(|(min, max)| min > max)
    {
        return Err("Choose a valid media filter and a page of 1–100 messages.".into());
    }
    let chat = chat(&checked(
        request(
            &query.account_id,
            json!({"@type":"getChat","chat_id":chat_id}),
        )
        .await?,
    )?)?;
    let history = query.kind == "all" && query.query.is_empty();
    let legacy = schema(&query.account_id).await? == TdSchema::Legacy180;
    let history_limit = if from == 0 {
        query.limit
    } else {
        (query.limit + 1).min(100)
    };
    let mut value = if history {
        json!({"@type":"getChatHistory","chat_id":chat_id,"from_message_id":from,"offset":0,"limit":history_limit,"only_local":false})
    } else {
        let filter = match query.kind.as_str() {
            "photo" => Some("searchMessagesFilterPhoto"),
            "video" => Some("searchMessagesFilterVideo"),
            "audio" => Some("searchMessagesFilterAudio"),
            "document" => Some("searchMessagesFilterDocument"),
            _ => None,
        };
        json!({"@type":"searchChatMessages","chat_id":chat_id,"query":query.query,"sender_id":null,"topic_id":null,"from_message_id":from,"offset":0,"limit":if legacy { history_limit } else { query.limit },"filter":filter.map(|kind| json!({"@type":kind}))})
    };
    if !history && legacy {
        value
            .as_object_mut()
            .ok_or("Invalid Telegram search.")?
            .remove("topic_id");
        value["message_thread_id"] = json!(0);
    }
    let response = checked(request(&query.account_id, value).await?)?;
    let messages = response["messages"]
        .as_array()
        .ok_or("Telegram returned an invalid media page.")?;
    let inspected = messages.len().min(100);
    let next = response["next_from_message_id"]
        .as_i64()
        .or_else(|| messages.last()?["id"].as_i64())
        .unwrap_or(0);
    let items = messages
        .iter()
        .filter(|message| !(history || legacy) || message["id"].as_i64() != Some(from))
        .take(query.limit as usize)
        .flat_map(|message| attachments(&query.account_id, message, chat.protected))
        .filter(|item| {
            query.start_date.is_none_or(|date| item.date >= date)
                && query.end_date.is_none_or(|date| item.date <= date)
                && query
                    .min_size
                    .is_none_or(|min| item.size.is_some_and(|size| size >= min))
                && query
                    .max_size
                    .is_none_or(|max| item.size.is_some_and(|size| size <= max))
        })
        .collect();
    Ok(MediaPage {
        items,
        next_message_id: if inspected == 0 || next == from {
            "0".into()
        } else {
            next.to_string()
        },
        inspected,
    })
}

#[tauri::command]
pub async fn telegram_message_link(account: String, url: String) -> Result<MediaPage, String> {
    let normalized = crate::model::validate_url(&url)?;
    let parsed = url::Url::parse(&normalized).map_err(|_| "Invalid Telegram message link.")?;
    if !matches!(parsed.host_str(), Some("t.me" | "telegram.me")) || parsed.scheme() != "https" {
        return Err(
            "Open an HTTPS t.me or telegram.me message link accessible to this account.".into(),
        );
    }
    let value = checked(
        request(
            &account,
            json!({"@type":"getMessageLinkInfo","url":normalized}),
        )
        .await?,
    )?;
    if value["message"].is_null() {
        return Err(
            "This message is not accessible to the selected account. No channel was joined.".into(),
        );
    }
    let chat_id = value["message"]["chat_id"]
        .as_i64()
        .ok_or("Telegram returned no message chat.")?;
    let chat = chat(&checked(
        request(&account, json!({"@type":"getChat","chat_id":chat_id})).await?,
    )?)?;
    Ok(MediaPage {
        items: attachments(&account, &value["message"], chat.protected),
        next_message_id: "0".into(),
        inspected: 1,
    })
}

pub async fn validate_attachment(
    attachment: &TelegramAttachment,
) -> Result<TelegramAttachment, String> {
    let chat_id = numeric_id(&attachment.chat_id, false)?;
    let message_id = numeric_id(&attachment.message_id, false)?;
    let chat = chat(&checked(
        request(
            &attachment.account_id,
            json!({"@type":"getChat","chat_id":chat_id}),
        )
        .await?,
    )?)?;
    let message = checked(
        request(
            &attachment.account_id,
            json!({"@type":"getMessage","chat_id":chat_id,"message_id":message_id}),
        )
        .await?,
    )?;
    let actual = attachments(&attachment.account_id, &message, chat.protected)
        .into_iter()
        .find(|item| item.file_id == attachment.file_id && item.id == attachment.id)
        .ok_or("The selected Telegram attachment has changed or is no longer available.")?;
    if !actual.can_save {
        return Err(actual
            .reason
            .unwrap_or_else(|| "Telegram does not permit saving this attachment.".into()));
    }
    Ok(actual)
}

pub async fn download(
    app: &AppHandle,
    attachment: &TelegramAttachment,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    request_id: u64,
    emit: impl Fn(Progress) + Clone + Send + Sync + 'static,
) -> Result<PathBuf, String> {
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let attachment = validate_attachment(attachment).await?;
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let started = Instant::now();
    let mut file = checked(request(&attachment.account_id, json!({"@type":"downloadFile","file_id":attachment.file_id,"priority":16,"offset":0,"limit":0,"synchronous":false})).await?)?;
    loop {
        if *cancel.borrow() || started.elapsed() > Duration::from_secs(24 * 60 * 60) {
            let _ = request(&attachment.account_id, json!({"@type":"cancelDownloadFile","file_id":attachment.file_id,"only_if_pending":false})).await;
            return Err(if *cancel.borrow() {
                crate::engine::CANCELLED
            } else {
                "The Telegram download reached its 24-hour limit. Partial data is kept."
            }
            .into());
        }
        let downloaded = file["local"]["downloaded_size"].as_u64().unwrap_or(0);
        let total = file["size"].as_u64().filter(|size| *size > 0);
        emit(Progress {
            request_id,
            phase: "downloading".into(),
            percent: total.map(|total| (downloaded as f64 / total as f64 * 100.0).min(100.0)),
            speed: None,
            downloaded: Some(downloaded as f64),
            total: total.map(|total| total as f64),
        });
        if file["local"]["is_downloading_completed"].as_bool() == Some(true) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        file = checked(
            request(
                &attachment.account_id,
                json!({"@type":"getFile","file_id":attachment.file_id}),
            )
            .await?,
        )?;
        if file["local"]["is_downloading_active"].as_bool() == Some(false)
            && file["local"]["is_downloading_completed"].as_bool() != Some(true)
        {
            return Err("Telegram stopped this file download. Check access and retry the individual attachment.".into());
        }
    }
    validate_attachment(&attachment).await?;
    let input = crate::library::authorized_path(Path::new(
        file["local"]["path"]
            .as_str()
            .ok_or("Telegram returned no local file.")?,
    ))?;
    let cache =
        crate::library::authorized_path(&account_root(app, &attachment.account_id)?.join("files"))?;
    if !input.starts_with(cache) {
        return Err("Telegram returned a file outside this account's media cache.".into());
    }
    if file["size"].as_u64().is_some_and(|size| {
        size > 0
            && fs::metadata(&input)
                .map(|metadata| metadata.len() != size)
                .unwrap_or(true)
    }) {
        return Err("The downloaded Telegram file does not match its reported size.".into());
    }
    let staging = crate::library::authorized_path(staging)?;
    let target = staging.join(format!("{}-{}", attachment.message_id, attachment.filename));
    let partial = staging.join(format!(".telegram-copy-{request_id}.part"));
    let target_copy = target.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut source = fs::File::open(input).map_err(|e| e.to_string())?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|e| e.to_string())?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if *cancel.borrow() {
                return Err(crate::engine::CANCELLED.into());
            }
            let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        }
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        if target_copy.exists() {
            return Err(
                "A Telegram output already exists in this job. No file was overwritten.".into(),
            );
        }
        fs::rename(partial, target_copy).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Could not copy the Telegram attachment.")??;
    Ok(target)
}

fn authorization(value: &Value) -> Authorization {
    match value["@type"].as_str().unwrap_or_default() {
        "authorizationStateWaitTdlibParameters" | "authorizationStateWaitEncryptionKey" => {
            Authorization::Initializing
        }
        "authorizationStateWaitPhoneNumber" => Authorization::Phone,
        "authorizationStateWaitCode" => Authorization::Code,
        "authorizationStateWaitPassword" => Authorization::Password {
            hint: value["password_hint"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(160)
                .collect(),
        },
        "authorizationStateWaitOtherDeviceConfirmation" => {
            let link = value["link"].as_str().unwrap_or_default();
            if link.starts_with("tg://login?token=") && link.len() <= 1024 {
                Authorization::Qr {
                    svg: qr_svg(link).unwrap_or_default(),
                }
            } else {
                Authorization::Unsupported {
                    state: "Invalid QR authorization link".into(),
                }
            }
        }
        "authorizationStateReady" => Authorization::Ready,
        "authorizationStateClosed" => Authorization::Closed,
        "authorizationStateClosing" | "authorizationStateLoggingOut" => Authorization::Closing,
        state => Authorization::Unsupported {
            state: state.chars().take(120).collect(),
        },
    }
}

fn qr_svg(link: &str) -> Result<String, String> {
    let code = qrcodegen::QrCode::encode_text(link, qrcodegen::QrCodeEcc::Medium)
        .map_err(|_| "Could not render this authorization QR code.")?;
    let size = code.size();
    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if code.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + 4, y + 4));
            }
        }
    }
    Ok(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"220\" height=\"220\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"Telegram authorization QR\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/><path fill=\"black\" d=\"{path}\"/></svg>",size+8,size+8))
}

#[cfg(windows)]
fn random_key() -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut bytes = [0u8; 32];
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
    {
        return Err("Windows could not generate the Telegram database encryption key.".into());
    }
    Ok(base64(&bytes))
}
#[cfg(not(windows))]
fn random_key() -> Result<String, String> {
    Err("Telegram session protection is available on Windows only.".into())
}
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut value = String::new();
    for part in bytes.chunks(3) {
        let block = (part[0] as u32) << 16
            | (part.get(1).copied().unwrap_or(0) as u32) << 8
            | part.get(2).copied().unwrap_or(0) as u32;
        for shift in [18, 12, 6, 0] {
            value.push(ALPHABET[((block >> shift) & 63) as usize] as char);
        }
        if part.len() < 3 {
            value.pop();
            if part.len() == 1 {
                value.pop();
                value.push('=');
            }
            value.push('=');
        }
    }
    value
}

fn dll_path(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("NABNAB_TDLIB_DLL") {
        let path = crate::library::authorized_path(Path::new(&path))?;
        if !path.is_file()
            || path
                .extension()
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("dll"))
        {
            return Err("NABNAB_TDLIB_DLL must name a regular tdjson.dll file.".into());
        }
        return Ok(path);
    }
    crate::component_archives::verify(&crate::components::root(app)?, "telegram")
}

#[cfg(windows)]
struct LoadedLibrary(*mut std::ffi::c_void);
#[cfg(windows)]
impl Drop for LoadedLibrary {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}
#[cfg(windows)]
struct Native {
    _library: LoadedLibrary,
    pointer: *mut std::ffi::c_void,
    send: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char),
    receive: unsafe extern "C" fn(*mut std::ffi::c_void, f64) -> *const std::ffi::c_char,
    destroy: unsafe extern "C" fn(*mut std::ffi::c_void),
    schema: TdSchema,
    encryption_key: String,
    sequence: u64,
    authorization: Authorization,
}
// TDLib send permits any thread; the mutex serializes receive and destruction for this instance.
#[cfg(windows)]
unsafe impl Send for Native {}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(
        path: *const u16,
        file: *mut std::ffi::c_void,
        flags: u32,
    ) -> *mut std::ffi::c_void;
    fn GetProcAddress(
        module: *mut std::ffi::c_void,
        name: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void;
    fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
impl Native {
    fn open(dll: &Path, directory: &Path, config: Configuration) -> Result<Self, String> {
        let mut native=Self::unconfigured(dll)?;
        native.encryption_key=config.encryption_key.clone();
        native.send(&initialization(native.schema,directory,&config))?;
        Ok(native)
    }
    fn unconfigured(dll: &Path) -> Result<Self, String> {
        use std::{ffi::CString, os::windows::ffi::OsStrExt};
        let path: Vec<u16> = dll.as_os_str().encode_wide().chain(Some(0)).collect();
        let library = unsafe { LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), 0x100 | 0x800) };
        if library.is_null() {
            return Err("TDLib could not load. Check the 64-bit DLL and its required libraries. No session was started.".into());
        }
        let loaded_library = LoadedLibrary(library);
        let symbol = |name: &str| -> Result<*mut std::ffi::c_void, String> {
            let name = CString::new(name).map_err(|_| "Invalid TDLib symbol.")?;
            let symbol = unsafe { GetProcAddress(library, name.as_ptr()) };
            if symbol.is_null() {
                Err("The TDLib JSON DLL does not expose the supported 1.8 interface.".into())
            } else {
                Ok(symbol)
            }
        };
        let loaded = (|| unsafe {
            let create: unsafe extern "C" fn() -> *mut std::ffi::c_void =
                std::mem::transmute(symbol("td_json_client_create")?);
            let send = std::mem::transmute::<
                *mut std::ffi::c_void,
                unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char),
            >(symbol("td_json_client_send")?);
            let receive = std::mem::transmute::<
                *mut std::ffi::c_void,
                unsafe extern "C" fn(*mut std::ffi::c_void, f64) -> *const std::ffi::c_char,
            >(symbol("td_json_client_receive")?);
            let destroy = std::mem::transmute::<
                *mut std::ffi::c_void,
                unsafe extern "C" fn(*mut std::ffi::c_void),
            >(symbol("td_json_client_destroy")?);
            let execute: unsafe extern "C" fn(
                *mut std::ffi::c_void,
                *const std::ffi::c_char,
            ) -> *const std::ffi::c_char = std::mem::transmute(symbol("td_json_client_execute")?);
            for raw in [
                "{\"@type\":\"setLogVerbosityLevel\",\"new_verbosity_level\":0}",
                "{\"@type\":\"setLogStream\",\"log_stream\":{\"@type\":\"logStreamEmpty\"}}",
            ] {
                let value =
                    CString::new(raw).map_err(|_| "Invalid TDLib logging configuration.")?;
                execute(std::ptr::null_mut(), value.as_ptr());
            }
            let pointer = create();
            if pointer.is_null() {
                return Err("TDLib could not create a client.".into());
            }
            let mut native = Self {
                _library: loaded_library,
                pointer,
                send,
                receive,
                destroy,
                schema: TdSchema::Legacy180,
                encryption_key: String::new(),
                sequence: 0,
                authorization: Authorization::Initializing,
            };
            let version = checked(native.request(json!({"@type":"getOption","name":"version"}))?)?;
            let schema = TdSchema::parse(version["value"].as_str().ok_or("TDLib did not return its API version. Initialization was stopped before authorization.")?)?;
            native.schema = schema;
            Ok(native)
        })();
        loaded
    }
    fn send(&self, query: &Value) -> Result<(), String> {
        let value = std::ffi::CString::new(
            serde_json::to_vec(query).map_err(|_| "Could not prepare a Telegram request.")?,
        )
        .map_err(|_| "Invalid Telegram request.")?;
        unsafe {
            (self.send)(self.pointer, value.as_ptr());
        }
        Ok(())
    }
    fn request(&mut self, mut query: Value) -> Result<Value, String> {
        let authorization_request = query["@type"] == "getAuthorizationState";
        self.sequence = self.sequence.wrapping_add(1);
        let extra = self.sequence.to_string();
        query["@extra"] = json!(extra);
        self.send(&query)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let raw = unsafe { (self.receive)(self.pointer, 0.2) };
            if raw.is_null() {
                continue;
            }
            let bytes = unsafe { std::ffi::CStr::from_ptr(raw) }.to_bytes();
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("Telegram response exceeds its size limit.".into());
            }
            let value: Value =
                serde_json::from_slice(bytes).map_err(|_| "TDLib returned invalid JSON.")?;
            if value["@type"] == "updateAuthorizationState" {
                let state = &value["authorization_state"];
                self.authorization = authorization(state);
                if state["@type"] == "authorizationStateWaitEncryptionKey" {
                    self.send(&json!({"@type":"checkDatabaseEncryptionKey","encryption_key":self.encryption_key,"@extra":"initialization"}))?;
                }
            }
            if value["@extra"] == "initialization" && value["@type"] == "error" {
                checked(value)?;
                continue;
            }
            if value["@extra"].as_str() == Some(extra.as_str()) {
                if authorization_request {
                    self.authorization = authorization(&value);
                }
                return Ok(value);
            }
        }
        Err("Telegram took too long to respond. Check the connection and retry.".into())
    }
    fn wait_closed(&mut self) -> Result<(), String> {
        if matches!(self.authorization, Authorization::Closed) {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let raw = unsafe { (self.receive)(self.pointer, 0.2) };
            if raw.is_null() {
                continue;
            }
            let bytes = unsafe { std::ffi::CStr::from_ptr(raw) }.to_bytes();
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("Telegram response exceeds its size limit.".into());
            }
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|_| "TDLib returned invalid JSON while closing.")?;
            if value["@type"] == "updateAuthorizationState" {
                self.authorization = authorization(&value["authorization_state"]);
                if matches!(self.authorization, Authorization::Closed) {
                    return Ok(());
                }
            }
        }
        Err("Telegram has not confirmed session closure. Its protected key and connection were kept; check the network and retry.".into())
    }
    fn finish(&mut self, remove: bool) -> Result<(), String> {
        if matches!(self.authorization, Authorization::Closed) {
            return Ok(());
        }
        if matches!(self.authorization, Authorization::Closing) {
            return self.wait_closed();
        }
        let state = checked(self.request(json!({"@type":"getAuthorizationState"}))?)?;
        let kind = state["@type"].as_str().unwrap_or_default();
        if kind == "authorizationStateClosed" {
            self.authorization = Authorization::Closed;
            return Ok(());
        }
        if let Some(action) = closing_request(kind, remove)? {
            checked(self.request(json!({"@type":action}))?)?;
        }
        self.wait_closed()
    }
}
#[cfg(windows)]
impl Drop for Native {
    fn drop(&mut self) {
        if !matches!(self.authorization, Authorization::Closed) {
            let _ = self.send(&json!({"@type":"close"}));
        }
        unsafe {
            (self.destroy)(self.pointer);
        }
    }
}
#[cfg(not(windows))]
struct Native {
    authorization: Authorization,
    schema: TdSchema,
}
#[cfg(not(windows))]
impl Native {
    fn open(_: &Path, _: &Path, _: Configuration) -> Result<Self, String> {
        Err("Telegram TDLib is currently available on Windows only.".into())
    }
    fn request(&mut self, _: Value) -> Result<Value, String> {
        Err("Telegram TDLib is currently available on Windows only.".into())
    }
    fn finish(&mut self, _: bool) -> Result<(), String> {
        Err("Telegram TDLib is currently available on Windows only.".into())
    }
}

pub fn shutdown() {
    if let Some(clients) = CLIENTS.get() {
        if let Ok(mut clients) = clients.lock() {
            clients.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    #[ignore = "Requires NABNAB_TEST_TDLIB_DLL from the pinned package; checks version locally without account initialization."]
    fn real_tdlib_reports_pinned_schema_without_credentials() {
        let path=std::env::var_os("NABNAB_TEST_TDLIB_DLL").map(PathBuf::from).expect("Set pinned TDLib path");
        let native=Native::unconfigured(&path).unwrap();
        assert_eq!(native.schema,TdSchema::Current1869);
        assert!(native.encryption_key.is_empty());
    }
    fn message() -> Value {
        json!({"id":1234567890123i64,"chat_id":-1000000012345i64,"date":1700000000,"can_be_saved":true,"content":{"@type":"messageVideo","caption":{"text":"A permitted clip"},"video":{"file_name":"clip.mp4","video":{"id":41,"size":512,"remote":{"unique_id":"remote-stable"}}}}})
    }
    #[test]
    fn media_keeps_account_chat_message_and_attachment_identity() {
        let result = attachments("work", &message(), false);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].id,
            "work:-1000000012345:1234567890123:remote-stable"
        );
        assert_eq!(result[0].chat_id, "-1000000012345");
        assert_eq!(result[0].file_id, 41);
        assert_eq!(result[0].size, Some(512));
        assert!(result[0].can_save);
    }
    #[test]
    fn protected_ephemeral_and_unconfirmed_permissions_never_allow_saving() {
        assert!(!attachments("a", &message(), true)[0].can_save);
        let mut ephemeral = message();
        ephemeral["self_destruct_type"] = json!({"@type":"messageSelfDestructTypeImmediately"});
        assert!(!attachments("a", &ephemeral, false)[0].can_save);
        let mut denied = message();
        denied.as_object_mut().unwrap().remove("can_be_saved");
        assert!(!attachments("a", &denied, false)[0].can_save);
    }
    #[test]
    fn photos_select_the_largest_variant_and_do_not_download_thumbnails() {
        let value = json!({"id":1,"chat_id":2,"can_be_saved":true,"content":{"@type":"messagePhoto","photo":{"sizes":[{"width":100,"height":50,"photo":{"id":10,"size":100}},{"width":1024,"height":768,"photo":{"id":20,"size":900}}]}}});
        let result = attachments("a", &value, false);
        assert_eq!(result[0].file_id, 20);
        assert_eq!(result[0].kind, "photo");
        assert_eq!(result[0].filename, "photo.jpg");
    }
    #[test]
    fn authorization_is_typed_without_returning_credentials() {
        assert!(matches!(
            authorization(&json!({"@type":"authorizationStateWaitCode"})),
            Authorization::Code
        ));
        assert!(matches!(
            authorization(&json!({"@type":"authorizationStateReady"})),
            Authorization::Ready
        ));
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert!(configuration_valid(123, &"a".repeat(32)).is_ok());
        assert!(configuration_valid(0, &"a".repeat(32)).is_err());
    }
    #[test]
    fn qr_render_is_local_and_never_embeds_the_authentication_token() {
        let svg = qr_svg("tg://login?token=private-fixture").unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(!svg.contains("private-fixture"));
        assert!(!svg.contains("<script"));
        assert!(!svg.contains("href="));
    }
    #[test]
    fn known_versions_use_their_actual_parameter_shapes_and_reject_unknown_versions() {
        let config = Configuration {
            api_id: 123,
            api_hash: "a".repeat(32),
            encryption_key: "fixture-key".into(),
        };
        let legacy = initialization(
            TdSchema::parse("1.8.0").unwrap(),
            Path::new("C:/fixture"),
            &config,
        );
        assert_eq!(legacy["parameters"]["@type"], "tdlibParameters");
        assert_eq!(legacy["parameters"]["api_id"], 123);
        assert!(legacy["database_encryption_key"].is_null());
        let current = initialization(
            TdSchema::parse("1.8.69").unwrap(),
            Path::new("C:/fixture"),
            &config,
        );
        assert_eq!(current["api_id"], 123);
        assert_eq!(current["database_encryption_key"], "fixture-key");
        assert!(current["parameters"].is_null());
        assert!(TdSchema::parse("1.8.68").unwrap_err().contains("1.8.68"));
        assert!(TdSchema::parse("2.0.0").is_err());
        assert!(!TdSchema::parse("private-token")
            .unwrap_err()
            .contains("private-token"));
    }
    #[test]
    fn logout_and_local_forget_keep_initialization_and_closed_states_distinct() {
        assert_eq!(
            closing_request("authorizationStateReady", true).unwrap(),
            Some("logOut")
        );
        assert_eq!(
            closing_request("authorizationStateWaitPhoneNumber", true).unwrap(),
            Some("close")
        );
        assert_eq!(
            closing_request("authorizationStateWaitPassword", true).unwrap(),
            Some("close")
        );
        assert!(closing_request("authorizationStateWaitTdlibParameters", true).is_err());
        assert_eq!(
            closing_request("authorizationStateLoggingOut", true).unwrap(),
            None
        );
        assert!(matches!(
            authorization(&json!({"@type":"authorizationStateClosing"})),
            Authorization::Closing
        ));
        assert!(matches!(
            authorization(&json!({"@type":"authorizationStateClosed"})),
            Authorization::Closed
        ));
    }
    #[test]
    fn ephemeral_and_restricted_messages_are_rejected_across_both_supported_schemas() {
        for (field, value) in [
            ("ttl", json!(30)),
            ("ttl_expires_in", json!(10.5)),
            (
                "self_destruct_type",
                json!({"@type":"messageSelfDestructTypeTimer","self_destruct_time":30}),
            ),
            ("ephemeral_content", json!({"can_be_saved":false})),
            ("ephemeral_message_id", json!(7)),
            (
                "receiver_id",
                json!({"@type":"messageSenderUser","user_id":1}),
            ),
            ("restriction_reason", json!("Unavailable")),
            ("restriction_info", json!({})),
        ] {
            let mut value_message = message();
            value_message[field] = value;
            assert!(
                !attachments("a", &value_message, false)[0].can_save,
                "{field}"
            );
        }
    }
}
