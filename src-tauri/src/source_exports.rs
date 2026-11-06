use crate::{engine, model::Progress, providers::ProviderId, AppState};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::AppHandle;
use tokio::sync::watch;

const LIMIT: usize = 200;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: String,
    author: String,
    text: String,
    url: String,
    parent_id: Option<String>,
    date: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Archive {
    provider: String,
    source: String,
    title: String,
    body: String,
    entries: Vec<Entry>,
    limit: usize,
    complete: bool,
    expected: Option<u64>,
    missing_reported: u64,
    warnings: Vec<String>,
}

fn text(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .take(max)
        .collect()
}
fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

fn reddit_source(input: &str) -> Result<(String, String), String> {
    let normalized = crate::model::validate_url(input)?;
    let parsed = url::Url::parse(&normalized).map_err(|_| "Use a Reddit post URL.")?;
    let host = parsed.host_str().unwrap_or_default();
    let parts: Vec<_> = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|v| !v.is_empty())
        .collect();
    let post = if host == "redd.it" {
        parts.first().copied()
    } else if host == "reddit.com" || host.ends_with(".reddit.com") {
        parts
            .iter()
            .position(|part| *part == "comments")
            .and_then(|index| parts.get(index + 1).copied())
    } else {
        None
    }
    .ok_or("Use a Reddit post permalink, not a community or media CDN URL.")?;
    let post = post.trim_end_matches(".json");
    if !id(post) || !post.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("Invalid Reddit post identifier.".into());
    }
    Ok((
        format!("https://www.reddit.com/comments/{post}/"),
        post.into(),
    ))
}

fn twitter_source(input: &str) -> Result<(String, String), String> {
    if crate::providers::identify(input)?.id != ProviderId::Twitter {
        return Err("Use a direct X or Twitter status URL. Mirrors and extraction intermediaries are not supported.".into());
    }
    let parsed = url::Url::parse(input).map_err(|_| "Use a direct X status URL.")?;
    let parts: Vec<_> = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|v| !v.is_empty())
        .collect();
    let (name, post) = match parts.as_slice() {
        [name, "status", post, ..] => (*name, *post),
        ["i", "web", "status", post, ..] => ("i/web", *post),
        _ => return Err("Open an accessible X status to export its conversation.".into()),
    };
    if !post.bytes().all(|b| b.is_ascii_digit())
        || !id(post)
        || name != "i/web"
            && (!id(name) || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    {
        return Err("Invalid X status identifier.".into());
    }
    Ok((format!("https://x.com/{name}/status/{post}"), post.into()))
}

pub fn validate(input: &str, operation: &str, format: &str) -> Result<(), String> {
    match operation {
        "bilibiliDanmaku"=>{if format!="xml"{return Err("Danmaku exports use XML.".into());}crate::danmaku::validate(input)?;},
        "youtubeThumbnail" | "youtubeDescription" | "youtubeMetadata" | "youtubeChapters"
        | "youtubePlaylist" => {
            if crate::providers::identify(input)?.id != ProviderId::Youtube {
                return Err("Choose a YouTube video or playlist URL for this export.".into());
            }
            let valid = match operation {
                "youtubeThumbnail" => format == "original",
                "youtubeDescription" => format == "txt",
                "youtubeChapters" => matches!(format, "json" | "md"),
                _ => format == "json",
            };
            if !valid {
                return Err("Choose the supported format for this YouTube export.".into());
            }
        }
        "redditPost" => {
            reddit_source(input)?;
            if !matches!(format, "json" | "md" | "html") {
                return Err("Choose JSON, Markdown or static HTML.".into());
            }
        }
        "twitterThread" => {
            twitter_source(input)?;
            if !matches!(format, "json" | "md" | "html") {
                return Err("Choose JSON, Markdown or static HTML.".into());
            }
        }
        _ => return Err("Choose an implemented provider export operation.".into()),
    }
    Ok(())
}

fn metadata_args(root: &Path, url: &str, playlist: bool) -> Vec<String> {
    let mut args = engine::base_args(root);
    if playlist {
        args.retain(|arg| arg != "--no-playlist");
        args.extend([
            "--yes-playlist".into(),
            "--flat-playlist".into(),
            "--playlist-end".into(),
            (LIMIT + 1).to_string(),
        ]);
    }
    args.extend([
        "--dump-single-json".into(),
        "--skip-download".into(),
        "--".into(),
        url.into(),
    ]);
    args
}

fn public_youtube(value: &Value) -> Value {
    let mut output = serde_json::Map::new();
    for key in [
        "id",
        "title",
        "description",
        "uploader",
        "uploader_id",
        "channel",
        "channel_id",
        "upload_date",
        "duration",
        "timestamp",
        "license",
        "availability",
        "tags",
        "categories",
    ] {
        if let Some(value) = value.get(key).filter(|value| !value.is_null()) {
            output.insert(key.into(), value.clone());
        }
    }
    output.insert(
        "chapters".into(),
        serde_json::to_value(crate::model::chapters_from_value(value))
            .unwrap_or(Value::Array(Vec::new())),
    );
    if let Some(id) = value["id"].as_str().filter(|value| id(value)) {
        output.insert(
            "source".into(),
            Value::String(format!("https://www.youtube.com/watch?v={id}")),
        );
    }
    Value::Object(output)
}

fn playlist_metadata(value: &Value) -> Result<Value, String> {
    let entries = value["entries"]
        .as_array()
        .ok_or("This YouTube URL did not return an enumerable playlist.")?;
    let items: Vec<_> = entries
        .iter()
        .take(LIMIT)
        .enumerate()
        .map(|(index, entry)| json!({"position":index+1,"metadata":public_youtube(entry)}))
        .collect();
    if items.is_empty() {
        return Err("The playlist returned no accessible entries to export.".into());
    }
    let expected = value["playlist_count"].as_u64();
    let truncated =
        entries.len() > LIMIT || expected.is_some_and(|count| count > items.len() as u64);
    Ok(
        json!({"kind":"youtubePlaylist","id":value["id"],"title":value["title"],"uploader":value["uploader"],"entries":items,"limit":LIMIT,"expected":expected,"truncated":truncated,"complete":expected.map(|count| !truncated && count == items.len() as u64),"scope":"Metadata only. Unavailable or omitted entries are not reconstructed."}),
    )
}

fn cookie_header(
    file: &crate::sessions::CookieFile,
    host: &str,
    path: &str,
) -> Result<String, String> {
    let bytes =
        fs::read(&file.path).map_err(|_| "Could not read the authorized Reddit session.")?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("The authorized session exceeds its size limit.".into());
    }
    let content = std::str::from_utf8(&bytes).map_err(|_| "The authorized session is invalid.")?;
    let mut cookies = Vec::new();
    for line in content.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 7 {
            continue;
        }
        let domain = fields[0].trim_start_matches('.');
        if !(domain == host || fields[1] == "TRUE" && host.ends_with(&format!(".{domain}"))) {
            continue;
        }
        if !(path == fields[2]
            || path.starts_with(fields[2])
                && (fields[2].ends_with('/')
                    || path.as_bytes().get(fields[2].len()) == Some(&b'/')))
        {
            continue;
        }
        if fields[4]
            .parse::<u64>()
            .ok()
            .is_some_and(|expires| expires != 0 && expires < crate::queue::now() / 1000)
        {
            continue;
        }
        if fields[5].is_empty()
            || fields[5]
                .bytes()
                .any(|b| b.is_ascii_control() || b";= ".contains(&b))
            || fields[6].bytes().any(|b| b.is_ascii_control() || b == b';')
        {
            return Err("The authorized Reddit session contains invalid cookie values.".into());
        }
        cookies.push(format!("{}={}", fields[5], fields[6]));
    }
    Ok(cookies.join("; "))
}

async fn reddit_json(
    url: &str,
    cookies: Option<&crate::sessions::CookieFile>,
    mut cancel: watch::Receiver<bool>,
) -> Result<Value, String> {
    let operation = async {
        let url = url::Url::parse(url).map_err(|_| "Invalid Reddit archive URL.")?;
        if url.scheme() != "https"
            || url.host_str() != Some("www.reddit.com")
            || url.port_or_known_default() != Some(443)
        {
            return Err("Reddit authentication cannot be forwarded to another origin.".into());
        }
        let addresses: Vec<_> = tokio::time::timeout(
            Duration::from_secs(8),
            tokio::net::lookup_host(("www.reddit.com", 443)),
        )
        .await
        .map_err(|_| "Reddit hostname lookup timed out.")?
        .map_err(|_| "Could not resolve Reddit.")?
        .collect();
        if addresses.is_empty()
            || addresses
                .iter()
                .any(|address| !crate::providers::is_public_ip(address.ip()))
        {
            return Err("Reddit resolved to a reserved network destination.".into());
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs("www.reddit.com", &addresses)
            .timeout(Duration::from_secs(30))
            .user_agent("windows:nabnab:0.2 (bounded post export)")
            .build()
            .map_err(|_| "Could not prepare the Reddit archive request.")?;
        let mut request = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity");
        if let Some(cookies) = cookies {
            let header = cookie_header(cookies, "www.reddit.com", url.path())?;
            if !header.is_empty() {
                request = request.header(reqwest::header::COOKIE, header);
            }
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| "The Reddit archive request failed.")?;
        match response.status().as_u16() { 401|403 => return Err("Reddit denied this post archive. Refresh the authorized Reddit session or check API access; private and unavailable comments are not reconstructed.".into()),429 => return Err("Reddit returned HTTP 429. Wait before exporting this post again.".into()),_ if !response.status().is_success() => return Err(format!("Reddit returned HTTP {} for the post archive.",response.status().as_u16())),_=>{} }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The Reddit archive response was interrupted.")?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
                return Err("The Reddit archive response exceeds 8 MiB.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| "Reddit did not return readable post/comment JSON.".into())
    };
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    tokio::select! { result=operation=>result,_=cancel.changed()=>Err(engine::CANCELLED.into()) }
}

fn reddit_archive(value: &Value, source: &str) -> Result<Archive, String> {
    let post = value[0]["data"]["children"][0]
        .get("data")
        .filter(|post| post["id"].as_str().is_some_and(id))
        .ok_or("Reddit returned no accessible post data.")?;
    let mut entries = Vec::new();
    let mut missing = 0u64;
    let mut truncated = false;
    fn walk(
        value: &Value,
        depth: usize,
        entries: &mut Vec<Entry>,
        missing: &mut u64,
        truncated: &mut bool,
        source: &str,
    ) {
        if depth > 16 {
            *truncated = true;
            return;
        }
        for child in value["data"]["children"].as_array().into_iter().flatten() {
            if child["kind"] == "more" {
                *missing = missing.saturating_add(child["data"]["count"].as_u64().unwrap_or(1));
                continue;
            }
            if child["kind"] != "t1" {
                continue;
            }
            if entries.len() >= LIMIT {
                *truncated = true;
                continue;
            }
            let data = &child["data"];
            let Some(comment) = data["id"].as_str().filter(|value| id(value)) else {
                *truncated = true;
                continue;
            };
            let permalink = data["permalink"]
                .as_str()
                .and_then(|url| {
                    url::Url::parse("https://www.reddit.com")
                        .ok()?
                        .join(url)
                        .ok()
                })
                .filter(|url| {
                    url.scheme() == "https"
                        && url.host_str() == Some("www.reddit.com")
                        && url.port_or_known_default() == Some(443)
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                })
                .map(|url| url.to_string())
                .unwrap_or_else(|| source.to_owned());
            entries.push(Entry {
                id: comment.into(),
                author: text(data["author"].as_str().unwrap_or("[unavailable]"), 256),
                text: text(
                    data["body"].as_str().unwrap_or("[Text unavailable]"),
                    1_000_000,
                ),
                url: permalink,
                parent_id: data["parent_id"].as_str().map(|v| text(v, 128)),
                date: data["created_utc"].as_f64().map(|v| v.to_string()),
            });
            walk(
                &data["replies"],
                depth + 1,
                entries,
                missing,
                truncated,
                source,
            );
        }
    }
    walk(
        &value[1],
        0,
        &mut entries,
        &mut missing,
        &mut truncated,
        source,
    );
    let expected = post["num_comments"].as_u64();
    let mut warnings=vec!["Only the comments returned by this bounded request are included. Deleted, restricted and unloaded comments may be absent.".into()];
    if truncated {
        warnings.push("The 200-comment or recursion limit was reached.".into());
    }
    if missing > 0 {
        warnings.push(format!(
            "Reddit reported {missing} additional comments that were not fetched."
        ));
    }
    Ok(Archive {
        provider: "Reddit".into(),
        source: source.into(),
        title: text(post["title"].as_str().unwrap_or("Reddit post"), 2048),
        body: text(post["selftext"].as_str().unwrap_or_default(), 2_000_000),
        entries,
        limit: LIMIT,
        complete: false,
        expected,
        missing_reported: missing,
        warnings,
    })
}

fn thread_archive(raw: &str, source: &str) -> Result<Archive, String> {
    let messages: Vec<Value> = serde_json::from_str(raw)
        .map_err(|_| "The X conversation engine returned unreadable metadata.")?;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut warnings=vec!["Accessible conversation only. Restricted, deleted or unreturned posts may be missing; this is not a certified complete thread.".into()];
    for message in messages {
        if message[0] == -1 {
            warnings.push(crate::failure::sanitize(
                message[1]["message"]
                    .as_str()
                    .unwrap_or("The engine reported an extraction error."),
            ));
            continue;
        }
        if message[0] != 2 {
            continue;
        }
        let data = &message[1];
        let post = data["tweet_id"]
            .as_u64()
            .map(|v| v.to_string())
            .or_else(|| data["tweet_id"].as_str().map(str::to_owned))
            .filter(|v| id(v));
        let Some(post) = post else {
            continue;
        };
        if !seen.insert(post.clone()) {
            continue;
        }
        let author = data["author"]["name"]
            .as_str()
            .or_else(|| data["user"]["name"].as_str())
            .filter(|value| id(value))
            .unwrap_or("i/web");
        entries.push(Entry {
            id: post.clone(),
            author: author.into(),
            text: text(data["content"].as_str().unwrap_or_default(), 2_000_000),
            url: format!("https://x.com/{author}/status/{post}"),
            parent_id: data["reply_id"]
                .as_u64()
                .filter(|v| *v > 0)
                .map(|v| v.to_string()),
            date: data["date"].as_str().map(|v| text(v, 128)),
        });
    }
    if entries.is_empty() {
        return Err("X returned no accessible conversation posts. The gallery-dl conversation endpoint and authorized X session require verification; no mirrors or fictitious thread were used.".into());
    }
    entries.sort_by_key(|entry| entry.id.parse::<u64>().unwrap_or(0));
    if entries.len() > LIMIT {
        entries.truncate(LIMIT);
        warnings.push("The export reached its 200-post limit.".into());
    }
    Ok(Archive {
        provider: "X / Twitter".into(),
        source: source.into(),
        title: "Accessible X conversation".into(),
        body: String::new(),
        entries,
        limit: LIMIT,
        complete: false,
        expected: None,
        missing_reported: 0,
        warnings,
    })
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn markdown(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if "\\`*_{}[]<>()!#+-.|".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
fn render_archive(archive: &Archive, format: &str) -> Result<Vec<u8>, String> {
    match format {
        "json" => serde_json::to_vec_pretty(archive)
            .map_err(|_| "Could not encode the source archive.".into()),
        "md" => {
            let mut out = format!(
                "# {}\n\n{}\n\n{}\n",
                markdown(&archive.title),
                archive.source,
                markdown(&archive.body)
            );
            for warning in &archive.warnings {
                out.push_str(&format!("\n> {}\n", markdown(warning)));
            }
            for entry in &archive.entries {
                out.push_str(&format!(
                    "\n## {} · {}\n\n{}\n\n{}\n",
                    markdown(&entry.author),
                    markdown(&entry.id),
                    entry.url,
                    markdown(&entry.text)
                ));
            }
            Ok(out.into_bytes())
        }
        "html" => {
            let mut out=format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>{}</title><style>body{{max-width:900px;margin:32px auto;padding:0 20px;font:16px/1.6 system-ui}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;font:inherit}}article{{border-top:1px solid #888;padding:16px 0}}a{{color:inherit}}</style><h1>{}</h1><p><a href=\"{}\" rel=\"noopener noreferrer\">Source</a></p><pre>{}</pre>",html(&archive.title),html(&archive.title),html(&archive.source),html(&archive.body));
            for warning in &archive.warnings {
                out.push_str(&format!("<p>{}</p>", html(warning)));
            }
            for entry in &archive.entries {
                out.push_str(&format!("<article><h2>{} · {}</h2><p><a href=\"{}\" rel=\"noopener noreferrer\">Original post</a></p><pre>{}</pre></article>",html(&entry.author),html(&entry.id),html(&entry.url),html(&entry.text)));
            }
            out.push_str("</html>");
            Ok(out.into_bytes())
        }
        _ => Err("Choose JSON, Markdown or static HTML.".into()),
    }
}

fn write_output(staging: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let staging = crate::library::authorized_path(staging)?;
    let output = staging.join(name);
    if output.exists() {
        let safe = crate::library::authorized_path(&output)?;
        if safe.parent() != Some(staging.as_path()) {
            return Err("The export output escaped its job folder.".into());
        }
        if fs::metadata(&safe).map_err(|e| e.to_string())?.len() == bytes.len() as u64
            && fs::read(&safe).map_err(|e| e.to_string())? == bytes
        {
            return Ok(output);
        }
        return Err(
            "An earlier export already exists in this job. Start a new job to keep both versions."
                .into(),
        );
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&output)
        .map_err(|_| "Could not create the source export.")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save the source export.")?;
    Ok(output)
}

pub async fn execute(
    app: &AppHandle,
    state: &Arc<AppState>,
    root: &Path,
    url: &str,
    operation: &str,
    format: &str,
    staging: &Path,
    mut cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Clone + Send + Sync + 'static,
) -> Result<PathBuf, String> {
    validate(url, operation, format)?;
    if operation=="bilibiliDanmaku"{return crate::danmaku::execute(app,state,root,url,staging,cancel,run_id,emit).await;}
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    let config = crate::config_root(state)?;
    if operation == "redditPost" {
        let (source, id) = reddit_source(url)?;
        let cookies = crate::sessions::materialize(&config, &source)?;
        let value = reddit_json(
            &format!("https://www.reddit.com/comments/{id}.json?limit={LIMIT}&depth=8&raw_json=1"),
            cookies.as_ref(),
            cancel.clone(),
        )
        .await?;
        let archive = reddit_archive(&value, &source)?;
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        return write_output(
            staging,
            &format!("reddit-{id}.{format}"),
            &render_archive(&archive, format)?,
        );
    }
    if operation == "twitterThread" {
        let (source, id) = twitter_source(url)?;
        let executable = crate::components::executable(app, "gallery")?;
        let cookies = crate::sessions::materialize(&config, &source)?;
        let mut args: Vec<String> = [
            "--ignore-config",
            "--no-input",
            "--no-postprocessors",
            "-o",
            "cache.file=null",
            "--dump-json",
            "--post-range",
            "1-201",
            "-o",
            "extractor.twitter.conversations=accessible",
            "-o",
            "extractor.twitter.text-tweets=true",
            "-o",
            "extractor.twitter.replies=true",
            "-o",
            "extractor.twitter.quoted=false",
            "-o",
            "extractor.twitter.retries=2",
            "-o",
            "extractor.twitter.timeout=20",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if let Some(cookies) = &cookies {
            args.extend([
                "--cookies".into(),
                cookies.path.to_string_lossy().into_owned(),
            ]);
        }
        args.extend(["--".into(), source.clone()]);
        let raw = engine::run_process(executable, args, cancel.clone(), run_id, true, emit).await?;
        let archive = thread_archive(&raw, &source)?;
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        return write_output(
            staging,
            &format!("x-conversation-{id}.{format}"),
            &render_archive(&archive, format)?,
        );
    }
    let url = crate::providers::validate_remote(url).await?;
    let cookies = crate::sessions::materialize(&config, &url)?;
    let mut args = metadata_args(root, &url, operation == "youtubePlaylist");
    if let Some(cookies) = &cookies {
        args.splice(
            0..0,
            [
                "--cookies".into(),
                cookies.path.to_string_lossy().into_owned(),
            ],
        );
    }
    let raw = engine::run_process(
        crate::updater::executable(app, root)?,
        args,
        cancel.clone(),
        run_id,
        true,
        emit,
    )
    .await?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|_| "YouTube returned unreadable metadata.")?;
    let video_id = value["id"]
        .as_str()
        .filter(|value| id(value))
        .unwrap_or("export");
    match operation {
        "youtubeThumbnail" => {
            let thumbnail = value["thumbnail"]
                .as_str()
                .or_else(|| {
                    value["thumbnails"]
                        .as_array()?
                        .iter()
                        .rev()
                        .find_map(|v| v["url"].as_str())
                })
                .ok_or("This video did not return an accessible thumbnail.")?;
            let (mime, bytes) = tokio::select! {value=crate::direct::image_preview(thumbnail)=>value?,_=cancel.changed()=>return Err(engine::CANCELLED.into())};
            let extension = match mime.as_str() {
                "image/jpeg" => "jpg",
                "image/png" => "png",
                "image/webp" => "webp",
                "image/gif" => "gif",
                "image/avif" => "avif",
                _ => return Err("The thumbnail format is not supported.".into()),
            };
            write_output(
                staging,
                &format!("youtube-{video_id}-thumbnail.{extension}"),
                &bytes,
            )
        }
        "youtubeDescription" => {
            let description = value["description"]
                .as_str()
                .filter(|v| !v.trim().is_empty())
                .ok_or("This video has no accessible description text.")?;
            write_output(
                staging,
                &format!("youtube-{video_id}-description.txt"),
                description.as_bytes(),
            )
        }
        "youtubeChapters" => {
            let chapters = crate::model::chapters_from_value(&value);
            if chapters.is_empty() {
                return Err(
                    "This video returned no available chapters. Nothing was exported.".into(),
                );
            }
            let bytes = if format == "md" {
                chapters
                    .iter()
                    .map(|chapter| {
                        format!(
                            "- {:.3}–{:.3} s · {}\n",
                            chapter.start,
                            chapter.end,
                            markdown(&chapter.title)
                        )
                    })
                    .collect::<String>()
                    .into_bytes()
            } else {
                serde_json::to_vec_pretty(&chapters).map_err(|_| "Could not encode chapters.")?
            };
            write_output(
                staging,
                &format!("youtube-{video_id}-chapters.{format}"),
                &bytes,
            )
        }
        "youtubePlaylist" => write_output(
            staging,
            &format!("youtube-{video_id}-playlist.json"),
            &serde_json::to_vec_pretty(&playlist_metadata(&value)?)
                .map_err(|_| "Could not encode playlist metadata.")?,
        ),
        _ => write_output(
            staging,
            &format!("youtube-{video_id}-metadata.json"),
            &serde_json::to_vec_pretty(&public_youtube(&value))
                .map_err(|_| "Could not encode video metadata.")?,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn youtube_exports_do_not_download_videos_or_include_engine_credentials() {
        let args = metadata_args(
            Path::new("engines"),
            "https://youtube.com/watch?v=fixture",
            false,
        );
        assert!(args.contains(&"--skip-download".into()));
        assert!(args.contains(&"--no-playlist".into()));
        let metadata = public_youtube(
            &json!({"id":"video","title":"Title","description":"Text","url":"https://cdn.example.org/a?token=secret","http_headers":{"Cookie":"secret"},"formats":[{"url":"private"}],"chapters":[]}),
        );
        let text = metadata.to_string();
        assert!(text.contains("Text"));
        assert!(!text.contains("secret"));
        assert!(!text.contains("formats"));
        let playlist = playlist_metadata(
            &json!({"id":"list","entries":[{"id":"a","title":"First"}],"playlist_count":3}),
        )
        .unwrap();
        assert_eq!(playlist["entries"][0]["position"], 1);
        assert_eq!(playlist["truncated"], true);
        assert_eq!(playlist["complete"], false);
    }
    #[test]
    fn reddit_archives_actual_returned_comments_and_reports_missing_items() {
        let value = json!([{"data":{"children":[{"data":{"id":"abc","title":"Post","selftext":"Text","num_comments":5}}]}},{"data":{"children":[{"kind":"t1","data":{"id":"c1","body":"A real returned comment","author":"author","parent_id":"t3_abc","permalink":"/r/example/comments/abc/post/c1/","replies":""}},{"kind":"more","data":{"count":4}}]}}]);
        let archive = reddit_archive(&value, "https://www.reddit.com/comments/abc/").unwrap();
        assert_eq!(archive.entries.len(), 1);
        assert_eq!(archive.entries[0].text, "A real returned comment");
        assert_eq!(archive.expected, Some(5));
        assert_eq!(archive.missing_reported, 4);
        assert!(!archive.complete);
    }
    #[test]
    fn x_conversation_includes_text_only_posts_and_surfaces_engine_errors() {
        let raw = r#"[[2,{"tweet_id":12,"author":{"name":"author"},"content":"Second","reply_id":11}],[2,{"tweet_id":11,"author":{"name":"author"},"content":"First"}],[-1,{"error":"AuthenticationError","message":"Some posts were unavailable"}]]"#;
        let archive = thread_archive(raw, "https://x.com/author/status/11").unwrap();
        assert_eq!(archive.entries.len(), 2);
        assert_eq!(archive.entries[0].text, "First");
        assert_eq!(archive.entries[1].parent_id.as_deref(), Some("11"));
        assert_eq!(archive.warnings.len(), 2);
        assert!(!archive.complete);
        assert!(thread_archive(
            r#"[[-1,{"message":"Sign in required"}]]"#,
            "https://x.com/a/status/1"
        )
        .is_err());
        assert!(twitter_source("https://fxtwitter.com/a/status/1").is_err());
    }
    #[test]
    fn html_exports_escape_untrusted_text_and_never_embed_remote_media() {
        let archive = Archive {
            provider: "Reddit".into(),
            source: "https://www.reddit.com/comments/abc/".into(),
            title: "<script>bad()</script>".into(),
            body: "<img src='https://external.example.org/track'>".into(),
            entries: vec![],
            limit: 200,
            complete: false,
            expected: None,
            missing_reported: 0,
            warnings: vec![],
        };
        let output = String::from_utf8(render_archive(&archive, "html").unwrap()).unwrap();
        assert!(output.contains("default-src 'none'"));
        assert!(!output.contains("<script>"));
        assert!(!output.contains("<img"));
        assert!(output.contains("&lt;img"));
    }
}
