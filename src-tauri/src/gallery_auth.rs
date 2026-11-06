use crate::AppState;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::State;

const SECRET: &str = "gallery-pixiv-refresh";

fn validate_site(site: &str) -> Result<(), String> {
    if site == "pixiv" {
        Ok(())
    } else {
        Err("Only Pixiv refresh tokens are supported here.".into())
    }
}
fn validate_token(token: &[u8]) -> Result<(), String> {
    if token.is_empty() || token.len() > 4096 || !token.iter().all(|b| b.is_ascii_graphic()) {
        return Err("Enter a Pixiv refresh token without spaces, up to 4096 characters.".into());
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub site: &'static str,
    pub configured: bool,
}

fn status(root: &Path) -> Result<AuthStatus, String> {
    let path = root.join("module-secrets").join(format!("{SECRET}.bin"));
    if !path.exists() {
        return Ok(AuthStatus {
            site: "pixiv",
            configured: false,
        });
    }
    let mut token = crate::sessions::load_module_secret(root, SECRET)?;
    let checked = validate_token(&token);
    token.fill(0);
    checked?;
    Ok(AuthStatus {
        site: "pixiv",
        configured: true,
    })
}

#[tauri::command]
pub fn gallery_auth_status(state: State<Arc<AppState>>) -> Result<AuthStatus, String> {
    status(&crate::config_root(&state)?)
}

#[tauri::command]
pub fn set_gallery_token(
    state: State<Arc<AppState>>,
    site: String,
    token: String,
    remove: bool,
) -> Result<AuthStatus, String> {
    validate_site(&site)?;
    let root = crate::config_root(&state)?;
    if remove {
        let directory = crate::sessions::secure_module_directory(&root, "module-secrets")?;
        match fs::remove_file(directory.join(format!("{SECRET}.bin"))) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not remove the protected Pixiv token.".into()),
        }
    } else {
        let mut bytes = token.into_bytes();
        let result = validate_token(&bytes)
            .and_then(|_| crate::sessions::save_module_secret(&root, SECRET, &bytes));
        bytes.fill(0);
        result?;
    }
    status(&root)
}

pub struct ConfigFile {
    path: Option<PathBuf>,
    token: Vec<u8>,
}
impl Drop for ConfigFile {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
        self.token.fill(0);
    }
}
impl ConfigFile {
    pub fn append_args(&self, args: &mut Vec<String>) {
        args.extend([
            "-o".into(),
            "extractor.instagram.videos=merged".into(),
            "-o".into(),
            "cache.file=null".into(),
        ]);
        if let Some(path) = &self.path {
            args.extend(["-c".into(), path.to_string_lossy().into_owned()]);
        }
    }
    pub fn redact(&self, error: String) -> String {
        let secret = std::str::from_utf8(&self.token).unwrap_or("");
        if secret.is_empty() {
            error
        } else {
            error.replace(secret, "[redacted]")
        }
    }
}

pub fn materialize(root: &Path, input: &str) -> Result<ConfigFile, String> {
    let url = url::Url::parse(input).map_err(|_| "Invalid gallery URL.")?;
    let host = url.host_str().unwrap_or("");
    if host != "pixiv.net" && !host.ends_with(".pixiv.net") {
        return Ok(ConfigFile {
            path: None,
            token: vec![],
        });
    }
    // Public Sketch uses another extractor and does not require an artwork API token.
    if host == "sketch.pixiv.net" {
        return Ok(ConfigFile {
            path: None,
            token: vec![],
        });
    }
    if !status(root)?.configured {
        return Err("Pixiv artwork requires a Pixiv refresh token in Accounts. Browser cookies alone are not sufficient.".into());
    }
    let token = crate::sessions::load_module_secret(root, SECRET)?;
    validate_token(&token)?;
    let directory = crate::sessions::secure_module_directory(root, "session-temp")?;
    // Cookie-prefixed .txt files are recovered by sessions::cleanup after an interrupted run.
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = directory.join(format!(
        "cookie-gallery-{}-{}-{}.txt",
        std::process::id(),
        crate::queue::now(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let temporary = ConfigFile {
        path: Some(path.clone()),
        token,
    };
    let config = serde_json::json!({"extractor":{"pixiv":{"refresh-token":std::str::from_utf8(&temporary.token).map_err(|_|"Invalid Pixiv token.")?}}});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "Could not create the private gallery configuration.")?;
    serde_json::to_writer(&mut file, &config)
        .map_err(|_| "Could not prepare the private gallery configuration.")?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not prepare the private gallery configuration.")?;
    Ok(temporary)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_other_sites_and_config_injection() {
        assert!(validate_site("../pixiv").is_err());
        for token in ["", "token\n\"extractor\":{}", " token", "tok\0en"] {
            assert!(validate_token(token.as_bytes()).is_err());
        }
        assert!(validate_token(b"fictional_token-123").is_ok());
    }
    #[test]
    fn args_contain_path_and_known_option_without_token() {
        let path = std::env::temp_dir().join(format!(
            "nabnab-gallery-auth-test-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let config = ConfigFile {
            path: Some(path.clone()),
            token: b"fictional-refresh-secret".to_vec(),
        };
        let mut args = vec![];
        config.append_args(&mut args);
        assert_eq!(
            args,
            vec![
                "-o",
                "extractor.instagram.videos=merged",
                "-o",
                "cache.file=null",
                "-c",
                path.to_str().unwrap()
            ]
        );
        assert!(!args.join(" ").contains("fictional-refresh-secret"));
        assert_eq!(
            config.redact("failure fictional-refresh-secret".into()),
            "failure [redacted]"
        );
        drop(config);
        assert!(!path.exists());
    }
}
