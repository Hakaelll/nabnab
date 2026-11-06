use crate::{galleries::Asset, AppState};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tokio::sync::watch;

pub fn valid_source_id(value: &str) -> bool {
    value
        .strip_prefix("gallery-source:")
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub fn valid_asset_id(value: &str) -> bool {
    value
        .strip_prefix("gallery:")
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub fn normalized_url(input: &str) -> Result<String, String> {
    let normalized = crate::model::validate_url(input)?;
    if !crate::galleries::supported(&normalized) {
        return Err(
            "Choose a gallery source supported by the optional Galleries component.".into(),
        );
    }
    let mut url = url::Url::parse(&normalized).map_err(|_| "Invalid gallery source URL.")?;
    url.set_fragment(None);
    let query: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| {
            !key.to_ascii_lowercase().starts_with("utm_")
                && !matches!(key.as_ref(), "ref" | "si" | "share_id")
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Ok(url.to_string())
}
pub fn source_id(input: &str) -> Result<String, String> {
    Ok(format!(
        "gallery-source:{:x}",
        Sha256::digest(normalized_url(input)?.as_bytes())
    ))
}

pub fn enabled(app: &AppHandle) -> bool {
    crate::components::root(app).is_ok_and(|root| root.join("gallery.enabled").is_file())
}

pub fn active_checks(state: &AppState) -> bool {
    state
        .repository
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|repo| {
            repo.data.sources.iter().any(|source| {
                source.kind == crate::sources::SourceKind::Gallery
                    && source.status == crate::sources::SourceStatus::Checking
            })
        })
}

pub struct GalleryPage {
    pub name: Option<String>,
    pub assets: Vec<Asset>,
    pub inspected: usize,
    pub has_more: bool,
}

fn parse_page(raw: &str, input: &str, offset: usize, limit: usize) -> Result<GalleryPage, String> {
    let messages: Vec<Value> = serde_json::from_str(raw)
        .map_err(|_| "The gallery source returned unreadable metadata.")?;
    if let Some(error) = messages.iter().find(|message| message[0] == -1) {
        return Err(crate::failure::sanitize(
            error[1]["message"]
                .as_str()
                .unwrap_or("The gallery source could not be accessed."),
        ));
    }
    let name = messages
        .iter()
        .filter(|message| message[0] == 2)
        .find_map(|message| {
            ["board", "gallery", "album"]
                .into_iter()
                .find_map(|key| {
                    message[1][key]["name"]
                        .as_str()
                        .or_else(|| message[1][key].as_str())
                })
                .map(|value| {
                    value
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(240)
                        .collect()
                })
        });
    let files: Vec<_> = messages
        .into_iter()
        .filter(|message| message[0] == 3)
        .collect();
    let inspected = files.len().min(limit);
    let has_more = files.len() > limit;
    let mut assets = Vec::new();
    let mut known = std::collections::HashSet::new();
    for (index, message) in files.into_iter().take(limit).enumerate() {
        let raw = serde_json::to_string(&vec![message])
            .map_err(|_| "Could not read the gallery source attachment.")?;
        let parsed = crate::galleries::parse(&raw, input, offset + index, 1)?;
        if !parsed.warnings.is_empty() {
            return Err("This followed gallery includes manifest-only video attachments. Automatic checks cannot preserve those files through the direct gallery route.".into());
        }
        for asset in parsed.assets {
            if known.insert(asset.id.clone()) {
                assets.push(asset);
            }
        }
    }
    Ok(GalleryPage {
        name,
        assets,
        inspected,
        has_more,
    })
}

pub fn published_at(value: &str) -> Option<u64> {
    if let Ok(date) = httpdate::parse_http_date(value) {
        return date
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis() as u64);
    }
    // An explicit timezone is required before a date can authorize an automatic addition.
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b' '))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let number = |from: usize, to: usize| {
        value
            .get(from..to)
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))?
            .parse::<i64>()
            .ok()
    };
    let (year, month, day, hour, minute, second) = (
        number(0, 4)?,
        number(5, 7)?,
        number(8, 10)?,
        number(11, 13)?,
        number(14, 16)?,
        number(17, 19)?,
    );
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day < 1 || day > days[(month - 1) as usize] {
        return None;
    }
    let rest = value.get(19..)?;
    let timezone = if rest.starts_with('.') {
        let digits = rest[1..].chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        rest.get(1 + digits..)?
    } else {
        rest
    };
    let offset = if timezone == "Z" {
        0
    } else {
        let bytes = timezone.as_bytes();
        if bytes.len() != 6
            || !matches!(bytes.first(), Some(b'+' | b'-'))
            || bytes.get(3) != Some(&b':')
        {
            return None;
        }
        let hours = timezone.get(1..3)?.parse::<i64>().ok()?;
        let minutes = timezone.get(4..6)?.parse::<i64>().ok()?;
        if !(0..=23).contains(&hours) || !(0..=59).contains(&minutes) {
            return None;
        }
        (hours * 3600 + minutes * 60) * if bytes[0] == b'+' { 1 } else { -1 }
    };
    let adjusted = year - i64::from(month <= 2);
    let era = adjusted.div_euclid(400);
    let year_of_era = adjusted - era * 400;
    let shifted = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let days = era * 146097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year
        - 719468;
    let seconds = days * 86400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(seconds).ok()?.checked_mul(1000)
}

pub async fn inspect(
    app: &AppHandle,
    state: &AppState,
    input: &str,
    offset: usize,
    limit: usize,
    mut cancel: watch::Receiver<bool>,
) -> Result<GalleryPage, String> {
    if limit == 0 || limit > 1000 || offset > 100000 {
        return Err("Explore 1–1000 gallery files per page, up to 100000 positions.".into());
    }
    source_id(input)?;
    let executable = crate::components::executable(app, "gallery")?;
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let _permit = tokio::select! {permit=crate::batch::metadata_permit(input)=>permit?,_=cancel.changed()=>return Err(crate::engine::CANCELLED.into())};
    let input = crate::providers::validate_remote(input).await?;
    let root = crate::config_root(state)?;
    let cookies = crate::sessions::materialize(&root, &input)?;
    let config = crate::gallery_auth::materialize(&root, &input)?;
    let mut args: Vec<String> = [
        "--ignore-config",
        "--no-input",
        "--no-postprocessors",
        "--dump-json",
        "--range",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(format!("{}-{}", offset + 1, offset + limit + 1));
    args.extend([
        "-o".into(),
        "extractor.retries=2".into(),
        "-o".into(),
        "extractor.timeout=20".into(),
    ]);
    config.append_args(&mut args);
    if let Some(cookies) = &cookies {
        args.extend([
            "--cookies".into(),
            cookies.path.to_string_lossy().into_owned(),
        ]);
    }
    args.extend(["--".into(), input.clone()]);
    let raw = crate::engine::run_process(
        executable,
        args,
        cancel.clone(),
        crate::queue::now(),
        true,
        |_| {},
    )
    .await
    .map_err(|error| config.redact(error))?;
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    parse_page(&raw, &input, offset, limit).map_err(|error| config.redact(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_identity_ignores_tracking_and_rejects_unsupported_hosts() {
        assert_eq!(
            source_id("https://www.pinterest.com/author/board/?utm_source=share#top").unwrap(),
            source_id("https://www.pinterest.com/author/board/").unwrap()
        );
        assert_ne!(
            source_id("https://www.pinterest.com/author/board/?section=1").unwrap(),
            source_id("https://www.pinterest.com/author/board/?section=2").unwrap()
        );
        assert!(source_id("https://pinterest.com.attacker.org/author/board/").is_err());
    }
    #[test]
    fn file_identity_is_independent_from_followed_collection() {
        let raw = r#"[[3,"https://i.pinimg.com/originals/file.jpg?token=one",{"extension":"jpg","width":800,"height":600}]]"#;
        let first =
            crate::galleries::parse(raw, "https://pinterest.com/author/board/", 0, 50).unwrap();
        let second = crate::galleries::parse(raw, "https://pinterest.com/pin/123/", 0, 50).unwrap();
        assert_eq!(first.assets[0].id, second.assets[0].id);
        assert!(valid_asset_id(&first.assets[0].id));
        assert!(crate::galleries::parse(
            r#"[[-1,{"message":"Session required"}]]"#,
            "https://pinterest.com/pin/123/",
            0,
            50
        )
        .is_err());
    }
    #[test]
    fn unsupported_files_still_advance_the_real_source_cursor() {
        let raw = r#"[[3,"https://cdn.example.org/notes.pdf",{"extension":"pdf"}],[3,"https://cdn.example.org/image.jpg",{"extension":"jpg"}],[3,"https://cdn.example.org/later.jpg",{"extension":"jpg"}]]"#;
        let page = parse_page(raw, "https://pinterest.com/author/board/", 10, 2).unwrap();
        assert_eq!(page.inspected, 2);
        assert!(page.has_more);
        assert_eq!(page.assets.len(), 1);
        assert_eq!(page.assets[0].position, 12);
    }
    #[test]
    fn automatic_dates_require_valid_explicit_timezones() {
        assert_eq!(published_at("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            published_at("2015-10-21T09:28:00+02:00"),
            Some(1445412480000)
        );
        assert_eq!(
            published_at("2015-10-21T07:28:00.123Z"),
            Some(1445412480000)
        );
        assert!(published_at("2026-02-30T12:00:00Z").is_none());
        assert!(published_at("2026-11-06 12:00:00").is_none());
        assert!(published_at("2026-11-06T12:00:00+-1:00").is_none());
    }
}
