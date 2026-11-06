//! RSS enclosures use the shared source scheduler and download queue.
use crate::{model, providers, sessions};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, net::SocketAddr, time::Duration};
use tokio::sync::watch;

const MAX_FEED_BYTES: usize = 4 * 1024 * 1024;
const MAX_EPISODES: usize = 10_000;

#[derive(Clone, Debug)]
pub struct Episode {
    pub id: String,
    pub title: String,
    pub author: String,
    pub enclosure: String,
    pub duration: Option<f64>,
    pub published_at: Option<u64>,
    pub size: Option<u64>,
    pub media_type: String,
}

#[derive(Clone, Debug)]
pub struct Feed {
    pub title: String,
    pub author: String,
    pub artwork: Option<String>,
    pub episodes: Vec<Episode>,
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn feed_id(url: &str) -> String {
    format!("podcast:{}", digest(url))
}

pub fn valid_episode_id(id: &str) -> bool {
    id.strip_prefix("podcast:")
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn clean(value: &str, limit: usize) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .take(limit)
        .collect()
}

fn duration(value: &str) -> Option<f64> {
    let values: Vec<_> = value
        .trim()
        .split(':')
        .map(str::parse::<f64>)
        .collect::<Result<_, _>>()
        .ok()?;
    if values.is_empty() || values.len() > 3 || values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return None;
    }
    let seconds = values.iter().fold(0.0, |sum, part| sum * 60.0 + part);
    (seconds > 0.0 && seconds < 365.0 * 86400.0).then_some(seconds)
}

fn attr(event: &BytesStart<'_>, name: &str) -> Option<String> {
    event.attributes().find_map(|attribute| {
        let attribute = attribute.ok()?;
        (attribute.key.as_ref() == name)
            .then(|| {
                attribute
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
            .flatten()
    })
}

fn remote(base: &str, value: &str) -> Option<String> {
    let resolved = url::Url::parse(base).ok()?.join(value).ok()?;
    model::validate_url(resolved.as_str()).ok()
}

#[derive(Default)]
struct Entry {
    guid: String,
    title: String,
    author: String,
    enclosure: String,
    duration: Option<f64>,
    published_at: Option<u64>,
    size: Option<u64>,
    media_type: String,
}

fn episode_identity(feed: &str, entry: &Entry, reliable_guid: bool) -> String {
    let identity = if reliable_guid {
        format!("guid\0{}", entry.guid)
    } else {
        // Feed GUIDs win. Without one, ignore transport query/fragment tokens while retaining the path.
        let mut enclosure = url::Url::parse(&entry.enclosure).expect("validated enclosure");
        enclosure.set_query(None);
        enclosure.set_fragment(None);
        format!("enclosure\0{}", enclosure.as_str())
    };
    format!("podcast:{}", digest(&format!("{feed}\0{identity}")))
}

pub fn parse_feed(xml: &str, feed_url: &str) -> Result<Feed, String> {
    if xml.len() > MAX_FEED_BYTES {
        return Err("The podcast feed exceeds 4 MiB.".into());
    }
    model::validate_url(feed_url)?;
    let mut reader = Reader::from_str(xml.trim_start_matches('\u{feff}'));
    reader.config_mut().trim_text(false);
    let mut stack = Vec::<String>::new();
    let mut feed = Feed {
        title: String::new(),
        author: String::new(),
        artwork: None,
        episodes: Vec::new(),
    };
    let mut current: Option<Entry> = None;
    let mut entries = Vec::new();
    let mut saw_rss = false;
    loop {
        match reader
            .read_event()
            .map_err(|_| "The podcast returned malformed UTF-8 RSS XML.")?
        {
            Event::DocType(_) => return Err("RSS documents with a DTD are not supported.".into()),
            Event::Start(event) | Event::Empty(event) => {
                let name = event.name().as_ref().to_owned();
                let empty = xml
                    .as_bytes()
                    .get(reader.buffer_position().saturating_sub(2) as usize)
                    == Some(&b'/');
                if name == "rss" {
                    saw_rss = true;
                }
                if name == "item" {
                    if current.is_some() || entries.len() >= MAX_EPISODES {
                        return Err(
                            "RSS item nesting or the 10000-episode limit was exceeded.".into()
                        );
                    }
                    current = Some(Entry::default());
                }
                if let Some(entry) = &mut current {
                    if name == "enclosure" && entry.enclosure.is_empty() {
                        if let Some(value) =
                            attr(&event, "url").and_then(|value| remote(feed_url, &value))
                        {
                            let media_type = attr(&event, "type").unwrap_or_default();
                            if media_type.is_empty()
                                || media_type.starts_with("audio/")
                                || media_type.starts_with("video/")
                                || media_type == "application/octet-stream"
                            {
                                entry.enclosure = value;
                                entry.media_type = media_type;
                                entry.size = attr(&event, "length")
                                    .and_then(|v| v.parse().ok())
                                    .filter(|v| *v > 0);
                            }
                        }
                    }
                } else if name == "itunes:image" {
                    feed.artwork = attr(&event, "href").and_then(|v| remote(feed_url, &v));
                }
                if !empty {
                    stack.push(name);
                }
                if stack.len() > 64 {
                    return Err("The RSS nesting limit was exceeded.".into());
                }
            }
            Event::End(event) => {
                if event.name().as_ref() == "item" {
                    if let Some(entry) = current.take().filter(|entry| !entry.enclosure.is_empty())
                    {
                        entries.push(entry);
                    }
                }
                stack.pop();
            }
            Event::Text(event) => {
                let text = event.xml10_content();
                append_text(&mut feed, &mut current, &stack, &text);
            }
            Event::CData(event) => {
                let text = event.as_ref();
                append_text(&mut feed, &mut current, &stack, &text);
            }
            Event::GeneralRef(event) => {
                let name = event.xml10_content();
                let value = quick_xml::escape::unescape(&format!("&{name};"))
                    .map_err(|_| "The feed contains an unsupported XML entity.")?
                    .into_owned();
                append_text(&mut feed, &mut current, &stack, &value);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_rss || !stack.is_empty() {
        return Err("Use an RSS podcast feed with enclosure attachments.".into());
    }
    let mut guid_counts = std::collections::HashMap::new();
    for entry in &entries {
        if !entry.guid.trim().is_empty() {
            *guid_counts
                .entry(entry.guid.trim().to_owned())
                .or_insert(0usize) += 1;
        }
    }
    let mut known = HashSet::new();
    let identity = feed_id(feed_url);
    for mut entry in entries {
        entry.guid = clean(&entry.guid, 4096);
        let id = episode_identity(
            &identity,
            &entry,
            !entry.guid.is_empty() && guid_counts.get(&entry.guid) == Some(&1),
        );
        if !known.insert(id.clone()) {
            continue;
        }
        feed.episodes.push(Episode {
            id,
            title: if entry.title.trim().is_empty() {
                "Podcast episode".into()
            } else {
                clean(&entry.title, 1024)
            },
            author: if entry.author.trim().is_empty() {
                clean(&feed.author, 512)
            } else {
                clean(&entry.author, 512)
            },
            enclosure: entry.enclosure,
            duration: entry.duration,
            published_at: entry.published_at,
            size: entry.size,
            media_type: entry.media_type,
        });
    }
    feed.title = clean(&feed.title, 240);
    feed.author = clean(&feed.author, 512);
    Ok(feed)
}

fn append_text(feed: &mut Feed, current: &mut Option<Entry>, stack: &[String], text: &str) {
    let Some(name) = stack.last().map(String::as_str) else {
        return;
    };
    if let Some(entry) = current {
        match name {
            "title" if entry.title.len() < 8192 => entry.title.push_str(text),
            "guid" if entry.guid.len() < 8192 => entry.guid.push_str(text),
            "itunes:author" | "dc:creator" if entry.author.len() < 4096 => {
                entry.author.push_str(text)
            }
            "itunes:duration" => entry.duration = duration(text),
            "pubDate" => {
                entry.published_at = httpdate::parse_http_date(text.trim())
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
            }
            _ => {}
        }
    } else {
        match name {
            "title"
                if stack.get(stack.len().saturating_sub(2)).map(String::as_str)
                    == Some("channel")
                    && feed.title.len() < 8192 =>
            {
                feed.title.push_str(text)
            }
            "itunes:author" | "dc:creator" if feed.author.len() < 4096 => {
                feed.author.push_str(text)
            }
            "url" if stack.iter().any(|part| part == "image") && feed.artwork.is_none() => {}
            _ => {}
        }
    }
}

async fn fetch_inner(input: &str) -> Result<Feed, String> {
    let mut url = url::Url::parse(&model::validate_url(input)?).map_err(|_| "Invalid RSS URL.")?;
    for hop in 0..=5 {
        let host = url.host_str().ok_or("The RSS URL has no host.")?;
        let port = url
            .port_or_known_default()
            .ok_or("Unsupported RSS scheme.")?;
        let approved: Vec<SocketAddr> = tokio::time::timeout(
            Duration::from_secs(8),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| "RSS hostname lookup timed out.")?
        .map_err(|_| "RSS hostname lookup failed.")?
        .collect();
        if approved.is_empty()
            || approved
                .iter()
                .any(|address| !providers::is_public_ip(address.ip()))
        {
            return Err("Local or reserved RSS destinations are not supported.".into());
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &approved)
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .user_agent("nabnab podcast reader")
            .build()
            .map_err(|_| "Could not prepare RSS request.")?;
        let mut response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| "The RSS request failed. Check the connection and retry.")?;
        if response.status().is_redirection() {
            if hop == 5 {
                return Err("The RSS redirect limit was exceeded.".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("Invalid RSS redirect.")?;
            let next = url
                .join(location)
                .map_err(|_| "Invalid RSS redirect URL.")?;
            if url.scheme() == "https" && next.scheme() != "https" {
                return Err("RSS redirects cannot downgrade HTTPS.".into());
            }
            model::validate_url(next.as_str())?;
            url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "RSS server returned HTTP {}.",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_FEED_BYTES as u64)
        {
            return Err("The podcast feed exceeds 4 MiB.".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The RSS transfer was interrupted.")?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_FEED_BYTES {
                return Err("The podcast feed exceeds 4 MiB.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let xml = std::str::from_utf8(&bytes).map_err(|_| "RSS feeds must use UTF-8 XML.")?;
        return parse_feed(xml, input);
    }
    Err("The RSS redirect limit was exceeded.".into())
}

pub async fn fetch_feed(input: &str, mut cancel: watch::Receiver<bool>) -> Result<Feed, String> {
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    tokio::select! { result = fetch_inner(input) => result, _ = cancel.changed() => Err(crate::engine::CANCELLED.into()) }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub title: String,
    pub url: String,
}

pub fn parse_opml(xml: &str) -> Result<Vec<Subscription>, String> {
    if xml.len() > MAX_FEED_BYTES {
        return Err("The OPML file exceeds 4 MiB.".into());
    }
    let mut reader = Reader::from_str(xml);
    let mut feeds = Vec::new();
    let mut known = HashSet::new();
    let mut opml = false;
    loop {
        match reader
            .read_event()
            .map_err(|_| "The OPML file contains malformed XML.")?
        {
            Event::DocType(_) => return Err("OPML files with a DTD are not supported.".into()),
            Event::Start(event) | Event::Empty(event) => {
                if event.name().as_ref() == "opml" {
                    opml = true;
                }
                if event.name().as_ref() != "outline" {
                    continue;
                }
                if let Some(url) = attr(&event, "xmlUrl") {
                    let url = model::validate_url(&url)?;
                    if known.insert(url.clone()) {
                        feeds.push(Subscription {
                            title: clean(
                                &attr(&event, "title")
                                    .or_else(|| attr(&event, "text"))
                                    .unwrap_or_else(|| "Podcast".into()),
                                240,
                            ),
                            url,
                        });
                        if feeds.len() > 1000 {
                            return Err("Import at most 1000 podcast subscriptions.".into());
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !opml {
        return Err("Choose an OPML subscription file.".into());
    }
    Ok(feeds)
}

pub fn export_opml(subscriptions: &[Subscription]) -> String {
    let mut output = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\"><head><title>nabnab podcasts</title></head><body>\n");
    for subscription in subscriptions {
        output.push_str(&format!(
            "<outline type=\"rss\" text=\"{}\" xmlUrl=\"{}\"/>\n",
            quick_xml::escape::escape(&subscription.title),
            quick_xml::escape::escape(&subscription.url)
        ));
    }
    output.push_str("</body></opml>\n");
    output
}

pub fn protected_url(
    root: &std::path::Path,
    reference: &Option<String>,
    public: &str,
) -> Result<String, String> {
    match reference {
        Some(reference) => Ok(sessions::resolve_url(root, reference)?.as_str().to_owned()),
        None => model::validate_url(public),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FEED: &str = r#"<?xml version="1.0"?><rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel><title>Study &amp; listen</title><itunes:author>Author</itunes:author><itunes:image href="https://example.org/cover.jpg"/><item><title><![CDATA[Episode <one>]]></title><guid>stable-one</guid><pubDate>Wed, 21 Oct 2015 07:28:00 GMT</pubDate><itunes:duration>01:02:03</itunes:duration><enclosure url="https://cdn.example.org/one.mp3?token=first&amp;v=1" type="audio/mpeg" length="1234"/></item><item><title>Second</title><enclosure url="https://cdn.example.org/two.mp3?token=changing" type="audio/mpeg"/></item></channel></rss>"#;

    #[test]
    fn rss_preserves_metadata_order_and_guid_identity() {
        let feed = parse_feed(FEED, "https://example.org/podcast.xml").unwrap();
        assert_eq!(feed.title, "Study & listen");
        assert_eq!(feed.author, "Author");
        assert_eq!(
            feed.artwork.as_deref(),
            Some("https://example.org/cover.jpg")
        );
        assert_eq!(feed.episodes.len(), 2);
        assert_eq!(feed.episodes[0].title, "Episode <one>");
        assert_eq!(feed.episodes[0].duration, Some(3723.0));
        assert_eq!(feed.episodes[0].size, Some(1234));
        assert_eq!(feed.episodes[0].published_at, Some(1445412480000));
        let changed = parse_feed(
            &FEED
                .replace("token=first", "token=second")
                .replace("token=changing", "token=other"),
            "https://example.org/podcast.xml",
        )
        .unwrap();
        assert_eq!(feed.episodes[0].id, changed.episodes[0].id);
        assert_eq!(feed.episodes[1].id, changed.episodes[1].id);
        assert_eq!(
            feed.episodes[0].enclosure,
            "https://cdn.example.org/one.mp3?token=first&v=1"
        );
    }

    #[test]
    fn untrusted_xml_and_reserved_enclosures_are_rejected() {
        assert!(parse_feed(
            "<!DOCTYPE rss [<!ENTITY secret SYSTEM 'file:///etc/passwd'>]><rss/>",
            "https://example.org/rss"
        )
        .is_err());
        assert!(parse_feed("<rss><channel>", "https://example.org/rss").is_err());
        let feed = parse_feed(
            &FEED.replace("https://cdn.example.org/", "http://127.0.0.1/"),
            "https://example.org/rss",
        )
        .unwrap();
        assert!(feed.episodes.is_empty());
    }

    #[test]
    fn duplicate_guids_use_enclosure_fallback() {
        let xml = FEED.replace(
            "<title>Second</title>",
            "<title>Second</title><guid>stable-one</guid>",
        );
        let feed = parse_feed(&xml, "https://example.org/rss").unwrap();
        assert_eq!(feed.episodes.len(), 2);
        assert_ne!(feed.episodes[0].id, feed.episodes[1].id);
    }

    #[test]
    fn opml_imports_nested_feeds_and_roundtrips_escaped_attributes() {
        let xml = r#"<opml version="2.0"><body><outline text="Group"><outline type="rss" text="A &amp; B" xmlUrl="https://example.org/rss?x=1&amp;y=2"/></outline></body></opml>"#;
        let feeds = parse_opml(xml).unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].title, "A & B");
        assert_eq!(feeds[0].url, "https://example.org/rss?x=1&y=2");
        assert_eq!(
            parse_opml(&export_opml(&feeds)).unwrap()[0].url,
            feeds[0].url
        );
        assert!(parse_opml("<rss/>").is_err());
    }
}
