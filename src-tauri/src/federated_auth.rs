use crate::AppState;
use reqwest::header::HeaderValue;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::Arc};
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FederatedAuthStatus {
    pub origin: String,
    pub configured: bool,
}

pub(crate) struct Bearer {
    origin: String,
    value: HeaderValue,
}
impl Bearer {
    pub(crate) fn header_for(&self, url: &url::Url) -> Result<HeaderValue, String> {
        if url.scheme() != "https"
            || url.origin().ascii_serialization() != self.origin
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("A Mastodon token cannot be sent to another origin. Configure that instance separately.".into());
        }
        Ok(self.value.clone())
    }
}
struct Secret(Vec<u8>);
impl Drop for Secret {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

fn validated_origin(input: &str) -> Result<String, String> {
    if input.len() > 2048 || input.chars().any(char::is_control) {
        return Err("Enter the public HTTPS origin of your Mastodon instance.".into());
    }
    let parsed = url::Url::parse(&crate::model::validate_url(input)?)
        .map_err(|_| "Enter a complete Mastodon instance origin.")?;
    if parsed.scheme() != "https"
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "Use only the HTTPS instance origin, without a post path, query or fragment.".into(),
        );
    }
    Ok(parsed.origin().ascii_serialization())
}
fn secret_name(origin: &str) -> String {
    format!("mastodon-{:x}", Sha256::digest(origin.as_bytes()))
}
fn token_valid(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 4096
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte))
}
fn header(origin: &str, token: &str) -> Result<Bearer, String> {
    if !token_valid(token) {
        return Err(
            "Paste the already-authorized OAuth bearer token without spaces or control characters."
                .into(),
        );
    }
    let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "The OAuth token cannot be used as an HTTP bearer token.")?;
    value.set_sensitive(true);
    Ok(Bearer {
        origin: origin.into(),
        value,
    })
}
fn configured(root: &Path, origin: &str) -> Result<bool, String> {
    let path = root
        .join("module-secrets")
        .join(format!("{}.bin", secret_name(origin)));
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err("The protected instance token is not a regular file.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Could not inspect the protected instance token.".into()),
    }
}
pub(crate) fn load(root: &Path, instance_origin: &str) -> Result<Option<Bearer>, String> {
    let origin = validated_origin(instance_origin)?;
    if !configured(root, &origin)? {
        return Ok(None);
    }
    let secret = Secret(crate::sessions::load_module_secret(
        root,
        &secret_name(&origin),
    )?);
    let token = std::str::from_utf8(&secret.0)
        .map_err(|_| "The protected Mastodon token is invalid. Remove it and add a valid token.")?;
    header(&origin, token).map(Some)
}

#[tauri::command]
pub fn federated_auth_status(
    state: State<Arc<AppState>>,
    origin: String,
) -> Result<FederatedAuthStatus, String> {
    let origin = validated_origin(&origin)?;
    let root = crate::config_root(&state)?;
    Ok(FederatedAuthStatus {
        configured: configured(&root, &origin)?,
        origin,
    })
}
#[tauri::command]
pub async fn set_federated_token(
    state: State<'_, Arc<AppState>>,
    origin: String,
    token: String,
    remove: bool,
    consent: bool,
) -> Result<FederatedAuthStatus, String> {
    let origin = validated_origin(&origin)?;
    if !consent {
        return Err(
            "Explicitly approve using or removing the token for this exact instance origin.".into(),
        );
    }
    let root = crate::config_root(&state)?;
    let name = secret_name(&origin);
    if remove {
        let path = root.join("module-secrets").join(format!("{name}.bin"));
        if configured(&root, &origin)? {
            let path = crate::library::authorized_path(&path)?;
            fs::remove_file(path).map_err(|_| "Could not remove the protected instance token.")?;
        }
    } else {
        header(&origin, &token)?;
        crate::providers::validate_remote(&format!("{origin}/")).await?;
        crate::sessions::save_module_secret(&root, &name, token.as_bytes())?;
    }
    Ok(FederatedAuthStatus {
        configured: configured(&root, &origin)?,
        origin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_scopes_are_canonical_https_origins_not_embedded_names() {
        assert_eq!(
            validated_origin("https://Social.Example.org:443/").unwrap(),
            "https://social.example.org"
        );
        for value in [
            "http://social.example.org/",
            "https://social.example.org/@user/123",
            "https://social.example.org/?token=private",
            "https://user:private@social.example.org/",
            "https://127.0.0.1/",
        ] {
            assert!(validated_origin(value).is_err(), "{value}");
        }
        assert_ne!(
            secret_name("https://social.example.org"),
            secret_name("https://social.example.org:8443")
        );
    }
    #[test]
    fn bearer_headers_are_sensitive_and_never_cross_instance_or_scheme() {
        let bearer = header("https://social.example.org", "authorized-token_fixture").unwrap();
        let value = bearer
            .header_for(&url::Url::parse("https://social.example.org/api/v1/statuses/123").unwrap())
            .unwrap();
        assert!(value.is_sensitive());
        assert_eq!(value.to_str().unwrap(), "Bearer authorized-token_fixture");
        for value in [
            "https://other.example.org/api/v1/statuses/123",
            "http://social.example.org/api/v1/statuses/123",
            "https://social.example.org:8443/api/v1/statuses/123",
        ] {
            assert!(bearer.header_for(&url::Url::parse(value).unwrap()).is_err());
        }
    }
    #[test]
    fn invalid_token_errors_do_not_repeat_credentials() {
        for token in ["private token", "private\r\nAuthorization: injected", ""] {
            let error = header("https://social.example.org", token).err().unwrap();
            assert!(!error.contains("private"));
        }
        let status = FederatedAuthStatus {
            origin: "https://social.example.org".into(),
            configured: true,
        };
        let encoded = serde_json::to_string(&status).unwrap();
        assert_eq!(
            encoded,
            "{\"origin\":\"https://social.example.org\",\"configured\":true}"
        );
    }
}
