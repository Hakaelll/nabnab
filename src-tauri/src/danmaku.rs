use crate::{model::Progress, AppState};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::AppHandle;
use tokio::sync::watch;

const MAX_XML: usize = 32 * 1024 * 1024;
const MAX_COMMENTS: usize = 200_000;

#[derive(Debug, PartialEq, Eq)]
struct Source {
    id: String,
    bvid: bool,
    page: u32,
}
struct Part {
    cid: u64,
    title: String,
}

fn source(input: &str) -> Result<Source, String> {
    let parsed = url::Url::parse(&crate::model::validate_url(input)?)
        .map_err(|_| "Use a complete Bilibili video URL.")?;
    if !matches!(
        parsed.host_str(),
        Some("bilibili.com" | "www.bilibili.com" | "m.bilibili.com")
    ) {
        return Err(
            "Use a direct bilibili.com/video/BV… or /video/av… URL, not a short link or mirror."
                .into(),
        );
    }
    let parts: Vec<_> = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    let id=match parts.as_slice(){["video",id]=>*id,_=>return Err("Danmaku export supports standard Bilibili video pages. Open the selected video's full URL.".into())};
    let bvid = id.starts_with("BV")
        && id.len() == 12
        && id[2..].bytes().all(|byte| byte.is_ascii_alphanumeric());
    let aid = id
        .strip_prefix("av")
        .or_else(|| id.bytes().all(|byte| byte.is_ascii_digit()).then_some(id));
    let id = if bvid {
        id.to_owned()
    } else {
        let aid = aid.ok_or("The Bilibili video identifier is invalid.")?;
        if aid.is_empty()
            || aid.len() > 16
            || !aid.bytes().all(|byte| byte.is_ascii_digit())
            || aid
                .parse::<u64>()
                .ok()
                .is_none_or(|aid| aid == 0 || aid > 9_007_199_254_740_991)
        {
            return Err("The Bilibili video identifier is invalid.".into());
        }
        aid.parse::<u64>()
            .map_err(|_| "The Bilibili video identifier is invalid.")?
            .to_string()
    };
    let mut page = None;
    for (key, value) in parsed.query_pairs() {
        if key == "p" {
            if page.is_some() {
                return Err("Choose exactly one Bilibili part number.".into());
            }
            page = Some(
                value
                    .parse::<u32>()
                    .map_err(|_| "Choose a valid Bilibili part number.")?,
            );
        }
    }
    let page = page.unwrap_or(1);
    if !(1..=1000).contains(&page) {
        return Err("Choose a Bilibili part between 1 and 1000.".into());
    }
    Ok(Source { id, bvid, page })
}
pub fn validate(input: &str) -> Result<(), String> {
    source(input).map(|_| ())
}
fn metadata_url(source: &Source) -> String {
    let mut url =
        url::Url::parse("https://api.bilibili.com/x/web-interface/view").expect("constant API URL");
    url.query_pairs_mut()
        .append_pair(if source.bvid { "bvid" } else { "aid" }, &source.id);
    url.to_string()
}
fn positive_integer(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse::<u64>().ok())
        .filter(|value| *value > 0 && *value <= 9_007_199_254_740_991)
}
fn part(value: &Value, source: &Source) -> Result<Part, String> {
    match value["code"].as_i64(){Some(0)=>{},Some(-101)|Some(-403)=>return Err("Bilibili requires authorized access for this video or denies it. The public danmaku route cannot access it.".into()),Some(-352)|Some(-412)=>return Err("Bilibili restricted this API request. Retry later; no bypass or mirror was used.".into()),Some(code)=>return Err(format!("Bilibili metadata returned code {code}. This video or part may be unavailable.")),None=>return Err("Bilibili returned an invalid metadata response.".into())}
    let data = &value["data"];
    if source.bvid {
        if data["bvid"].as_str() != Some(source.id.as_str()) {
            return Err("Bilibili returned metadata for another video.".into());
        }
    } else if positive_integer(&data["aid"])
        .map(|aid| aid.to_string())
        .as_deref()
        != Some(source.id.as_str())
    {
        return Err("Bilibili returned metadata for another video.".into());
    }
    let pages = data["pages"]
        .as_array()
        .filter(|pages| pages.len() <= 1000)
        .ok_or("Bilibili returned no bounded video part list.")?;
    let page = pages
        .iter()
        .find(|page| page["page"].as_u64() == Some(source.page as u64))
        .ok_or("The selected Bilibili part is not available.")?;
    let cid = positive_integer(&page["cid"])
        .ok_or("The selected part has no valid danmaku content identifier.")?;
    let title = data["title"]
        .as_str()
        .unwrap_or("Bilibili")
        .chars()
        .filter(|c| !c.is_control())
        .take(240)
        .collect();
    Ok(Part { cid, title })
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
fn endpoint(input: &str, host: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(&crate::model::validate_url(input)?)
        .map_err(|_| "Invalid Bilibili export endpoint.")?;
    if parsed.scheme() != "https" || parsed.host_str() != Some(host) || parsed.port().is_some() {
        return Err(
            "Danmaku exports only use the expected official Bilibili HTTPS endpoint.".into(),
        );
    }
    Ok(parsed)
}
async fn fetch(
    input: &str,
    host: &str,
    limit: usize,
    cancel: watch::Receiver<bool>,
    request_id: u64,
    emit: impl Fn(Progress),
) -> Result<Vec<u8>, String> {
    let work = async {
        let mut url = endpoint(input, host)?;
        for hop in 0..=3 {
            let addresses: Vec<_> =
                tokio::time::timeout(Duration::from_secs(8), tokio::net::lookup_host((host, 443)))
                    .await
                    .map_err(|_| "Bilibili endpoint lookup timed out.")?
                    .map_err(|_| "Bilibili endpoint could not be resolved.")?
                    .collect();
            if addresses.is_empty()
                || addresses
                    .iter()
                    .any(|address| !crate::providers::is_public_ip(address.ip()))
            {
                return Err(
                    "Local or reserved Bilibili endpoint addresses are not supported.".into(),
                );
            }
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, &addresses)
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(90))
                .user_agent(concat!("nabnab/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| "Could not prepare the Bilibili export request.")?;
            let mut response = client
                .get(url.clone())
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .header(reqwest::header::REFERER, "https://www.bilibili.com/")
                .send()
                .await
                .map_err(|_| "The official Bilibili export endpoint could not be reached.")?;
            if response.status().is_redirection() {
                if hop == 3 {
                    return Err("The Bilibili endpoint exceeded its redirect limit.".into());
                }
                let next = url
                    .join(
                        response
                            .headers()
                            .get(reqwest::header::LOCATION)
                            .and_then(|value| value.to_str().ok())
                            .ok_or("Bilibili returned an invalid redirect.")?,
                    )
                    .map_err(|_| "Bilibili returned an invalid redirect URL.")?;
                url = endpoint(next.as_str(), host)?;
                continue;
            }
            match response.status().as_u16(){401|403=>return Err("HTTP 403: Bilibili denied access to this export. No credentials, mirror or access bypass were attempted.".into()),429=>return Err("HTTP 429: Bilibili requested a pause. Retry the export later.".into()),_ if !response.status().is_success()=>return Err(format!("Bilibili export returned HTTP {}.",response.status().as_u16())),_=>{}}
            if response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_some_and(|value| value.as_bytes() != b"identity")
            {
                return Err("Bilibili returned an unsupported encoded export response.".into());
            }
            let total = response.content_length();
            if total.is_some_and(|size| size > limit as u64) {
                return Err("The Bilibili export exceeds its size limit.".into());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "The Bilibili export transfer was interrupted.")?
            {
                if bytes.len().saturating_add(chunk.len()) > limit {
                    return Err("The Bilibili export exceeds its size limit.".into());
                }
                bytes.extend_from_slice(&chunk);
                emit(Progress {
                    request_id,
                    phase: "downloading".into(),
                    percent: total
                        .filter(|total| *total > 0)
                        .map(|total| bytes.len() as f64 / total as f64 * 100.0),
                    speed: None,
                    downloaded: Some(bytes.len() as f64),
                    total: total.map(|value| value as f64),
                });
            }
            return Ok(bytes);
        }
        Err("The Bilibili endpoint redirect limit was exceeded.".into())
    };
    tokio::select! {result=work=>result,_=cancelled(cancel)=>Err(crate::engine::CANCELLED.into())}
}
fn parameters(event: &BytesStart<'_>) -> Result<String, String> {
    let mut result = None;
    for attribute in event.attributes() {
        let attribute = attribute.map_err(|_| "Invalid danmaku XML attributes.")?;
        if attribute.key.as_ref() == "p" {
            if result.is_some() {
                return Err("Duplicate danmaku parameters are not supported.".into());
            }
            result = Some(
                attribute
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|_| "Invalid danmaku XML parameters.")?
                    .into_owned(),
            );
        }
    }
    let value = result.ok_or("A danmaku entry has no timing parameters.")?;
    let fields: Vec<_> = value.split(',').collect();
    if value.len() > 1024
        || fields.len() < 8
        || fields.len() > 16
        || fields[0]
            .parse::<f64>()
            .ok()
            .is_none_or(|time| !time.is_finite() || time < 0.0)
    {
        return Err("A danmaku entry has invalid timing parameters.".into());
    }
    Ok(value)
}
fn append_comment(
    output: &mut String,
    params: &str,
    text: &str,
    count: &mut usize,
) -> Result<(), String> {
    if text.len() > 16 * 1024 || *count >= MAX_COMMENTS {
        return Err("Danmaku entries exceed the supported count or text limit.".into());
    }
    output.push_str("<d p=\"");
    output.push_str(&quick_xml::escape::escape(params));
    output.push_str("\">");
    output.push_str(&quick_xml::escape::escape(text));
    output.push_str("</d>\n");
    *count += 1;
    if output.len() > MAX_XML {
        return Err("The normalized danmaku XML exceeds its size limit.".into());
    }
    Ok(())
}
fn canonical_xml(bytes: &[u8], cid: u64) -> Result<(String, usize), String> {
    if bytes.len() > MAX_XML {
        return Err("Danmaku XML exceeds 32 MiB.".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Danmaku XML must be UTF-8.")?;
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut saw_root = false;
    let mut pending: Option<(String, String)> = None;
    let mut count = 0;
    let mut output=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<i><chatid>{cid}</chatid><!-- Returned accessible comments only; not a complete historical archive. -->\n");
    loop {
        match reader
            .read_event()
            .map_err(|_| "Bilibili returned invalid danmaku XML.")?
        {
            Event::Start(event) => {
                if depth == 0 {
                    if saw_root || event.name().as_ref() != "i" {
                        return Err("The endpoint did not return a danmaku XML document.".into());
                    }
                    saw_root = true;
                } else if pending.is_some() {
                    return Err("Danmaku entries cannot contain active or nested markup.".into());
                } else if depth == 1 && event.name().as_ref() == "d" {
                    pending = Some((parameters(&event)?, String::new()));
                }
                depth += 1;
                if depth > 8 {
                    return Err("Danmaku XML nesting exceeds its limit.".into());
                }
            }
            Event::Empty(event) => {
                if depth == 1 && event.name().as_ref() == "d" {
                    append_comment(&mut output, &parameters(&event)?, "", &mut count)?;
                } else if pending.is_some() {
                    return Err("Danmaku entries cannot contain nested markup.".into());
                }
            }
            Event::End(event) => {
                if depth == 0 {
                    return Err("Unexpected danmaku XML closing tag.".into());
                }
                if event.name().as_ref() == "d" && depth == 2 {
                    if let Some((params, text)) = pending.take() {
                        append_comment(&mut output, &params, &text, &mut count)?;
                    }
                }
                depth -= 1;
            }
            Event::Text(event) => {
                if let Some((_, text)) = &mut pending {
                    text.push_str(&event.xml10_content());
                    if text.len() > 16 * 1024 {
                        return Err("A danmaku comment exceeds its text limit.".into());
                    }
                }
            }
            Event::CData(event) => {
                if let Some((_, text)) = &mut pending {
                    text.push_str(&event.xml10_content());
                    if text.len() > 16 * 1024 {
                        return Err("A danmaku comment exceeds its text limit.".into());
                    }
                }
            }
            Event::GeneralRef(event) => {
                if let Some((_, text)) = &mut pending {
                    let entity = format!("&{};", event.xml10_content());
                    text.push_str(
                        &quick_xml::escape::unescape(&entity)
                            .map_err(|_| "Danmaku XML contains an unsupported entity.")?,
                    );
                }
            }
            Event::DocType(_) | Event::PI(_) => return Err(
                "Danmaku XML with external entities or processing instructions is not supported."
                    .into(),
            ),
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_root || depth != 0 || pending.is_some() {
        return Err("Danmaku XML is incomplete.".into());
    }
    if count == 0 {
        return Err("Bilibili returned no accessible danmaku comments for this part. No empty export was created.".into());
    }
    output.push_str("</i>\n");
    if output.len() > MAX_XML {
        return Err("The normalized danmaku XML exceeds its size limit.".into());
    }
    Ok((output, count))
}

pub async fn execute(
    _app: &AppHandle,
    _state: &Arc<AppState>,
    _root: &Path,
    url: &str,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Clone + Send + Sync + 'static,
) -> Result<PathBuf, String> {
    let source = source(url)?;
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let metadata = fetch(
        &metadata_url(&source),
        "api.bilibili.com",
        2 * 1024 * 1024,
        cancel.clone(),
        run_id,
        emit.clone(),
    )
    .await?;
    let value: Value = serde_json::from_slice(&metadata)
        .map_err(|_| "Bilibili returned unreadable video metadata.")?;
    let part = part(&value, &source)?;
    let bytes = fetch(
        &format!("https://comment.bilibili.com/{}.xml", part.cid),
        "comment.bilibili.com",
        MAX_XML,
        cancel.clone(),
        run_id,
        emit.clone(),
    )
    .await?;
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    let (xml, _) = canonical_xml(&bytes, part.cid)?;
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let staging = crate::library::authorized_path(staging)?;
    let title: String = crate::options::safe_segment(&part.title)
        .chars()
        .take(80)
        .collect();
    let path = staging.join(format!(
        "{} [{}]-p{}-danmaku.xml",
        title, source.id, source.page
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "Could not create a new danmaku export. No existing file was overwritten.")?;
    file.write_all(xml.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save the danmaku export.")?;
    if !fs::metadata(&path).map_err(|e| e.to_string())?.is_file() {
        return Err("The danmaku export did not create a regular file.".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sources_select_a_standard_video_and_one_part() {
        assert_eq!(
            source("https://www.bilibili.com/video/BV1Q541167Qg/?p=2&tracking=discard").unwrap(),
            Source {
                id: "BV1Q541167Qg".into(),
                bvid: true,
                page: 2
            }
        );
        assert_eq!(
            source("https://www.bilibili.com/video/av1234").unwrap().id,
            "1234"
        );
        for value in [
            "https://b23.tv/short",
            "https://www.bilibili.com/bangumi/play/ep1",
            "https://bilibili.com.attacker.net/video/av123",
            "https://www.bilibili.com/video/av123?p=0",
            "https://www.bilibili.com/video/av123?p=1&p=2",
        ] {
            assert!(source(value).is_err(), "{value}");
        }
    }
    #[test]
    fn metadata_cid_is_bound_to_the_requested_video_and_part() {
        let source = source("https://www.bilibili.com/video/BV1Q541167Qg?p=2").unwrap();
        let mut value = serde_json::json!({"code":0,"data":{"bvid":"BV1Q541167Qg","title":"Fixture","pages":[{"page":1,"cid":100},{"page":2,"cid":200}]}});
        assert_eq!(part(&value, &source).unwrap().cid, 200);
        value["data"]["bvid"] = serde_json::json!("other");
        assert!(part(&value, &source).is_err());
        value["code"] = serde_json::json!(-403);
        assert!(part(&value, &source).is_err());
    }
    #[test]
    fn canonical_xml_preserves_timing_order_and_escaped_comments() {
        let xml=b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><i><chatserver>comment.bilibili.com</chatserver><d p=\"1.5,1,25,16777215,1700000000,0,user,42\">A &amp; B</d><d p=\"2,5,25,1,1700000000,0,user,43\"><![CDATA[<script>data only</script>]]></d></i>";
        let (output, count) = canonical_xml(xml, 200).unwrap();
        assert_eq!(count, 2);
        assert!(output.contains("<chatid>200</chatid>"));
        assert!(output.contains("p=\"1.5,1,25,16777215,1700000000,0,user,42\">A &amp; B</d>"));
        assert!(output.contains("&lt;script&gt;data only&lt;/script&gt;"));
        assert!(!output.contains("<script>"));
        assert!(output.find("1.5,1").unwrap() < output.find("2,5").unwrap());
    }
    #[test]
    fn external_entities_markup_and_empty_archives_are_not_reported_as_success() {
        for xml in [
            "<i></i>",
            "<!DOCTYPE i SYSTEM 'https://attacker.example.org/private'><i/>",
            "<?xml-stylesheet href='https://attacker.example.org/style'?><i/>",
            "<html><script>not danmaku</script></html>",
            "<i><d p='1,1,25,1,1,0,u,1'><script>active</script></d></i>",
        ] {
            assert!(canonical_xml(xml.as_bytes(), 1).is_err(), "{xml}");
        }
        assert!(endpoint(
            "https://attacker.example.org/200.xml",
            "comment.bilibili.com"
        )
        .is_err());
        assert!(endpoint(
            "http://comment.bilibili.com/200.xml",
            "comment.bilibili.com"
        )
        .is_err());
    }
}
