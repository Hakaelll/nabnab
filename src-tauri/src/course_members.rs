//! Independent student APIs. Streams are not treated as download permission.
use super::{Course, CourseAsset, CourseLesson, CourseSection};
use crate::{queue, sessions, AppState};
use serde_json::Value;
use std::{fs, sync::Arc, time::Duration};
use tauri::State;

fn provider_name(provider: &str) -> Result<&str, String> {
    if matches!(provider, "hotmart" | "kiwify") {
        Ok(provider)
    } else {
        Err("Choose Hotmart or Kiwify for a student token connection.".into())
    }
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
fn field_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|v| v.to_string()))
        .filter(|id| identifier(id))
}

#[tauri::command]
pub fn save_course_token(
    state: State<Arc<AppState>>,
    provider: String,
    token: String,
    remove: bool,
) -> Result<(), String> {
    provider_name(&provider)?;
    let root = crate::config_root(&state)?;
    let name = format!("course-{provider}");
    if remove {
        let path = root.join("module-secrets").join(format!("{name}.bin"));
        if path.exists() {
            crate::library::authorized_path(&path)?;
            fs::remove_file(path).map_err(|_| "Could not remove the course connection.")?;
        }
        return Ok(());
    }
    let token = token.trim();
    if token.is_empty()
        || token.len() > 16384
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))
    {
        return Err("Enter a valid student access token without spaces.".into());
    }
    sessions::save_module_secret(&root, &name, token.as_bytes())
}

async fn get_json(
    provider: &str,
    token: &str,
    url: &str,
    product_id: &str,
    slug: &str,
) -> Result<Value, String> {
    let url = url::Url::parse(&crate::model::validate_url(url)?)
        .map_err(|_| "Invalid course API URL.")?;
    let host = url.host_str().ok_or("Missing course API host.")?;
    let approved = match provider {
        "hotmart" => [
            "api-club-course-consumption-gateway-ga.cb.hotmart.com",
            "api-club-hot-club-api.cb.hotmart.com",
        ]
        .contains(&host),
        "kiwify" => ["admin-api.kiwify.com", "admin-api.kiwify.com.br"].contains(&host),
        _ => false,
    };
    if !approved || url.scheme() != "https" || url.port_or_known_default() != Some(443) {
        return Err("Student access tokens cannot be forwarded outside the selected provider's API origins.".into());
    }
    let addresses: Vec<_> =
        tokio::time::timeout(Duration::from_secs(8), tokio::net::lookup_host((host, 443)))
            .await
            .map_err(|_| "Course API hostname lookup timed out.")?
            .map_err(|_| "Could not resolve the course API.")?
            .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !crate::providers::is_public_ip(address.ip()))
    {
        return Err("The course API resolved to a reserved network destination.".into());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Could not prepare course API request.")?;
    let mut request = client
        .get(url.clone())
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if provider == "hotmart" {
        request = request
            .header("slug", slug)
            .header("x-product-id", product_id)
            .header("Origin", "https://hotmart.com");
    } else {
        request = request.header("Origin", "https://admin.kiwify.com.br");
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| "The authorized course API request failed.")?;
    if !response.status().is_success() {
        return Err(format!("{} returned HTTP {}. Check the student session, course access and download permissions.",provider,response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The course response was interrupted.")?
    {
        if bytes.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 {
            return Err("The course response exceeds 8 MiB.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "The course API returned unreadable lesson data.".into())
}

fn ordered(value: &Value) -> Vec<(String, &Value)> {
    let mut entries: Vec<_> = match value {
        Value::Array(values) => values
            .iter()
            .filter_map(|value| Some((field_id(&value["id"])?, value)))
            .collect(),
        Value::Object(values) => values
            .iter()
            .filter(|(id, _)| identifier(id))
            .map(|(id, value)| (id.clone(), value))
            .collect(),
        _ => Vec::new(),
    };
    entries.sort_by_key(|(_, value)| value["order"].as_i64().unwrap_or(0));
    entries
}

fn navigation(provider: &str, value: &Value, limit: usize) -> Result<Vec<CourseSection>, String> {
    let modules = value
        .get("modules")
        .or_else(|| value.get("content"))
        .unwrap_or(value);
    let mut sections = Vec::new();
    let mut count = 0;
    for (module_id, module) in ordered(modules).into_iter().take(1000) {
        let title = super::label(
            module["title"]
                .as_str()
                .or_else(|| module["name"].as_str())
                .unwrap_or("Section"),
            240,
        );
        let mut lessons = Vec::new();
        let lessons_value = if provider == "hotmart" {
            &module["pages"]
        } else {
            module
                .get("lessons")
                .or_else(|| module.get("classes"))
                .unwrap_or(&Value::Null)
        };
        let entries = if provider == "hotmart" {
            lessons_value
                .as_array()
                .map(|pages| {
                    pages
                        .iter()
                        .filter_map(|page| Some((field_id(&page["hash"])?, page)))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            ordered(lessons_value)
        };
        for (lesson_id, lesson) in entries {
            if count >= limit {
                break;
            }
            count += 1;
            let title = format!(
                "{count:03} {}",
                super::label(
                    lesson["title"]
                        .as_str()
                        .or_else(|| lesson["name"].as_str())
                        .unwrap_or("Lesson"),
                    500
                )
            );
            lessons.push(CourseLesson {
                id: format!("lesson-{lesson_id}"),
                title,
                learned: false,
                assets: Vec::new(),
                notes: Vec::new(),
            });
        }
        if !lessons.is_empty() {
            sections.push(CourseSection {
                id: format!("section-{module_id}"),
                title,
                lessons,
            });
        }
    }
    if sections.is_empty() {
        return Err(
            "The student API returned no accessible sections or stable lesson identifiers.".into(),
        );
    }
    Ok(sections)
}

fn explicit_download(value: &Value) -> Option<String> {
    if value["drm"] == true
        || value["is_drm"] == true
        || value["has_drm"] == true
        || value.to_string().contains("drm-protection")
    {
        return None;
    }
    value["directDownloadUrl"]
        .as_str()
        .or_else(|| value["download_url"].as_str())
        .or_else(|| value["url"].as_str())
        .and_then(|url| crate::model::validate_url(url).ok())
}

#[tauri::command]
pub async fn inspect_member_course(
    state: State<'_, Arc<AppState>>,
    provider: String,
    url: String,
    product_id: String,
    slug: String,
    limit: usize,
) -> Result<Course, String> {
    provider_name(&provider)?;
    if !identifier(&product_id)
        || !product_id.bytes().all(|b| b.is_ascii_digit())
        || !(1..=1000).contains(&limit)
        || provider == "hotmart" && !identifier(&slug)
    {
        return Err("Use the numeric course/product identifier, a valid Hotmart slug and a lesson limit from 1 to 1000.".into());
    }
    let normalized = crate::model::validate_url(&url)?;
    let mut course_url = url::Url::parse(&normalized).map_err(|_| "Use the course page URL.")?;
    let host = course_url.host_str().unwrap_or_default();
    let valid_host = if provider == "hotmart" {
        host == "hotmart.com" || host.ends_with(".hotmart.com")
    } else {
        host == "kiwify.com.br"
            || host.ends_with(".kiwify.com.br")
            || host == "kiwify.com"
            || host.ends_with(".kiwify.com")
    };
    if !valid_host {
        return Err("The course URL belongs to another provider.".into());
    }
    course_url.set_query(None);
    course_url.set_fragment(None);
    let config = crate::config_root(&state)?;
    let token = sessions::load_module_secret(&config, &format!("course-{provider}"))?;
    let token = std::str::from_utf8(&token).map_err(|_| "Reconnect the student session.")?;
    let endpoint = if provider == "hotmart" {
        "https://api-club-course-consumption-gateway-ga.cb.hotmart.com/v1/navigation".into()
    } else {
        format!("https://admin-api.kiwify.com/v1/viewer/clubs/{product_id}/content?caipirinha=true")
    };
    let navigation_value = match get_json(&provider, token, &endpoint, &product_id, &slug).await {
        Ok(value) => value,
        Err(error) if provider == "kiwify" && error.contains("HTTP 404") => {
            get_json(
                &provider,
                token,
                &format!("https://admin-api.kiwify.com.br/v1/viewer/courses/{product_id}"),
                &product_id,
                &slug,
            )
            .await?
        }
        Err(error) => return Err(error),
    };
    let mut course = Course {
        id: format!("{provider}-{product_id}"),
        title: super::label(
            navigation_value["title"]
                .as_str()
                .or_else(|| navigation_value["name"].as_str())
                .unwrap_or(&format!("{provider} {product_id}")),
            240,
        ),
        provider: provider.clone(),
        url: Some(course_url.to_string()),
        slug: (provider == "hotmart").then(|| slug.clone()),
        sections: navigation(&provider, &navigation_value, limit)?,
        created_at: queue::now(),
    };
    for lesson in course
        .sections
        .iter_mut()
        .flat_map(|section| &mut section.lessons)
    {
        let lesson_id = lesson
            .id
            .strip_prefix("lesson-")
            .ok_or("Invalid lesson identity.")?;
        let endpoint = if provider == "hotmart" {
            format!("https://api-club-course-consumption-gateway-ga.cb.hotmart.com/v2/web/lessons/{lesson_id}")
        } else {
            format!(
                "https://admin-api.kiwify.com/v1/viewer/courses/{product_id}/lesson/{lesson_id}"
            )
        };
        let value = get_json(&provider, token, &endpoint, &product_id, &slug).await?;
        let detail = value.get("lesson").unwrap_or(&value);
        let locked = detail["locked"] == true || detail["has_access"] == false;
        // Playback URLs and video codes alone never establish permission to save a lesson.
        let media_download = if !locked && detail["downloadable"] == true {
            detail["download_url"]
                .as_str()
                .and_then(|url| crate::model::validate_url(url).ok())
        } else {
            None
        };
        let mut materials = vec![(
            format!("video-{lesson_id}"),
            lesson.title.clone(),
            "video",
            media_download,
            detail["drm"] == true || detail["has_drm"] == true,
        )];
        if !locked {
            let attachments = if provider == "hotmart" {
                &detail["attachments"]
            } else {
                &detail["files"]
            };
            for attachment in attachments.as_array().into_iter().flatten().take(99) {
                let key = if provider == "hotmart" {
                    &attachment["fileMembershipId"]
                } else {
                    &attachment["id"]
                };
                let Some(file_id) = field_id(key) else {
                    continue;
                };
                let endpoint = if provider == "hotmart" {
                    format!("https://api-club-hot-club-api.cb.hotmart.com/rest/v3/attachment/{file_id}/download")
                } else {
                    format!("https://admin-api.kiwify.com.br/v1/viewer/courses/{product_id}/files/{file_id}?forceDownload=true")
                };
                let download_value =
                    get_json(&provider, token, &endpoint, &product_id, &slug).await?;
                let title = attachment["fileName"]
                    .as_str()
                    .or_else(|| attachment["name"].as_str())
                    .or_else(|| attachment["filename"].as_str())
                    .unwrap_or("Attachment");
                let drm = download_value["drm"] == true
                    || download_value["is_drm"] == true
                    || download_value.to_string().contains("drm-protection");
                materials.push((
                    format!("file-{}", super::digest(&format!("{lesson_id}\0{file_id}"))),
                    super::label(title, 500),
                    "file",
                    explicit_download(&download_value),
                    drm,
                ));
            }
        }
        for (id, title, kind, url, drm) in materials {
            let reference = if let Some(url) = &url {
                Some(sessions::seal_url(
                    &config,
                    &format!(
                        "course-{}",
                        super::digest(&format!("{provider}\0{id}\0{url}"))
                    ),
                    url,
                )?)
            } else {
                None
            };
            lesson.assets.push(CourseAsset {
                id,
                title,
                kind: kind.into(),
                status: if drm {
                    "drm"
                } else if reference.is_some() {
                    "downloadable"
                } else {
                    "unavailable"
                }
                .into(),
                restriction: locked.then(|| {
                    "This lesson is not accessible to the connected student account.".into()
                }),
                library_id: None,
                url_ref: reference,
                job_ids: Vec::new(),
            });
        }
    }
    course.validate()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    super::save_course(repo,course)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_navigation_uses_real_ids_and_preserves_order() {
        let hotmart = serde_json::json!({"modules":[{"id":"m1","name":"Section","pages":[{"hash":"l1","name":"First"},{"hash":"l2","name":"Second"}]}]});
        let sections = navigation("hotmart", &hotmart, 1).unwrap();
        assert_eq!(sections[0].lessons.len(), 1);
        assert_eq!(sections[0].lessons[0].id, "lesson-l1");
        let kiwify = serde_json::json!({"modules":{"m1":{"title":"Section","order":1,"lessons":{"later":{"title":"Later","order":2},"earlier":{"title":"Earlier","order":1}}}}});
        let sections = navigation("kiwify", &kiwify, 100).unwrap();
        assert_eq!(sections[0].lessons[0].id, "lesson-earlier");
        assert_eq!(sections[0].lessons[1].id, "lesson-later");
    }
    #[test]
    fn download_permission_excludes_drm_and_stream_only_metadata() {
        assert!(explicit_download(
            &serde_json::json!({"stream_link":"https://cdn.example.org/play.mp4"})
        )
        .is_none());
        assert!(explicit_download(
            &serde_json::json!({"url":"https://cdn.example.org/drm-protection/file.pdf"})
        )
        .is_none());
        assert_eq!(explicit_download(&serde_json::json!({"directDownloadUrl":"https://cdn.example.org/file.pdf?token=authorized"})).as_deref(),Some("https://cdn.example.org/file.pdf?token=authorized"));
    }
}
