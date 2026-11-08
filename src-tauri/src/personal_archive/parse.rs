use crate::{delimited, gallery_sources, model};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::LazyLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Instagram,
    Facebook,
    X,
    Spotify,
    Linkedin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Posts,
    Saved,
    Media,
    Likes,
    Bookmarks,
    Listening,
    Playlists,
    Contacts,
    Messages,
}

impl Category {
    pub fn sensitive(self) -> bool {
        matches!(self, Self::Contacts | Self::Messages)
    }
}

impl Provider {
    pub fn categories(self) -> &'static [Category] {
        use Category::*;
        match self {
            Self::Instagram | Self::Facebook => &[Posts, Saved, Media, Contacts, Messages],
            Self::X => &[Posts, Likes, Bookmarks, Media, Messages],
            Self::Spotify => &[Listening, Playlists],
            Self::Linkedin => &[Contacts, Posts, Messages],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Json,
    Csv,
    Html,
    XAssignment,
    Media,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub category: Category,
    pub format: Format,
}

pub fn classify(provider: Provider, path: &str) -> Option<Classification> {
    use Category::*;
    let path = path.to_ascii_lowercase().replace(' ', "_");
    let file = path.rsplit('/').next()?;
    let (stem, extension) = file.rsplit_once('.')?;
    if path.split('/').any(|part| {
        let basename = part.split('.').next().unwrap_or(part);
        [
            "cookie",
            "session",
            "credential",
            "password",
            "login",
            "access_token",
            "oauth",
        ]
        .iter()
        .any(|term| basename.contains(term))
    }) {
        return None;
    }

    let media = matches!(
        extension,
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "mp4" | "mov" | "webm" | "mp3" | "m4a" | "wav"
    );
    let parent = path
        .rsplit_once('/')
        .map(|(parents, _)| parents)
        .unwrap_or("");
    let parents = || parent.split('/');
    let messages = parents().any(|part| {
        matches!(
            part,
            "messages"
                | "direct_messages"
                | "direct-messages"
                | "direct_messages_media"
                | "direct-messages-media"
                | "direct_message_media"
                | "direct-message-media"
                | "direct_messages_group_media"
                | "direct-messages-group-media"
        )
    }) || stem == "messages"
        || stem.starts_with("message_")
        || stem.starts_with("direct-message");
    let contacts = parents().any(|part| matches!(part, "contacts" | "connections"))
        || stem.contains("contact")
        || stem == "connections";
    let category = match provider {
        Provider::Instagram | Provider::Facebook => {
            if messages {
                Messages
            } else if contacts {
                Contacts
            } else if media
                && parents().any(|part| {
                    matches!(
                        part,
                        "media"
                            | "posts"
                            | "photos_and_videos"
                            | "photos"
                            | "videos"
                            | "reels"
                            | "stories"
                            | "profile_photos"
                    )
                })
            {
                Media
            } else if stem.starts_with("saved")
                || parents().any(|part| matches!(part, "saved" | "saved_items" | "saved_posts"))
            {
                Saved
            } else if stem.starts_with("your_posts")
                || stem.starts_with("posts")
                || stem.starts_with("photos_and_videos")
            {
                Posts
            } else {
                return None;
            }
        }
        Provider::X => {
            if messages {
                Messages
            } else if media
                && parents().any(|part| {
                    matches!(
                        part,
                        "tweets_media" | "tweet_media" | "tweet-media" | "tweets-media"
                    )
                })
            {
                Media
            } else if stem.starts_with("tweets") || stem.starts_with("tweet-") || stem == "tweet" {
                Posts
            } else if stem == "like" || stem.starts_with("likes") {
                Likes
            } else if stem.starts_with("bookmark") {
                Bookmarks
            } else {
                return None;
            }
        }
        Provider::Spotify => {
            if stem.starts_with("streaminghistory")
                || stem.starts_with("streaming_history")
                || stem.starts_with("endsong")
            {
                Listening
            } else if stem.starts_with("playlist") {
                Playlists
            } else {
                return None;
            }
        }
        Provider::Linkedin => {
            if messages {
                Messages
            } else if contacts {
                Contacts
            } else if matches!(stem, "shares" | "posts" | "publications" | "articles") {
                Posts
            } else {
                return None;
            }
        }
    };
    let format = match extension {
        "json" => Format::Json,
        "csv" => Format::Csv,
        "html" | "htm" => Format::Html,
        "js" if provider == Provider::X => Format::XAssignment,
        _ if media => Format::Media,
        _ => Format::Unsupported,
    };
    Some(Classification { category, format })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: Option<String>,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub source: String,
    pub title: String,
    pub text: String,
    pub time: Option<u64>,
    pub link: Option<String>,
    pub author: Option<String>,
    pub thread_parent: Option<String>,
    pub played_ms: Option<u64>,
    pub reached_end: Option<bool>,
    pub completed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listening_item: Option<ListeningItem>,
    pub tracks: Vec<Track>,
    pub media: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListeningItem {
    pub uri: Option<String>,
    pub album: Option<String>,
    pub episode: bool,
}

impl Record {
    pub(super) fn reply_parent(&self) -> Option<String> {
        numeric_record_id(self.thread_parent.as_deref()?)
    }
}

fn numeric_record_id(id: &str) -> Option<String> {
    (id.len() <= 20 && !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| record_id(id))
}

pub(super) fn x_attachment_owner(source: &str) -> Option<String> {
    if classify(Provider::X, source)?
        != (Classification {
            category: Category::Media,
            format: Format::Media,
        })
    {
        return None;
    }
    let (id, original) = source.rsplit('/').next()?.split_once('-')?;
    let (stem, _) = original.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    numeric_record_id(id)
}

fn record_id(identity: &str) -> String {
    format!("{:x}", Sha256::digest(identity))
}

fn clean(value: &str, limit: usize) -> Option<String> {
    let value = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .take(limit)
        .collect::<String>();
    (!value.is_empty()).then_some(value)
}

fn string(value: &Value, keys: &[&str], limit: usize) -> Option<String> {
    keys.iter()
        .find_map(|key| value[*key].as_str().and_then(|text| clean(text, limit)))
}

fn field_id(value: &Value, keys: &[&str]) -> Option<String> {
    string(value, keys, 192).or_else(|| {
        keys.iter()
            .find_map(|key| value[*key].as_u64().map(|id| id.to_string()))
    })
}

fn public_link(value: &str) -> Option<String> {
    let input = model::validate_url(value.trim()).ok()?;
    let mut parsed = url::Url::parse(&input).ok()?;
    if parsed.host_str().is_none_or(|host| {
        !host.contains('.')
            || host.ends_with(".local")
            || host.ends_with(".localhost")
            || host.parse::<std::net::IpAddr>().is_ok()
    }) || parsed.query_pairs().any(|(key, _)| {
        let key = key.to_ascii_lowercase();
        [
            "token",
            "sign",
            "secret",
            "cookie",
            "session",
            "auth",
            "credential",
            "password",
            "api_key",
            "code",
        ]
        .iter()
        .any(|term| key.contains(term))
    }) {
        return None;
    }
    parsed.set_fragment(None);
    let url = parsed.to_string();
    (url.len() <= 2048).then_some(url)
}

fn time_string(value: &str) -> Option<u64> {
    let value = value.trim();
    if let Some(time) = gallery_sources::published_at(value) {
        return Some(time);
    }
    if value.as_bytes().get(4) == Some(&b'-') && value.as_bytes().get(7) == Some(&b'-') {
        let date = value.get(..10)?;
        let clock = value.get(11..).unwrap_or("");
        let clock = clock
            .get(..8)
            .or_else(|| clock.get(..5))
            .unwrap_or("00:00:00");
        let iso = if clock.len() == 5 {
            format!("{date}T{clock}:00Z")
        } else {
            format!("{date}T{clock}Z")
        };
        return gallery_sources::published_at(&iso);
    }
    let parts: Vec<_> = value.split_whitespace().collect();
    if let [_, month, day, clock, zone, year] = parts.as_slice() {
        let number = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ]
        .iter()
        .position(|candidate| candidate == month)?
            + 1;
        let hour = zone.get(..3)?;
        let minute = zone.get(3..5)?;
        return gallery_sources::published_at(&format!(
            "{year}-{number:02}-{day}T{clock}{hour}:{minute}"
        ));
    }
    if let [day, month, year] = parts.as_slice() {
        let number = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ]
        .iter()
        .position(|candidate| candidate == month)?
            + 1;
        let day: u8 = day.parse().ok()?;
        return gallery_sources::published_at(&format!("{year}-{number:02}-{day:02}T00:00:00Z"));
    }
    None
}

fn timestamp(value: &Value) -> Option<u64> {
    for key in [
        "timestamp_ms",
        "timestamp",
        "creation_timestamp",
        "created_at",
        "createdAt",
        "date",
        "endTime",
        "ts",
    ] {
        let raw = &value[key];
        if let Some(number) = raw
            .as_u64()
            .or_else(|| raw.as_str().and_then(|value| value.parse().ok()))
        {
            let ms = if key == "timestamp_ms" || number >= 100_000_000_000 {
                number
            } else {
                number.checked_mul(1000)?
            };
            if ms <= 253_402_300_799_999 {
                return Some(ms);
            }
        }
        if let Some(date) = raw.as_str().and_then(time_string) {
            return Some(date);
        }
    }
    None
}

fn empty(source: &str) -> Record {
    Record {
        id: String::new(),
        source: source.into(),
        title: String::new(),
        text: String::new(),
        time: None,
        link: None,
        author: None,
        thread_parent: None,
        played_ms: None,
        reached_end: None,
        completed: None,
        listening_item: None,
        tracks: vec![],
        media: None,
    }
}

fn finish(mut record: Record, external_id: Option<String>) -> Record {
    let identity = external_id.unwrap_or_else(|| {
        format!(
            "{:?}\n{}\n{}\n{:?}\n{:?}\n{:?}",
            record.time, record.title, record.text, record.link, record.author, record.played_ms
        )
    });
    record.id = record_id(&identity);
    record
}

fn spotify_playlist(value: &Value, source: &str) -> Option<Record> {
    let mut record = empty(source);
    record.title = string(value, &["name"], 256)?;
    let items = value["items"].as_array()?;
    record.time = string(value, &["lastModifiedDate"], 64)
        .as_deref()
        .and_then(time_string);
    record.text = string(value, &["description"], 12000).unwrap_or_default();
    record.tracks = items
        .iter()
        .take(2000)
        .filter_map(|item| {
            let track = &item["track"];
            let episode = &item["episode"];
            Some(Track {
                id: string(track, &["trackUri", "uri"], 192)
                    .or_else(|| string(episode, &["episodeUri", "uri"], 192)),
                title: string(track, &["trackName", "name"], 256)
                    .or_else(|| string(episode, &["episodeName", "name"], 256))
                    .or_else(|| clean(item["localTrack"].as_str()?, 256))?,
                artist: string(track, &["artistName", "artist"], 256)
                    .or_else(|| string(episode, &["showName"], 256)),
                album: string(track, &["albumName", "album"], 256),
            })
        })
        .collect();
    Some(finish(record, field_id(value, &["id", "uri"])))
}

fn json_record(
    provider: Provider,
    category: Category,
    value: &Value,
    source: &str,
) -> Option<Record> {
    if !value.is_object() {
        return None;
    }
    if provider == Provider::Spotify && category == Category::Playlists {
        return spotify_playlist(value, source);
    }
    let value = [
        "tweet",
        "like",
        "bookmark",
        "messageCreate",
        "message",
        "post",
    ]
    .iter()
    .find_map(|key| value[*key].is_object().then_some(&value[*key]))
    .unwrap_or(value);
    let mut record = empty(source);
    let external = field_id(value, &["id_str", "id", "tweetId", "post_id", "messageId"]);
    record.time = timestamp(value);
    record.author = string(
        value,
        &[
            "artistName",
            "master_metadata_album_artist_name",
            "episode_show_name",
            "sender_name",
            "senderName",
            "from",
            "author",
            "name",
        ],
        256,
    );
    record.title = string(
        value,
        &[
            "title",
            "trackName",
            "master_metadata_track_name",
            "episode_name",
            "name",
        ],
        256,
    )
    .unwrap_or_default();
    record.text = string(
        value,
        &[
            "full_text",
            "fullText",
            "text",
            "content",
            "description",
            "post",
            "message",
        ],
        12000,
    )
    .or_else(|| {
        value["data"].as_array().and_then(|items| {
            items
                .iter()
                .find_map(|entry| string(entry, &["post", "text", "content"], 12000))
        })
    })
    .unwrap_or_default();
    record.link = string(
        value,
        &["expandedUrl", "url", "href", "link", "permalink"],
        2048,
    )
    .as_deref()
    .and_then(public_link)
    .or_else(|| {
        value["entities"]["urls"].as_array().and_then(|items| {
            items
                .iter()
                .find_map(|item| item["expanded_url"].as_str().and_then(public_link))
        })
    });
    if let Some(fields) = value["string_map_data"].as_object() {
        for details in fields.values() {
            if record.link.is_none() {
                record.link = details["href"].as_str().and_then(public_link);
            }
            if record.time.is_none() {
                record.time = timestamp(details);
            }
            if record.text.is_empty() {
                record.text = string(details, &["value"], 12000).unwrap_or_default();
            }
        }
    }
    if provider == Provider::X {
        record.thread_parent = field_id(
            value,
            &["in_reply_to_status_id_str", "in_reply_to_status_id"],
        );
    }
    if category == Category::Listening {
        let episode = value["episode_name"]
            .as_str()
            .is_some_and(|name| !name.trim().is_empty());
        let (uri_field, uri_prefix, album_field) = if episode {
            (
                "spotify_episode_uri",
                "spotify:episode:",
                "episode_show_name",
            )
        } else {
            (
                "spotify_track_uri",
                "spotify:track:",
                "master_metadata_album_album_name",
            )
        };
        if !record.title.is_empty() {
            let uri = value[uri_field]
                .as_str()
                .filter(|uri| {
                    uri.strip_prefix(uri_prefix).is_some_and(|id| {
                        id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    })
                })
                .map(str::to_owned);
            record.listening_item = Some(ListeningItem {
                uri,
                album: string(value, &[album_field], 256),
                episode,
            });
        }
        record.played_ms = ["msPlayed", "ms_played"]
            .iter()
            .find_map(|key| {
                value[*key]
                    .as_u64()
                    .or_else(|| value[*key].as_str()?.parse().ok())
            })
            .filter(|value| *value <= 7 * 86_400_000);
        record.reached_end = value["reason_end"]
            .as_str()
            .filter(|reason| !reason.is_empty())
            .map(|reason| reason == "trackdone");
        record.completed = value["completed"].as_bool().or_else(|| {
            let duration = value["duration_ms"]
                .as_u64()
                .filter(|duration| *duration > 0)?;
            Some(record.reached_end? && record.played_ms? >= duration)
        });
        if record.title.is_empty() && record.author.is_none() && record.played_ms.is_none() {
            return None;
        }
    }
    if record.title.is_empty() {
        record.title = clean(&record.text, 160)
            .or_else(|| record.author.clone())
            .or_else(|| external.clone())
            .unwrap_or_default();
    }
    if record.title.is_empty()
        && record.text.is_empty()
        && record.link.is_none()
        && record.played_ms.is_none()
    {
        return None;
    }
    Some(finish(record, external))
}

fn json_rows(value: &Value, category: Category) -> Option<Vec<&Value>> {
    if let Some(items) = value.as_array() {
        if category == Category::Messages {
            let nested: Vec<_> = items
                .iter()
                .filter_map(|item| item["dmConversation"]["messages"].as_array())
                .flatten()
                .collect();
            if !nested.is_empty() {
                return Some(nested);
            }
        }
        return Some(items.iter().collect());
    }
    let keys: &[&str] = match category {
        Category::Posts => &["posts", "tweets", "your_posts", "items"],
        Category::Saved => &["saved_saved_media", "saved_posts", "saved_items", "items"],
        Category::Likes => &["likes", "items"],
        Category::Bookmarks => &["bookmarks", "items"],
        Category::Listening => &["streamingHistory", "history", "items"],
        Category::Playlists => &["playlists"],
        Category::Contacts => &["contacts", "connections", "contacts_contact_info"],
        Category::Messages => &["messages", "conversations"],
        Category::Media => &["media", "items"],
    };
    keys.iter()
        .find_map(|key| value[*key].as_array().map(|items| items.iter().collect()))
        .or_else(|| value.is_object().then_some(vec![value]))
}

fn parse_json(
    provider: Provider,
    category: Category,
    source: &str,
    text: &str,
    assignment: bool,
) -> Result<Vec<Record>, String> {
    let text = if assignment {
        let (name, body) = text
            .split_once('=')
            .ok_or("The X data file does not contain an exported assignment.")?;
        let name = name
            .trim()
            .strip_prefix("window.YTD.")
            .ok_or("The X data assignment is not supported.")?;
        let (dataset, part) = name
            .split_once('.')
            .ok_or("The X data assignment is not supported.")?;
        let expected: &[&str] = match category {
            Category::Posts => &["tweets", "tweet"],
            Category::Likes => &["like", "likes"],
            Category::Bookmarks => &["bookmarks", "bookmark"],
            Category::Messages => &["direct_messages", "direct_messages_group"],
            _ => &[],
        };
        if !expected.contains(&dataset)
            || !part.strip_prefix("part").is_some_and(|value| {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err("The X data assignment does not match this category.".into());
        }
        body.trim().strip_suffix(';').unwrap_or(body.trim())
    } else {
        text
    };
    let value: Value = serde_json::from_str(text).map_err(|_| "The export is not valid JSON.")?;
    let rows = json_rows(&value, category)
        .ok_or("The JSON schema is not recognized for this category.")?;
    let count = rows.len();
    let result: Vec<_> = rows
        .into_iter()
        .take(20_000)
        .filter_map(|value| json_record(provider, category, value, source))
        .collect();
    if count != 0 && result.is_empty() {
        return Err("The JSON schema is not recognized for this category.".into());
    }
    if count > 20_000 {
        return Err(
            "The export has more than 20000 records in one file. Choose a narrower export.".into(),
        );
    }
    Ok(result)
}

fn parse_csv(category: Category, source: &str, text: &str) -> Result<Vec<Record>, String> {
    let rows = delimited::parse(text, delimited::delimiter(text), 20_010, 80)?;
    let header_at = rows
        .iter()
        .take(8)
        .position(|row| {
            row.iter().any(|field| {
                matches!(
                    field.trim().to_ascii_lowercase().as_str(),
                    "first name"
                        | "last name"
                        | "from"
                        | "sender"
                        | "sharelink"
                        | "sharecommentary"
                        | "commentary"
                        | "title"
                        | "content"
                        | "trackname"
                        | "track name"
                        | "ms played"
                        | "msplayed"
                )
            })
        })
        .ok_or("The CSV headers do not match a supported export.")?;
    let headers = rows[header_at]
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut records = Vec::new();
    for row in &rows[header_at + 1..] {
        let get = |keys: &[&str], limit: usize| {
            keys.iter().find_map(|key| {
                headers
                    .iter()
                    .position(|header| header == key)
                    .and_then(|index| row.get(index))
                    .and_then(|value| clean(value, limit))
            })
        };
        let mut record = empty(source);
        record.title = if category == Category::Contacts {
            [get(&["first name"], 128), get(&["last name"], 128)]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            get(&["title", "subject", "trackname", "track name"], 256).unwrap_or_default()
        };
        record.text = if category == Category::Contacts {
            [
                get(&["position"], 256),
                get(&["company"], 256),
                get(&["email address"], 256),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
        } else {
            get(
                &[
                    "content",
                    "sharecommentary",
                    "commentary",
                    "description",
                    "message",
                ],
                12000,
            )
            .unwrap_or_default()
        };
        record.author = get(&["from", "sender", "artist", "artistname"], 256);
        record.link = get(&["url", "sharelink", "link"], 2048)
            .as_deref()
            .and_then(public_link);
        record.time = get(
            &[
                "date",
                "shared date",
                "connected on",
                "created at",
                "endtime",
            ],
            80,
        )
        .as_deref()
        .and_then(time_string);
        record.played_ms =
            get(&["ms played", "msplayed", "ms_played"], 32).and_then(|value| value.parse().ok());
        if record.title.is_empty() {
            record.title = clean(&record.text, 160)
                .or_else(|| record.author.clone())
                .unwrap_or_default();
        }
        if !record.title.is_empty() || record.link.is_some() || record.played_ms.is_some() {
            records.push(finish(record, get(&["id", "message id", "shareid"], 192)));
        }
    }
    Ok(records)
}

fn parse_html(source: &str, text: &str) -> Result<Vec<Record>, String> {
    static ACTIVE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<(?:script|style|iframe|object)\b[^>]*>.*?(?:</(?:script|style|iframe|object)\s*>|$)").unwrap()
    });
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)\bhref\s*=\s*["']([^"']{1,2048})["']"#).unwrap());
    let safe = ACTIVE.replace_all(text, "");
    let body = crate::public_sources::clean_text(&safe, 12000);
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut record = empty(source);
    record.title = Path::new(source)
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(|value| clean(value, 256))
        .unwrap_or_else(|| "Exported document".into());
    record.text = body;
    record.link = LINK
        .captures_iter(&safe)
        .find_map(|capture| public_link(capture.get(1)?.as_str()));
    Ok(vec![finish(record, None)])
}

pub fn parse(
    provider: Provider,
    classification: Classification,
    source: &str,
    text: &str,
) -> Result<Vec<Record>, String> {
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = match classification.format {
        Format::Json | Format::XAssignment => parse_json(
            provider,
            classification.category,
            source,
            text,
            classification.format == Format::XAssignment,
        ),
        Format::Csv => parse_csv(classification.category, source, text),
        Format::Html => parse_html(source, text),
        _ => Err("This export format is not supported for this category.".into()),
    }?;
    if classification.category == Category::Listening {
        // Streaming History can repeat the same title and millisecond count within one minute.
        for (position, row) in rows.iter_mut().enumerate() {
            row.id = format!(
                "{:x}",
                Sha256::digest(format!("{source}\n{position}\n{}", row.id))
            );
        }
    }
    Ok(rows)
}

pub fn media_record(source: &str) -> Record {
    let mut record = empty(source);
    record.title = source
        .rsplit('/')
        .next()
        .unwrap_or(source)
        .chars()
        .take(256)
        .collect();
    record.media = Some(source.into());
    finish(record, Some(source.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_assignments_are_parsed_as_json_and_sensitive_media_stays_separate() {
        let class = classify(Provider::X, "data/tweets.js").unwrap();
        let text = r#"window.YTD.tweets.part0 = [{"tweet":{"id_str":"100","full_text":"A post","in_reply_to_status_id_str":"99","created_at":"Wed Jan 01 01:23:45 +0000 2020"}}];"#;
        let posts = parse(Provider::X, class, "data/tweets.js", text).unwrap();
        assert_eq!(posts[0].text, "A post");
        assert_eq!(posts[0].thread_parent.as_deref(), Some("99"));
        assert!(posts[0].time.is_some());
        assert!(parse(
            Provider::X,
            class,
            "data/tweets.js",
            &format!("{text}alert(1)")
        )
        .is_err());
        assert_eq!(
            classify(Provider::X, "data/direct_messages_media/1.jpg")
                .unwrap()
                .category,
            Category::Messages
        );
        assert!(classify(Provider::Instagram, "messages/session_cookies.json").is_none());
    }

    #[test]
    fn spotify_counts_time_and_reports_completion_only_when_present() {
        let class = classify(Provider::Spotify, "Streaming_History_Audio_2024.json").unwrap();
        let records = parse(Provider::Spotify, class, "history", r#"[{"ts":"2024-01-01T09:00:00Z","ms_played":1240,"master_metadata_track_name":"Track","reason_end":"trackdone"},{"endTime":"2024-01-02 08:00","msPlayed":200,"trackName":"Sample"}]"#).unwrap();
        assert_eq!(records[0].played_ms, Some(1240));
        assert_eq!(records[0].reached_end, Some(true));
        assert_eq!(records[0].completed, None);
        assert_eq!(records[1].completed, None);
        assert!(records.iter().all(|record| record.time.is_some()));
    }

    #[test]
    fn repeated_spotify_events_keep_distinct_stable_ids() {
        let source = "StreamingHistory0.json";
        let class = classify(Provider::Spotify, source).unwrap();
        let event = r#"{"endTime":"2024-01-02 08:00","msPlayed":200,"trackName":"Sample"}"#;
        let text = format!("[{event},{event}]");
        let rows = parse(Provider::Spotify, class, source, &text).unwrap();
        assert_ne!(rows[0].id, rows[1].id);
        assert_eq!(
            parse(Provider::Spotify, class, source, &text).unwrap(),
            rows
        );
    }

    #[test]
    fn linkedin_csv_and_meta_html_are_parsed_without_active_content() {
        let source = "Connections.csv";
        let class = classify(Provider::Linkedin, source).unwrap();
        let rows = parse(Provider::Linkedin, class, source, "Notes:\n\nFirst Name,Last Name,URL,Email Address,Company,Position,Connected On\nExample,Person,https://www.linkedin.com/in/example,example@example.test,Test Co,Designer,08 Jan 2025\n").unwrap();
        assert_eq!(rows[0].title, "Example Person");
        assert!(rows[0].text.contains("example@example.test"));
        assert_eq!(rows[0].time, Some(1736294400000));
        let source = "saved_posts.html";
        let class = classify(Provider::Instagram, source).unwrap();
        let rows = parse(Provider::Instagram, class, source, r#"<script>privateScriptBody()</script><a href="https://example.org/file?session=1">Private</a><a href="https://example.org/post">A saved post</a>"#).unwrap();
        assert!(!rows[0].text.contains("privateScriptBody"));
        assert_eq!(rows[0].link.as_deref(), Some("https://example.org/post"));
    }
}
