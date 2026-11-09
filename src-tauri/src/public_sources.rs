use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::watch;

mod arxiv;
mod librivox;
pub use librivox::{catalog_languages as librivox_languages, CatalogLanguage};
mod zenodo;

const PAGE_SIZE: usize = 50;
const MAX_PAGES: usize = 100;
const MAX_JSON: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Github,
    Archive,
    Commons,
    Arxiv,
    Zenodo,
    Librivox,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudiobookChapter {
    pub book_id: String,
    pub book_title: String,
    pub chapter: u32,
    pub reader: Option<String>,
    pub language: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub source_url: String,
    pub download_url: String,
    pub filename: String,
    pub mime: String,
    pub size: Option<u64>,
    pub title: String,
    pub version: Option<String>,
    pub original: bool,
    pub author: Option<String>,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub sha256: Option<String>,
    pub platform: Option<String>,
    pub architecture: Option<String>,
    pub published_at: Option<String>,
    pub checksum: Option<String>,
    pub audiobook: Option<AudiobookChapter>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub prerelease: bool,
    pub published_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub provider: Provider,
    pub source_url: String,
    pub title: String,
    pub description: Option<String>,
    pub rights: Option<String>,
    pub entries: Vec<Entry>,
    pub assets: Vec<Asset>,
    pub next_cursor: Option<String>,
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zenodo: Option<ZenodoRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZenodoRecord {
    pub record_id: String,
    pub family_id: Option<String>,
    pub version: Option<String>,
    pub versions_url: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Target {
    Github {
        owner: String,
        repo: String,
        tag: Option<String>,
    },
    Archive {
        identifier: String,
    },
    Commons {
        title: String,
        category: bool,
    },
    Arxiv(arxiv::ArxivTarget),
    Zenodo(zenodo::ZenodoTarget),
    Librivox(librivox::LibrivoxTarget),
}

pub(crate) fn percent_decode(input: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut source = input.bytes();
    while let Some(byte) = source.next() {
        if byte == b'%' {
            let high = source.next().and_then(|b| (b as char).to_digit(16));
            let low = source.next().and_then(|b| (b as char).to_digit(16));
            bytes.push(match (high, low) {
                (Some(high), Some(low)) => (high * 16 + low) as u8,
                _ => return Err("The source URL contains invalid percent encoding.".into()),
            });
        } else {
            bytes.push(byte);
        }
    }
    let value = String::from_utf8(bytes).map_err(|_| "The source URL is not UTF-8.")?;
    if value.chars().any(char::is_control) {
        return Err("The source URL contains control characters.".into());
    }
    Ok(value)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn target(input: &str) -> Result<Target, String> {
    let input = input.trim();
    if input.len() > 4096 || input.chars().any(char::is_control) {
        return Err(
            "The public source URL exceeds its limit or contains control characters.".into(),
        );
    }
    let normalized = arxiv::normalize(input).or_else(|| zenodo::normalize(input));
    let parsed = url::Url::parse(&crate::model::validate_url(normalized.as_deref().unwrap_or(input))?)
        .map_err(|_| "Use a supported public source URL.")?;
    if parsed.port().is_some() || !matches!(parsed.scheme(), "https" | "http") {
        return Err(
            "Public sources use their official HTTPS endpoints without a custom port.".into(),
        );
    }
    let parts: Vec<_> = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .map(percent_decode)
        .collect::<Result<_, _>>()?;
    match parsed.host_str().unwrap_or_default() {
        "arxiv.org" | "www.arxiv.org" | "export.arxiv.org" => arxiv::parse(&parsed, &parts).map(Target::Arxiv),
        "zenodo.org" | "www.zenodo.org" | "doi.org" | "dx.doi.org" => zenodo::parse(&parsed, &parts).map(Target::Zenodo),
        "librivox.org" | "www.librivox.org" => librivox::parse(&parsed, &parts).map(Target::Librivox),
        "github.com" | "www.github.com" => {
            let (owner, repo, tag) = match parts.as_slice() {
                [owner, repo] => (owner, repo, None),
                [owner, repo, releases] if releases == "releases" => (owner, repo, None),
                [owner, repo, releases, tag_path, tag @ ..]
                    if releases == "releases" && tag_path == "tag" && !tag.is_empty() =>
                {
                    (owner, repo, Some(tag.join("/")))
                }
                _ => return Err("Use a GitHub repository or /releases/tag/version URL.".into()),
            };
            let repo = repo.strip_suffix(".git").unwrap_or(repo);
            if !identifier(owner)
                || !identifier(repo)
                || tag
                    .as_ref()
                    .is_some_and(|tag| tag.len() > 512 || tag.is_empty())
            {
                return Err("The GitHub repository or release identifier is invalid.".into());
            }
            Ok(Target::Github {
                owner: owner.clone(),
                repo: repo.into(),
                tag,
            })
        }
        "archive.org" | "www.archive.org" => match parts.as_slice() {
            [route, id] if matches!(route.as_str(), "details" | "download") && identifier(id) => {
                Ok(Target::Archive {
                    identifier: id.clone(),
                })
            }
            _ => {
                Err("Use an Internet Archive item or collection URL at /details/identifier.".into())
            }
        },
        "commons.wikimedia.org" => {
            let title = match parts.as_slice() {
                [wiki, title @ ..] if wiki == "wiki" && !title.is_empty() => title.join("/"),
                [w, index] if w == "w" && index == "index.php" => parsed
                    .query_pairs()
                    .find(|(key, _)| key == "title")
                    .map(|(_, value)| value.into_owned())
                    .ok_or("The Commons URL has no file or category title.")?,
                _ => return Err("Use a Wikimedia Commons File: or Category: page.".into()),
            };
            if title.len() > 1024
                || title.chars().any(char::is_control)
                || title.contains('|')
                || title.contains('#')
                || title.contains('[')
                || title.contains(']')
            {
                return Err("The Commons page title is invalid.".into());
            }
            let category = title.starts_with("Category:");
            if !(category || title.starts_with("File:"))
                || title
                    .split_once(':')
                    .is_none_or(|(_, name)| name.trim().is_empty())
            {
                return Err("Use a Wikimedia Commons File: or Category: page.".into());
            }
            Ok(Target::Commons { title, category })
        }
        _ => Err(
            "Use a supported GitHub, Archive, Commons, arXiv, Zenodo or LibriVox source."
                .into(),
        ),
    }
}

pub fn supported(input: &str) -> bool {
    target(input).is_ok()
}

pub fn canonical_followed_search(input: &str) -> Result<String, String> {
    let target = target(input)?;
    if !matches!(target, Target::Arxiv(arxiv::ArxivTarget::Search { .. })
        | Target::Librivox(librivox::LibrivoxTarget::Search { .. } | librivox::LibrivoxTarget::Language(_))
        | Target::Zenodo(zenodo::ZenodoTarget::Versions(_) | zenodo::ZenodoTarget::Community(_)))
    {
        return Err("Choose an arXiv or LibriVox search, or a Zenodo community or version history to follow.".into());
    }
    Ok(source_url(&target))
}

pub async fn prepare_followed_search(input: &str) -> Result<String, String> {
    let canonical = canonical_followed_search(input)?;
    if let Target::Zenodo(zenodo::ZenodoTarget::Community(identifier)) = target(&canonical)? {
        let (id, _) = zenodo::community_info(&identifier).await?;
        return Ok(zenodo::canonical(&zenodo::ZenodoTarget::Community(id)));
    }
    Ok(canonical)
}

pub fn followed_search_page_size(input: &str) -> Result<usize, String> {
    let canonical = canonical_followed_search(input)?;
    Ok(if matches!(target(&canonical)?, Target::Librivox(librivox::LibrivoxTarget::Language(_))) {
        librivox::LANGUAGE_PAGE_SIZE
    } else {
        20
    })
}

pub fn valid_followed_search_entry(search: &str, entry_url: &str, entry_id: &str) -> bool {
    let Ok(entry) = target(entry_url) else { return false; };
    if source_url(&entry) != entry_url { return false; }
    match (target(search), entry) {
        (Ok(Target::Arxiv(arxiv::ArxivTarget::Search { .. })), Target::Arxiv(arxiv::ArxivTarget::Article(id))) =>
            id.matches_search_entry(entry_id),
        (Ok(Target::Librivox(librivox::LibrivoxTarget::Search { .. })), Target::Librivox(librivox::LibrivoxTarget::Edition(id))) =>
            entry_id == format!("librivox:{id}"),
        (Ok(Target::Librivox(librivox::LibrivoxTarget::Language(_))), Target::Librivox(librivox::LibrivoxTarget::CatalogPage(slug))) =>
            entry_id == format!("librivox:catalog:{slug}"),
        (Ok(Target::Zenodo(zenodo::ZenodoTarget::Versions(_) | zenodo::ZenodoTarget::Community(_))), Target::Zenodo(zenodo::ZenodoTarget::Record(id))) =>
            entry_id == format!("zenodo:{id}"),
        _ => false,
    }
}

pub fn github_repository_url(input: &str) -> Result<String, String> {
    match target(input)? {
        Target::Github { owner, repo, .. } => Ok(source_url(&Target::Github {
            owner,
            repo,
            tag: None,
        })),
        _ => Err("Choose a public GitHub repository to follow its releases.".into()),
    }
}

pub fn github_release_url(input: &str) -> Result<String, String> {
    match target(input)? {
        release @ Target::Github { tag: Some(_), .. } => Ok(source_url(&release)),
        _ => Err("Choose a specific GitHub release to select its files.".into()),
    }
}

fn url_path(base: &str, parts: &[&str]) -> String {
    let mut parsed = url::Url::parse(base).expect("constant public source origin");
    parsed
        .path_segments_mut()
        .expect("constant HTTPS source URL")
        .extend(parts);
    parsed.to_string()
}

fn source_url(target: &Target) -> String {
    match target {
        Target::Arxiv(target) => arxiv::canonical(target),
        Target::Zenodo(target) => zenodo::canonical(target),
        Target::Librivox(target) => librivox::canonical(target),
        Target::Github {
            owner,
            repo,
            tag: None,
        } => url_path("https://github.com", &[owner, repo]),
        Target::Github {
            owner,
            repo,
            tag: Some(tag),
        } => url_path("https://github.com", &[owner, repo, "releases", "tag", tag]),
        Target::Archive { identifier } => url_path("https://archive.org", &["details", identifier]),
        Target::Commons { title, .. } => {
            url_path("https://commons.wikimedia.org", &["wiki", title])
        }
    }
}

pub(crate) fn research_share_url(link: &str, asset_id: &str, attribution: Option<&str>) -> Option<String> {
    let parsed = target(link).ok()?;
    if source_url(&parsed) != link {
        return None;
    }
    match parsed {
        Target::Arxiv(arxiv::ArxivTarget::Article(article))
            if article.matches_search_entry(asset_id) && arxiv::normalize(asset_id).as_deref() == Some(link) => Some(link.into()),
        Target::Zenodo(zenodo::ZenodoTarget::Record(id))
            if asset_id.starts_with(&format!("zenodo:{id}:")) => Some(link.into()),
        Target::Librivox(librivox::LibrivoxTarget::Edition(id))
            if asset_id.starts_with(&format!("librivox:{id}:")) => {
            // Older chapter entries already carry the human-facing catalog page in their attribution.
            let catalog = attribution?.lines().nth(2)?.strip_prefix(librivox::ATTRIBUTION_SOURCE)?;
            let parsed = target(catalog).ok()?;
            if matches!(parsed, Target::Librivox(librivox::LibrivoxTarget::CatalogPage(_)))
                && source_url(&parsed) == catalog
            {
                Some(catalog.into())
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(crate) fn clean_text(input: &str, max: usize) -> String {
    // These fields are plain text, never trusted HTML. Drop script/style bodies too.
    let lower = input.to_ascii_lowercase();
    let mut output = String::new();
    let mut position = 0;
    while position < input.len() && output.chars().count() < max {
        let rest = &input[position..];
        if rest.starts_with('<') {
            let skip_body = ["script", "style"].into_iter().find(|name| {
                let marker = format!("<{name}");
                lower[position..].starts_with(&marker)
                    && lower[position + marker.len()..]
                        .chars()
                        .next()
                        .is_some_and(|ch| ch == '>' || ch.is_ascii_whitespace())
            });
            if let Some(name) = skip_body {
                let end = format!("</{name}");
                if let Some(offset) = lower[position..].find(&end) {
                    position += offset;
                } else {
                    break;
                }
            }
            let mut quoted = None;
            let mut closed = false;
            for (offset, ch) in input[position..].char_indices() {
                match ch {
                    '\'' | '"' if quoted == Some(ch) => quoted = None,
                    '\'' | '"' if quoted.is_none() => quoted = Some(ch),
                    '>' if quoted.is_none() => {
                        position += offset + 1;
                        closed = true;
                        break;
                    }
                    _ => {}
                }
            }
            if !closed {
                break;
            }
            output.push(' ');
        } else if rest.starts_with('&') {
            let entity = rest
                .find(';')
                .filter(|index| *index <= 12)
                .map(|end| (&rest[1..end], end));
            let decoded = entity.and_then(|(name, end)| {
                let value = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" => Some('\''),
                    "nbsp" => Some(' '),
                    _ if name.starts_with("#x") || name.starts_with("#X") => {
                        u32::from_str_radix(&name[2..], 16)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    _ if name.starts_with('#') => name[1..].parse().ok().and_then(char::from_u32),
                    _ => None,
                }?;
                Some((value, end))
            });
            if let Some((ch, end)) = decoded {
                if !ch.is_control() {
                    output.push(ch);
                }
                position += end + 1;
            } else {
                output.push('&');
                position += 1;
            }
        } else {
            let ch = rest.chars().next().expect("nonempty text");
            if !ch.is_control() || ch.is_whitespace() {
                output.push(ch);
            }
            position += ch.len_utf8();
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn field(value: &Value, key: &str, max: usize) -> Option<String> {
    let raw = value.get(key)?;
    let result = match raw {
        Value::String(text) => clean_text(text, max),
        Value::Array(values) => clean_text(
            &values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("; "),
            max,
        ),
        _ => return None,
    };
    (!result.is_empty()).then_some(result)
}

fn truthy(value: &Value) -> bool {
    value.as_bool() == Some(true)
        || value.as_u64() == Some(1)
        || value
            .as_str()
            .is_some_and(|text| matches!(text.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
}

fn page(target: &Target, provider: Provider, title: String) -> Page {
    Page {
        provider,
        source_url: source_url(target),
        title,
        description: None,
        rights: None,
        entries: vec![],
        assets: vec![],
        next_cursor: None,
        warnings: vec![],
        zenodo: None,
    }
}

fn numeric_page(cursor: Option<&str>) -> Result<usize, String> {
    let number = cursor.unwrap_or("1");
    if number.is_empty() || number.len() > 3 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid source page cursor.".into());
    }
    number
        .parse::<usize>()
        .ok()
        .filter(|page| (1..=MAX_PAGES).contains(page))
        .ok_or_else(|| "The source page exceeds the 100-page exploration limit.".into())
}

fn next_page(page: &mut Page, current: usize, more: bool) {
    if more && current < MAX_PAGES {
        page.next_cursor = Some((current + 1).to_string());
    }
    if more && current == MAX_PAGES {
        page.warnings.push(
            "Exploration stopped at the 100-page limit. Only the loaded portion is available."
                .into(),
        );
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

const PUBLIC_NOT_FOUND: &str = "The public source item was not found or is not publicly accessible.";

fn zenodo_record_api(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("zenodo.org")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().strip_prefix("/api/records/").is_some_and(|id| {
            id.bytes().all(|byte| byte.is_ascii_digit()) && id.parse::<u64>().is_ok_and(|id| id > 0)
        })
}

async fn api_json(input: &str) -> Result<Value, String> {
    let bytes = api_bytes(input, "application/json").await?;
    serde_json::from_slice(&bytes)
        .map_err(|_| "The source did not return valid public metadata JSON.".into())
}

async fn api_bytes(input: &str, accept: &str) -> Result<Vec<u8>, String> {
    let parsed = url::Url::parse(input).map_err(|_| "Invalid public API URL.")?;
    let host = parsed.host_str().ok_or("The public API URL has no host.")?;
    if parsed.scheme() != "https"
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !matches!(
            host,
            "api.github.com" | "archive.org" | "commons.wikimedia.org" | "export.arxiv.org" | "arxiv.org" | "zenodo.org" | "librivox.org"
        )
    {
        return Err("Only official public source APIs are allowed.".into());
    }
    let addresses: Vec<_> =
        tokio::time::timeout(Duration::from_secs(8), tokio::net::lookup_host((host, 443)))
            .await
            .map_err(|_| "The public API DNS lookup timed out.")?
            .map_err(|_| "The public API hostname could not be resolved.")?
            .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !crate::providers::is_public_ip(address.ip()))
    {
        return Err("Local or reserved public API destinations are not allowed.".into());
    }
    let redirects = if zenodo_record_api(&parsed) {
        reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() == 1 && zenodo_record_api(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        })
    } else {
        reqwest::redirect::Policy::none()
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(redirects)
        .resolve_to_addrs(host, &addresses)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(45))
        .user_agent(concat!(
            "nabnab/",
            env!("CARGO_PKG_VERSION"),
            " (local public-file browser; https://github.com/Hakaelll/nabnab)"
        ))
        .build()
        .map_err(|_| "Could not prepare the public source request.")?;
    let mut request = client
        .get(parsed.clone())
        .header(reqwest::header::ACCEPT, accept)
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if host == "librivox.org" && parsed.path() == "/search/get_results" {
        request = request.header("X-Requested-With", "XMLHttpRequest");
    }
    let mut response = request.send()
        .await
        .map_err(|_| "The public source request failed. Check the connection and retry.")?;
    let status = response.status().as_u16();
    match status {
        200 => {}
        401 | 403 => return Err("This source denied access or exhausted its public API allowance. No authentication or access bypass was attempted.".into()),
        404 => return Err(PUBLIC_NOT_FOUND.into()),
        429 => return Err("The public source requested a pause. Retry later.".into()),
        _ => return Err(format!("The official public source API returned HTTP {status}.")),
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_JSON as u64)
    {
        return Err("The public metadata exceeds the 8 MiB limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The public metadata transfer was interrupted.")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_JSON {
            return Err("The public metadata exceeds the 8 MiB limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn safe_download_url(input: &str, provider: Provider) -> Result<String, String> {
    let mut parsed =
        url::Url::parse(&crate::model::validate_url(input)?).map_err(|_| "Invalid file URL.")?;
    let host = parsed.host_str().unwrap_or_default();
    let approved = match provider {
        Provider::Github => host == "github.com",
        Provider::Archive => host == "archive.org",
        Provider::Commons => host == "upload.wikimedia.org",
        Provider::Arxiv => host == "arxiv.org",
        Provider::Zenodo => host == "zenodo.org",
        Provider::Librivox => host == "archive.org" || host == "librivox.org",
    };
    // Imageinfo analytics parameters are unnecessary for file access and must not be persisted.
    if provider == Provider::Commons
        && parsed.query().is_some()
        && parsed
            .query_pairs()
            .all(|(key, _)| matches!(key.as_ref(), "utm_source" | "utm_campaign" | "utm_content"))
    {
        parsed.set_query(None);
    }
    if !approved
        || parsed.scheme() != "https"
        || parsed.port().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("The source returned an unexpected or temporary file URL.".into());
    }
    Ok(parsed.to_string())
}

fn filename(input: &str) -> Option<String> {
    let basename = input.rsplit('/').next()?;
    if basename.is_empty()
        || basename == "."
        || basename == ".."
        || basename.chars().any(char::is_control)
    {
        return None;
    }
    let name: String = basename
        .chars()
        .take(200)
        .map(|ch| if "<>:\"\\|?*".contains(ch) { '_' } else { ch })
        .collect();
    let name = name.trim_end_matches([' ', '.']).to_string();
    (!name.is_empty()).then_some(name)
}

fn hex_digest(input: &str, length: usize) -> Option<String> {
    (input.len() == length && input.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| input.to_ascii_lowercase())
}

fn inferred_target(name: &str) -> (Option<String>, Option<String>) {
    let tokens: Vec<_> = name
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect();
    let contains = |options: &[&str]| tokens.iter().any(|token| options.contains(&token.as_str()));
    let mut systems = vec![];
    if contains(&["windows", "win32", "win64", "win"]) {
        systems.push("Windows");
    }
    if contains(&["linux", "appimage"]) {
        systems.push("Linux");
    }
    if contains(&["macos", "darwin", "osx"]) {
        systems.push("macOS");
    }
    let mut architectures = vec![];
    if contains(&["arm64", "aarch64"]) {
        architectures.push("ARM64");
    }
    if contains(&["x64", "amd64", "x86_64"]) || tokens.windows(2).any(|pair| pair == ["x86", "64"])
    {
        architectures.push("x64");
    }
    if contains(&["i386", "i686", "win32"]) {
        architectures.push("x86");
    }
    (
        if systems.len() == 1 {
            Some(systems[0].into())
        } else {
            None
        },
        if architectures.len() == 1 {
            Some(architectures[0].into())
        } else {
            None
        },
    )
}

fn github_asset(value: &Value, target: &Target, version: &str) -> Result<Asset, String> {
    let Target::Github { owner, repo, .. } = target else {
        return Err("Invalid GitHub source.".into());
    };
    let id = value["id"]
        .as_u64()
        .ok_or("The release asset has no numeric identifier.")?;
    let name = value["name"]
        .as_str()
        .filter(|name| !name.contains('/') && !name.contains('\\'))
        .ok_or("The release asset has an invalid filename.")?;
    let download = safe_download_url(
        value["browser_download_url"]
            .as_str()
            .ok_or("The release asset has no download URL.")?,
        Provider::Github,
    )?;
    let expected = url_path(
        "https://github.com",
        &[owner, repo, "releases", "download", version, name],
    );
    if download != expected {
        return Err(
            "The release asset URL does not match the selected repository and version.".into(),
        );
    }
    let (platform, architecture) = inferred_target(name);
    Ok(Asset {
        id: format!("github:{id}"),
        source_url: source_url(target),
        download_url: download,
        filename: filename(name).ok_or("The release asset filename is invalid.")?,
        mime: field(value, "content_type", 120)
            .unwrap_or_else(|| "application/octet-stream".into()),
        size: value["size"].as_u64(),
        title: field(value, "label", 240).unwrap_or_else(|| name.into()),
        version: Some(version.into()),
        original: true,
        author: field(&value["uploader"], "login", 200),
        license: None,
        attribution: None,
        sha256: value["digest"]
            .as_str()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .and_then(|digest| hex_digest(digest, 64)),
        platform,
        architecture,
        published_at: None,
        checksum: None,
        audiobook: None,
    })
}

fn github_releases(value: &Value, target: &Target, current: usize) -> Result<Page, String> {
    let Target::Github { owner, repo, .. } = target else {
        return Err("Invalid GitHub source.".into());
    };
    let releases = value
        .as_array()
        .ok_or("The repository did not return a release list.")?;
    let mut result = page(target, Provider::Github, format!("{owner}/{repo}"));
    for release in releases.iter().take(PAGE_SIZE) {
        if truthy(&release["draft"]) {
            continue;
        }
        let tag = release["tag_name"].as_str().filter(|tag| {
            !tag.is_empty() && tag.len() <= 512 && !tag.chars().any(char::is_control)
        });
        let Some(tag) = tag else {
            continue;
        };
        let Some(id) = release["id"].as_u64() else {
            continue;
        };
        result.entries.push(Entry {
            id: format!("release:{id}"),
            title: field(release, "name", 240).unwrap_or_else(|| tag.into()),
            url: source_url(&Target::Github {
                owner: owner.clone(),
                repo: repo.clone(),
                tag: Some(tag.into()),
            }),
            prerelease: truthy(&release["prerelease"]),
            published_at: field(release, "published_at", 64),
        });
    }
    next_page(&mut result, current, releases.len() >= PAGE_SIZE);
    result.warnings.push("Downloading a release does not execute or install it. Platform and architecture labels are filename estimates; select the file you need.".into());
    Ok(result)
}

async fn github(target: &Target, cursor: Option<&str>) -> Result<Page, String> {
    let Target::Github { owner, repo, tag } = target else {
        return Err("Invalid GitHub source.".into());
    };
    let current = numeric_page(cursor)?;
    if let Some(tag) = tag {
        let release = api_json(&url_path(
            "https://api.github.com",
            &["repos", owner, repo, "releases", "tags", tag],
        ))
        .await?;
        if truthy(&release["draft"]) {
            return Err("Draft releases are not public download sources.".into());
        }
        let release_id = release["id"]
            .as_u64()
            .ok_or("The public release has no numeric identifier.")?
            .to_string();
        let version = release["tag_name"]
            .as_str()
            .ok_or("The release has no version tag.")?;
        if version != tag {
            return Err("The returned release does not match the requested version.".into());
        }
        let mut api = url::Url::parse(&url_path(
            "https://api.github.com",
            &["repos", owner, repo, "releases", &release_id, "assets"],
        ))
        .map_err(|_| "Invalid release endpoint.")?;
        api.query_pairs_mut()
            .append_pair("per_page", &PAGE_SIZE.to_string())
            .append_pair("page", &current.to_string());
        let raw = api_json(api.as_str()).await?;
        let assets = raw
            .as_array()
            .ok_or("The release did not return an asset list.")?;
        let mut result = page(
            target,
            Provider::Github,
            field(&release, "name", 240).unwrap_or_else(|| tag.into()),
        );
        result.description = field(&release, "body", 6000);
        for asset in assets.iter().take(PAGE_SIZE) {
            if asset["state"]
                .as_str()
                .is_some_and(|state| state != "uploaded")
            {
                continue;
            }
            result.assets.push(github_asset(asset, target, version)?);
        }
        next_page(&mut result, current, assets.len() >= PAGE_SIZE);
        result.warnings.push(
            if truthy(&release["prerelease"]) {
                "Prerelease: this version may be unfinished."
            } else {
                "Published stable release."
            }
            .into(),
        );
        result.warnings.push("A checksum checks file integrity, not the publisher's identity. Downloaded software is never run or installed.".into());
        Ok(result)
    } else {
        let mut api = url::Url::parse(&url_path(
            "https://api.github.com",
            &["repos", owner, repo, "releases"],
        ))
        .map_err(|_| "Invalid repository endpoint.")?;
        api.query_pairs_mut()
            .append_pair("per_page", &PAGE_SIZE.to_string())
            .append_pair("page", &current.to_string());
        github_releases(&api_json(api.as_str()).await?, target, current)
    }
}

fn archive_restricted(value: &Value) -> bool {
    [
        "is_dark",
        "nodownload",
        "is_lending_required",
        "access-restricted-item",
        "restricted",
    ]
    .iter()
    .any(|key| truthy(&value[*key]) || truthy(&value["metadata"][*key]))
        || truthy(&value["lendingInfo"]["is_lendable"])
}

fn archive_filename_allowed(name: &str, file: &Value) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && name.len() <= 2048
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
        && !name
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        && !["private", "restricted", "drm"]
            .iter()
            .any(|key| truthy(&file[*key]))
        && ![".acs", ".acsm", "_encrypted.pdf", "_lcp.epub", "_lcp.pdf"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
        && !file["format"].as_str().is_some_and(|format| {
            let format = format.to_ascii_lowercase();
            format.contains("encrypted") || format.contains("drm")
        })
}

fn mime_for_name(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "epub" => "application/epub+zip",
        "zip" => "application/zip",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "tif" | "tiff" => "image/tiff",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn archive_asset(file: &Value, metadata: &Value, target: &Target) -> Result<Option<Asset>, String> {
    let Target::Archive { identifier } = target else {
        return Err("Invalid Archive source.".into());
    };
    let Some(name) = file["name"].as_str() else {
        return Ok(None);
    };
    if !archive_filename_allowed(name, file) {
        return Ok(None);
    }
    let mut parts = vec!["download", identifier.as_str()];
    parts.extend(name.split('/'));
    let download = safe_download_url(&url_path("https://archive.org", &parts), Provider::Archive)?;
    let rights = field(metadata, "rights", 3000).or_else(|| field(metadata, "licenseurl", 1000));
    let author = field(metadata, "creator", 1000);
    let size = file["size"]
        .as_u64()
        .or_else(|| file["size"].as_str().and_then(|size| size.parse().ok()));
    let attribution = Some(format!("Internet Archive item: {identifier}\nSource: {}\nCreator: {}\nRights supplied by source: {}\nOriginal source: {}",
        source_url(target), author.as_deref().unwrap_or("Not supplied"), rights.as_deref().unwrap_or("Not supplied; no license inferred"),
        field(metadata, "source", 2000).as_deref().unwrap_or("Not supplied")));
    Ok(Some(Asset {
        id: format!("archive:{name}"),
        source_url: source_url(target),
        download_url: download,
        filename: filename(name).ok_or("Invalid Archive filename.")?,
        mime: mime_for_name(name).into(),
        size,
        title: clean_text(name, 240),
        version: field(file, "mtime", 64),
        original: file["source"].as_str() == Some("original"),
        author,
        license: rights,
        attribution,
        sha256: file["sha256"]
            .as_str()
            .and_then(|digest| hex_digest(digest, 64)),
        platform: None,
        architecture: None,
        published_at: None,
        checksum: None,
        audiobook: None,
    }))
}

fn archive_files(value: &Value, target: &Target, current: usize) -> Result<Page, String> {
    if archive_restricted(value) {
        return Err("This Internet Archive item is restricted, unavailable or requires lending. Restricted download routes are not supported.".into());
    }
    let metadata = &value["metadata"];
    if !metadata.is_object() {
        return Err("The Archive item has no public metadata.".into());
    }
    let Target::Archive { identifier } = target else {
        return Err("Invalid Archive source.".into());
    };
    let mut result = page(
        target,
        Provider::Archive,
        field(metadata, "title", 240).unwrap_or_else(|| identifier.clone()),
    );
    result.description = field(metadata, "description", 6000);
    result.rights = field(metadata, "rights", 3000).or_else(|| field(metadata, "licenseurl", 1000));
    let files = value["files"]
        .as_array()
        .ok_or("The item did not return a public file list.")?;
    let offset = (current - 1) * PAGE_SIZE;
    for file in files.iter().skip(offset).take(PAGE_SIZE) {
        if let Some(asset) = archive_asset(file, metadata, target)? {
            result.assets.push(asset);
        }
    }
    next_page(&mut result, current, files.len() > offset + PAGE_SIZE);
    result.warnings.push("Choose originals or derivatives explicitly. Availability does not establish public-domain status; review each item's rights and provenance.".into());
    if result.rights.is_none() {
        result.warnings.push(
            "This item did not supply rights or a license. No legal status has been inferred."
                .into(),
        );
    }
    Ok(result)
}

async fn archive(target: &Target, cursor: Option<&str>) -> Result<Page, String> {
    let Target::Archive { identifier } = target else {
        return Err("Invalid Archive source.".into());
    };
    let current = numeric_page(cursor)?;
    let raw = api_json(&url_path("https://archive.org", &["metadata", identifier])).await?;
    if archive_restricted(&raw) {
        return Err("This Internet Archive item is restricted, unavailable or requires lending. Restricted download routes are not supported.".into());
    }
    if raw["metadata"]["mediatype"].as_str() == Some("collection") || truthy(&raw["is_collection"])
    {
        let mut api = url::Url::parse("https://archive.org/advancedsearch.php")
            .expect("constant Archive search endpoint");
        api.query_pairs_mut()
            .append_pair("q", &format!("collection:{identifier}"))
            .append_pair("fl[]", "identifier")
            .append_pair("fl[]", "title")
            .append_pair("sort[]", "identifier asc")
            .append_pair("rows", &PAGE_SIZE.to_string())
            .append_pair("page", &current.to_string())
            .append_pair("output", "json");
        let search = api_json(api.as_str()).await?;
        let docs = search["response"]["docs"]
            .as_array()
            .ok_or("The Archive collection did not return search results.")?;
        let mut result = page(
            target,
            Provider::Archive,
            field(&raw["metadata"], "title", 240).unwrap_or_else(|| identifier.clone()),
        );
        result.description = field(&raw["metadata"], "description", 6000);
        result.rights = field(&raw["metadata"], "rights", 3000)
            .or_else(|| field(&raw["metadata"], "licenseurl", 1000));
        for doc in docs.iter().take(PAGE_SIZE) {
            let Some(id) = doc["identifier"]
                .as_str()
                .filter(|id| identifier_valid_archive(id))
            else {
                continue;
            };
            if id == identifier {
                continue;
            }
            result.entries.push(Entry {
                id: format!("archive-item:{id}"),
                title: field(doc, "title", 240).unwrap_or_else(|| id.into()),
                url: source_url(&Target::Archive {
                    identifier: id.into(),
                }),
                prerelease: false,
                published_at: None,
            });
        }
        let more = search["response"]["numFound"]
            .as_u64()
            .map_or(docs.len() >= PAGE_SIZE, |total| {
                total > (current * PAGE_SIZE) as u64
            });
        next_page(&mut result, current, more);
        result.warnings.push("Only direct collection members are listed. Open an item to review its files and rights; no collection is downloaded automatically.".into());
        Ok(result)
    } else {
        archive_files(&raw, target, current)
    }
}

fn identifier_valid_archive(value: &str) -> bool {
    identifier(value)
}

fn commons_api() -> url::Url {
    let mut api = url::Url::parse("https://commons.wikimedia.org/w/api.php")
        .expect("constant Commons API endpoint");
    api.query_pairs_mut()
        .append_pair("action", "query")
        .append_pair("format", "json")
        .append_pair("formatversion", "2")
        .append_pair("maxlag", "5");
    api
}

fn commons_metadata(info: &Value, name: &str, max: usize) -> Option<String> {
    let raw = info["extmetadata"][name]["value"].as_str()?;
    let text = clean_text(raw, max);
    (!text.is_empty()).then_some(text)
}

fn commons_assets(raw: &Value) -> Result<Vec<Asset>, String> {
    if raw.get("error").is_some() {
        return Err("Commons could not return file metadata. It may be busy or the page may be unavailable; retry later.".into());
    }
    let pages = raw["query"]["pages"]
        .as_array()
        .ok_or("Commons did not return file pages.")?;
    let mut assets = vec![];
    for page in pages.iter().take(5) {
        if page.get("missing").is_some() || page.get("invalid").is_some() {
            continue;
        }
        let Some(title) = page["title"]
            .as_str()
            .filter(|title| title.starts_with("File:"))
        else {
            continue;
        };
        let Some(id) = page["pageid"].as_u64() else {
            continue;
        };
        let Some(info) = page["imageinfo"].as_array().and_then(|infos| infos.first()) else {
            continue;
        };
        if info.get("filehidden").is_some() || info.get("suppressed").is_some() {
            continue;
        }
        let download = safe_download_url(
            info["url"]
                .as_str()
                .ok_or("Commons did not return an original file URL.")?,
            Provider::Commons,
        )?;
        let source = source_url(&Target::Commons {
            title: title.into(),
            category: false,
        });
        let author = commons_metadata(info, "Artist", 2000);
        let license = commons_metadata(info, "LicenseShortName", 500);
        let attribution = Some(format!("Title: {}\nSource: {source}\nAuthor supplied by source: {}\nCredit: {}\nLicense supplied by source: {}\nLicense URL supplied by source: {}\nUsage terms: {}\nAttribution required: {}\nMetadata does not constitute a legal assurance.",
            clean_text(title, 1024), author.as_deref().unwrap_or("Not supplied"),
            commons_metadata(info, "Credit", 2000).as_deref().unwrap_or("Not supplied"),
            license.as_deref().unwrap_or("Not supplied; no license inferred"),
            commons_metadata(info, "LicenseUrl", 1000).as_deref().unwrap_or("Not supplied"),
            commons_metadata(info, "UsageTerms", 2000).as_deref().unwrap_or("Not supplied"),
            commons_metadata(info, "AttributionRequired", 100).as_deref().unwrap_or("Not supplied")));
        let original = Asset {
            id: format!("commons:{id}:original"),
            source_url: source.clone(),
            download_url: download,
            filename: filename(title.strip_prefix("File:").unwrap_or(title))
                .ok_or("Invalid Commons filename.")?,
            mime: field(info, "mime", 120).unwrap_or_else(|| "application/octet-stream".into()),
            size: info["size"].as_u64(),
            title: clean_text(title.strip_prefix("File:").unwrap_or(title), 240),
            version: field(info, "timestamp", 64),
            original: true,
            author,
            license,
            attribution,
            sha256: None,
            platform: None,
            architecture: None,
            published_at: None,
            checksum: None,
            audiobook: None,
        };
        if let Some(thumb) = info["thumburl"].as_str() {
            let thumb = safe_download_url(thumb, Provider::Commons)?;
            if thumb != original.download_url
                && info["thumbwidth"]
                    .as_u64()
                    .is_some_and(|width| width <= 1280)
            {
                let mut variant = original.clone();
                variant.id = format!("commons:{id}:1280");
                variant.filename = filename(&percent_decode(
                    url::Url::parse(&thumb)
                        .map_err(|_| "Invalid Commons variant URL.")?
                        .path(),
                )?)
                .ok_or("Invalid Commons variant filename.")?;
                variant.download_url = thumb;
                variant.mime =
                    field(info, "thumbmime", 120).unwrap_or_else(|| original.mime.clone());
                variant.size = None;
                variant.title = format!(
                    "{} · {} px",
                    original.title,
                    info["thumbwidth"].as_u64().unwrap_or(1280)
                );
                variant.original = false;
                assets.push(variant);
            }
        }
        assets.push(original);
    }
    Ok(assets)
}

async fn commons(target: &Target, cursor: Option<&str>) -> Result<Page, String> {
    let Target::Commons { title, category } = target else {
        return Err("Invalid Commons source.".into());
    };
    let mut result = page(target, Provider::Commons, clean_text(title, 240));
    let pageids = if *category {
        let (number, token) = match cursor {
            None => (1, None),
            Some(cursor) if cursor.len() <= 2048 && !cursor.chars().any(char::is_control) => {
                let (number, token) = cursor
                    .split_once(':')
                    .ok_or("Invalid Commons category cursor.")?;
                let number = numeric_page(Some(number))?;
                if number > 20 || token.is_empty() {
                    return Err(
                        "The Commons category exceeds its 200-file exploration limit.".into(),
                    );
                }
                (number, Some(token))
            }
            _ => return Err("Invalid Commons category cursor.".into()),
        };
        let mut api = commons_api();
        api.query_pairs_mut()
            .append_pair("list", "categorymembers")
            .append_pair("cmtitle", title)
            .append_pair("cmtype", "file")
            .append_pair("cmlimit", "10")
            .append_pair("cmprop", "ids|title");
        if let Some(token) = token {
            api.query_pairs_mut().append_pair("cmcontinue", token);
        }
        let raw = api_json(api.as_str()).await?;
        let members = raw["query"]["categorymembers"]
            .as_array()
            .ok_or("Commons did not return category members.")?;
        if let Some(token) = raw["continue"]["cmcontinue"].as_str() {
            if number < 20 {
                result.next_cursor = Some(format!("{}:{token}", number + 1));
            } else {
                result.warnings.push("Exploration stopped at 200 direct category files. This is only part of the category; subcategories are not traversed.".into());
            }
        }
        members
            .iter()
            .take(10)
            .filter_map(|member| member["pageid"].as_u64())
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
    } else {
        if cursor.is_some() {
            return Err("A Commons file page does not have another results page.".into());
        }
        vec![]
    };
    let mut requests = if *category {
        pageids
            .chunks(5)
            .map(|ids| ("pageids", ids.join("|")))
            .collect::<Vec<_>>()
    } else {
        vec![("titles", title.clone())]
    };
    for (key, value) in requests.drain(..) {
        let mut api = commons_api();
        api.query_pairs_mut()
            .append_pair(key, &value)
            .append_pair("prop", "imageinfo")
            .append_pair("iiprop", "url|size|mime|sha1|timestamp|user|extmetadata")
            .append_pair(
                "iiextmetadatafilter",
                "Artist|Credit|LicenseShortName|LicenseUrl|UsageTerms|AttributionRequired",
            )
            .append_pair("iiextmetadatalanguage", "en")
            .append_pair("iiurlwidth", "1280");
        result
            .assets
            .extend(commons_assets(&api_json(api.as_str()).await?)?);
    }
    result.warnings.push("Originals and available 1280-pixel variants are separate choices. Attribution is saved with each result; licenses may differ and missing metadata implies no legal status.".into());
    if !category && result.assets.is_empty() {
        return Err("This Commons file has no publicly available original.".into());
    }
    Ok(result)
}

pub async fn inspect(
    input: &str,
    cursor: Option<&str>,
    cancel: watch::Receiver<bool>,
) -> Result<Page, String> {
    let target = target(input)?;
    let operation = async {
        match &target {
            Target::Arxiv(value) => arxiv::inspect(value, cursor).await,
            Target::Zenodo(value) => zenodo::inspect(value, cursor).await,
            Target::Librivox(value) => librivox::inspect(value, cursor).await,
            Target::Github { .. } => github(&target, cursor).await,
            Target::Archive { .. } => archive(&target, cursor).await,
            Target::Commons { .. } => commons(&target, cursor).await,
        }
    };
    tokio::select! { biased; _ = cancelled(cancel) => Err(crate::engine::CANCELLED.into()), result = operation => result }
}

pub async fn resolve(
    input: &str,
    asset_id: &str,
    cancel: watch::Receiver<bool>,
) -> Result<Asset, String> {
    if asset_id.len() > 2200 || asset_id.chars().any(char::is_control) {
        return Err("Invalid public file identity.".into());
    }
    let target = target(input)?;
    let operation = async {
        match &target {
            Target::Arxiv(value) => arxiv::resolve(value, asset_id).await,
            Target::Zenodo(value) => zenodo::resolve(value, asset_id).await,
            Target::Librivox(value) => librivox::resolve(value, asset_id).await,
            Target::Github {
                owner,
                repo,
                tag: Some(tag),
            } => {
                let id = asset_id
                    .strip_prefix("github:")
                    .filter(|id| {
                        !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
                    })
                    .ok_or("Invalid release asset identity.")?;
                // The immutable asset ID and its official download path bind it to the selected version.
                let raw = api_json(&url_path(
                    "https://api.github.com",
                    &["repos", owner, repo, "releases", "assets", id],
                ))
                .await?;
                let asset = github_asset(&raw, &target, tag)?;
                if asset.id != asset_id
                    || raw["state"]
                        .as_str()
                        .is_some_and(|state| state != "uploaded")
                {
                    return Err("The selected release asset is no longer available.".into());
                }
                Ok(asset)
            }
            Target::Archive { identifier } => {
                let name = asset_id
                    .strip_prefix("archive:")
                    .ok_or("Invalid Archive file identity.")?;
                let raw =
                    api_json(&url_path("https://archive.org", &["metadata", identifier])).await?;
                if archive_restricted(&raw) {
                    return Err(
                        "The Archive item is now restricted or requires lending. Download stopped."
                            .into(),
                    );
                }
                let files = raw["files"]
                    .as_array()
                    .ok_or("The Archive item has no public file list.")?;
                let file = files
                    .iter()
                    .find(|file| file["name"].as_str() == Some(name))
                    .ok_or("The selected Archive file is no longer present.")?;
                archive_asset(file, &raw["metadata"], &target)?.ok_or_else(|| {
                    "This Archive file is restricted or no longer downloadable.".into()
                })
            }
            Target::Commons {
                category: false, ..
            } => commons(&target, None)
                .await?
                .assets
                .into_iter()
                .find(|asset| asset.id == asset_id)
                .ok_or_else(|| {
                    "The selected Commons file or variant is no longer available.".into()
                }),
            _ => Err("Choose a specific release or file before preparing a download.".into()),
        }
    };
    tokio::select! { biased; _ = cancelled(cancel) => Err(crate::engine::CANCELLED.into()), result = operation => result }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_sources_and_traversal_validation() {
        let target = target("https://github.com/owner/repo/releases/tag/v1.2%2Brc").unwrap();
        assert_eq!(
            source_url(&target),
            "https://github.com/owner/repo/releases/tag/v1.2+rc"
        );
        assert!(supported("https://archive.org/details/opensource_movies"));
        assert!(supported(
            "https://commons.wikimedia.org/wiki/File:Albert_Einstein.jpg"
        ));
        for input in [
            "https://github.com.evil.com/a/b",
            "https://github.com:8443/a/b",
            "https://archive.org/details/a%2Fb",
            "https://commons.wikimedia.org/wiki/Special:Random",
            "https://commons.wikimedia.org/wiki/File:A%7CB.jpg",
        ] {
            assert!(!supported(input), "{input}");
        }
    }

    #[test]
    fn releases_are_versioned_and_prereleases_are_distinct() {
        let target = target("https://github.com/example/tool").unwrap();
        let result = github_releases(&json!([
            {"id": 1, "tag_name":"v1", "name":"Stable", "prerelease":false, "published_at":"2026-10-01T00:00:00Z"},
            {"id": 2, "tag_name":"v2-rc", "prerelease":true},
            {"id": 3, "tag_name":"draft", "draft":true}
        ]), &target, 1).unwrap();
        assert_eq!(result.entries.len(), 2);
        assert_eq!(
            result.entries[0].url,
            "https://github.com/example/tool/releases/tag/v1"
        );
        assert!(!result.entries[0].prerelease);
        assert!(result.entries[1].prerelease);
    }

    #[test]
    fn release_assets_bind_to_repo_tag_and_validate_digest() {
        let target = target("https://github.com/example/tool/releases/tag/v1").unwrap();
        let mut value = json!({"id":42,"name":"tool-linux-x86_64.zip","size":1234,"content_type":"application/zip","browser_download_url":"https://github.com/example/tool/releases/download/v1/tool-linux-x86_64.zip","digest":format!("sha256:{}", "a".repeat(64))});
        let asset = github_asset(&value, &target, "v1").unwrap();
        assert_eq!(asset.id, "github:42");
        assert_eq!(asset.platform.as_deref(), Some("Linux"));
        assert_eq!(asset.architecture.as_deref(), Some("x64"));
        assert_eq!(
            asset.sha256.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        value["browser_download_url"] =
            json!("https://github.com/other/tool/releases/download/v1/tool-linux-x86_64.zip");
        assert!(github_asset(&value, &target, "v1").is_err());
        assert_eq!(
            inferred_target("tool-linux-windows-arm64-amd64.zip"),
            (None, None)
        );
    }

    #[test]
    fn archive_preserves_rights_and_excludes_private_drm_and_paths() {
        let target = target("https://archive.org/details/example").unwrap();
        let mut raw = json!({"metadata":{"title":"Example", "rights":"Rights reserved", "creator":["Author"], "source":"Original repository"}, "files":[
            {"name":"original.pdf","source":"original","size":"123"},
            {"name":"derived.mp3","source":"derivative","size":"456"},
            {"name":"secret.pdf","private":"true"},
            {"name":"book_encrypted.pdf"},
            {"name":"../escape.pdf"},
            {"name":"unsafe\\name.pdf"}
        ]});
        let result = archive_files(&raw, &target, 1).unwrap();
        assert_eq!(result.assets.len(), 2);
        assert_eq!(result.rights.as_deref(), Some("Rights reserved"));
        assert!(result.assets[0].original);
        assert!(!result.assets[1].original);
        assert_eq!(result.assets[0].size, Some(123));
        assert!(result.assets[0]
            .attribution
            .as_deref()
            .unwrap()
            .contains("Original repository"));
        raw["metadata"]["access-restricted-item"] = json!("true");
        assert!(archive_files(&raw, &target, 1).is_err());
    }

    #[test]
    fn source_pagination_is_bounded_and_reports_partial_results() {
        let target = target("https://archive.org/details/example").unwrap();
        let files: Vec<_> = (0..51)
            .map(|index| json!({"name":format!("{index}.txt"),"source":"original"}))
            .collect();
        let raw = json!({"metadata":{"title":"Example"}, "files":files});
        let first = archive_files(&raw, &target, 1).unwrap();
        let second = archive_files(&raw, &target, 2).unwrap();
        assert_eq!(first.assets.len(), 50);
        assert_eq!(first.next_cursor.as_deref(), Some("2"));
        assert_eq!(second.assets.len(), 1);
        assert_eq!(second.assets[0].id, "archive:50.txt");
        assert!(numeric_page(Some("101")).is_err());
        assert!(numeric_page(Some("../1")).is_err());
    }

    #[test]
    fn commons_metadata_is_plain_text_and_variants_keep_attribution() {
        let raw = json!({"query":{"pages":[{"pageid":7,"title":"File:Example.svg","imageinfo":[{
            "url":"https://upload.wikimedia.org/wikipedia/commons/a/ab/Example.svg", "size":123,"mime":"image/svg+xml", "timestamp":"2026-01-01T00:00:00Z",
            "thumburl":"https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/Example.svg/1280px-Example.svg.png", "thumbwidth":1280,"thumbmime":"image/png",
            "extmetadata":{"Artist":{"value":"<b>A &amp; B</b><script>alert(1)</script>"},"LicenseShortName":{"value":"CC BY-SA 4.0"},"LicenseUrl":{"value":"https://creativecommons.org/licenses/by-sa/4.0/"}}
        }]}]}});
        let assets = commons_assets(&raw).unwrap();
        assert_eq!(assets.len(), 2);
        let original = assets.iter().find(|asset| asset.original).unwrap();
        let variant = assets.iter().find(|asset| !asset.original).unwrap();
        assert_eq!(original.author.as_deref(), Some("A & B"));
        assert_eq!(original.license.as_deref(), Some("CC BY-SA 4.0"));
        assert_eq!(variant.mime, "image/png");
        assert_eq!(variant.filename, "1280px-Example.svg.png");
        assert_eq!(original.attribution, variant.attribution);
        assert!(!original.attribution.as_deref().unwrap().contains("alert"));
    }

    #[test]
    fn untrusted_metadata_never_becomes_html_and_no_license_is_invented() {
        assert_eq!(
            clean_text(
                "<style>hide</style><i title='>'>A</i> &#x1F600; &lt;b&gt;",
                200
            ),
            "A 😀 <b>"
        );
        assert_eq!(clean_text("Before<script>never", 200), "Before");
        assert!(safe_download_url(
            "https://upload.wikimedia.org/a.png?token=secret",
            Provider::Commons
        )
        .is_err());
        assert_eq!(safe_download_url("https://upload.wikimedia.org/a.png?utm_source=commons.wikimedia.org&utm_content=original", Provider::Commons).unwrap(), "https://upload.wikimedia.org/a.png");
        assert!(safe_download_url("https://evil.example/a.png", Provider::Commons).is_err());
        let target = target("https://archive.org/details/example").unwrap();
        let asset = archive_asset(
            &json!({"name":"a.pdf","source":"original"}),
            &json!({}),
            &target,
        )
        .unwrap()
        .unwrap();
        assert!(asset.license.is_none());
        assert!(asset.attribution.unwrap().contains("no license inferred"));
    }

    #[tokio::test]
    async fn cancellation_does_not_make_a_network_request() {
        let (_, cancel) = watch::channel(true);
        assert_eq!(
            inspect("https://github.com/example/tool", None, cancel)
                .await
                .unwrap_err(),
            crate::engine::CANCELLED
        );
    }

    #[tokio::test]
    #[ignore = "Requires live public APIs; run explicitly, not as evidence from fixtures"]
    async fn live_public_sources() {
        let (_sender, cancel) = watch::channel(false);
        let releases = inspect("https://github.com/cli/cli", None, cancel.clone())
            .await
            .unwrap();
        let release = releases.entries.first().expect("public releases");
        let files = inspect(&release.url, None, cancel.clone()).await.unwrap();
        assert!(!files.assets.is_empty());
        let file = &files.assets[0];
        assert_eq!(
            resolve(&file.source_url, &file.id, cancel.clone())
                .await
                .unwrap()
                .id,
            file.id
        );
        let commons = inspect(
            "https://commons.wikimedia.org/wiki/File:Example.jpg",
            None,
            cancel.clone(),
        )
        .await
        .unwrap();
        assert!(commons.assets.iter().any(|asset| asset.original));
        let archive = inspect("https://archive.org/details/BigBuckBunny_328", None, cancel)
            .await
            .unwrap();
        assert!(!archive.assets.is_empty());
    }

    #[tokio::test]
    #[ignore = "Downloads small public samples; requires explicit live network access"]
    async fn live_public_downloads() {
        let (_sender, cancel) = watch::channel(false);
        let releases = inspect("https://github.com/cli/cli", None, cancel.clone())
            .await
            .expect("GitHub release list");
        let release = releases.entries.first().expect("GitHub published release");
        let github = inspect(&release.url, None, cancel.clone())
            .await
            .expect("GitHub release assets");
        let github = github
            .assets
            .into_iter()
            .find(|asset| {
                asset.filename.ends_with("checksums.txt")
                    && asset.size.is_some_and(|size| size > 0 && size < 512 * 1024)
            })
            .expect("small published checksums asset");
        let commons = inspect(
            "https://commons.wikimedia.org/wiki/File:Example.jpg",
            None,
            cancel.clone(),
        )
        .await
        .expect("Commons file metadata");
        let commons = commons
            .assets
            .into_iter()
            .find(|asset| asset.original && asset.size.is_some_and(|size| size < 512 * 1024))
            .expect("small Commons original");
        let archive = inspect(
            "https://archive.org/details/BigBuckBunny_328",
            None,
            cancel.clone(),
        )
        .await
        .expect("Archive item metadata");
        let archive = archive
            .assets
            .into_iter()
            .find(|asset| {
                asset.filename.ends_with("thumb.jpg")
                    && asset.size.is_some_and(|size| size > 0 && size < 512 * 1024)
            })
            .expect("small Archive thumbnail");
        for (index, chosen) in [github, commons, archive].into_iter().enumerate() {
            let asset = resolve(&chosen.source_url, &chosen.id, cancel.clone())
                .await
                .expect("fresh public identity resolution");
            let staging = std::env::temp_dir().join(format!(
                "nabnab-public-live-{}-{}-{index}",
                std::process::id(),
                crate::queue::now()
            ));
            std::fs::create_dir(&staging).expect("unique test staging directory");
            let path = crate::direct::download_file(
                &asset.download_url,
                &staging,
                &asset.filename,
                cancel.clone(),
                index as u64 + 1,
                |_| {},
            )
            .await
            .expect("public direct file download");
            let item = crate::library::index(&path, None, Some(asset.source_url.clone()), None)
                .or_else(|_| crate::library::index_file(&path))
                .expect("index the actual downloaded file");
            assert!(item.size > 0 && item.size < 512 * 1024);
            assert_eq!(item.size, asset.size.unwrap());
            assert_eq!(
                item.kind,
                if asset.mime.starts_with("image/") {
                    "image"
                } else {
                    "document"
                }
            );
            if let Some(expected) = asset.sha256 {
                use sha2::{Digest, Sha256};
                let actual = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
                assert_eq!(actual, expected);
            }
            eprintln!(
                "Downloaded and indexed {}: {} bytes, type {}",
                asset.filename, item.size, item.kind
            );
            std::fs::remove_dir_all(&staging).expect("remove owned test staging");
        }
    }
}
