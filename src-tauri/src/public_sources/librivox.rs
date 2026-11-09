use super::{
    api_bytes, archive_asset, archive_restricted, field, hex_digest, identifier, mime_for_name,
    next_page, numeric_page, page, percent_decode, safe_download_url, url_path, Asset,
    AudiobookChapter, Entry, Page, Provider, Target, PAGE_SIZE, PUBLIC_NOT_FOUND,
};
use crate::podcasts::{parse_feed, Episode, Feed};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};
use url::Url;

const ORIGIN: &str = "https://librivox.org";
const CATALOG: &str = "https://librivox.org/api/feed/audiobooks";
pub(super) const ATTRIBUTION_SOURCE: &str = "Source: ";
const SEARCH: &str = "https://librivox.org/search/";
const RIGHTS: &str = "Public domain in the United States (LibriVox). Check the status in your country: https://librivox.org/pages/public-domain/";
const SEARCH_SIZE: usize = 20;
pub(super) const LANGUAGE_PAGE_SIZE: usize = 25;
const CACHE_LIMIT: usize = 8;
const MAX_CACHED_RESPONSE: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LibrivoxTarget {
    Edition(u64),
    CatalogPage(String),
    Language(u64),
    Search {
        title: String,
        language: Option<String>,
    },
}

fn positive_id(input: &str) -> Option<u64> {
    input
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| input.parse::<u64>().ok())
        .flatten()
        .filter(|id| *id > 0)
}

fn numeric(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn search_value(input: &str, maximum: usize) -> bool {
    !input.is_empty() && input.len() <= maximum && !input.chars().any(char::is_control)
}

pub(super) fn parse(url: &Url, parts: &[String]) -> Result<LibrivoxTarget, String> {
    use LibrivoxTarget::*;
    match parts {
        [search] if search == "search" => {
            let mut title = None;
            let mut language = None;
            let mut language_id = None;
            for (name, value) in url.query_pairs() {
                let value = value.trim();
                match name.as_ref() {
                    "title" if title.is_none() && search_value(value, 200) => {
                        title = Some(value.to_owned())
                    }
                    "language" if language.is_none() && search_value(value, 80) => {
                        language = Some(value.to_owned())
                    }
                    "language_id" if language_id.is_none() => {
                        language_id = Some(positive_id(value).ok_or("Invalid LibriVox language ID.")?)
                    }
                    _ => {
                        return Err(
                            "Choose a LibriVox catalog language or enter a title.".into()
                        )
                    }
                }
            }
            match (title, language, language_id) {
                (None, None, Some(id)) => Ok(Language(id)),
                (Some(title), language, None) => Ok(Search { title, language }),
                _ => Err("Choose a LibriVox catalog language or enter a title.".into()),
            }
        }
        [rss, id] if rss == "rss" => positive_id(id)
            .map(Edition)
            .ok_or_else(|| "Invalid LibriVox edition ID.".into()),
        [api, feed, books] if api == "api" && feed == "feed" && books == "audiobooks" => url
            .query_pairs()
            .find(|(name, _)| name == "id")
            .and_then(|(_, value)| positive_id(&value))
            .map(Edition)
            .ok_or_else(|| "Enter a LibriVox edition ID or search by title.".into()),
        [api, feed, books, key, id]
            if api == "api" && feed == "feed" && books == "audiobooks" && key == "id" =>
        {
            positive_id(id)
                .map(Edition)
                .ok_or_else(|| "Invalid LibriVox edition ID.".into())
        }
        [slug]
            if slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && identifier(slug) =>
        {
            Ok(CatalogPage(slug.clone()))
        }
        _ => Err("Use a LibriVox catalog page, edition RSS link or catalog search.".into()),
    }
}

pub(super) fn canonical(target: &LibrivoxTarget) -> String {
    match target {
        LibrivoxTarget::Edition(id) => format!("{CATALOG}/id/{id}"),
        LibrivoxTarget::CatalogPage(slug) => format!("{ORIGIN}/{slug}/"),
        LibrivoxTarget::Language(id) => format!("{SEARCH}?language_id={id}"),
        LibrivoxTarget::Search { title, language } => {
            let mut url = Url::parse(SEARCH).expect("constant catalog URL");
            url.query_pairs_mut().append_pair("title", title);
            if let Some(language) = language {
                url.query_pairs_mut().append_pair("language", language);
            }
            url.to_string()
        }
    }
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(String, Instant, Arc<[u8]>)>,
    next_catalog_request: Option<Instant>,
}

async fn request(url: &str, accept: &str) -> Result<Arc<[u8]>, String> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().await;
    let now = Instant::now();
    cache.entries.retain(|(_, expiry, _)| *expiry > now);
    if let Some((_, _, bytes)) = cache.entries.iter().find(|(key, _, _)| key == url) {
        return Ok(bytes.clone());
    }
    let librivox = url.starts_with("https://librivox.org/");
    if librivox {
        if let Some(next) = cache.next_catalog_request {
            tokio::time::sleep_until(next).await;
        }
        cache.next_catalog_request = Some(Instant::now() + Duration::from_secs(3));
    }
    let response: Arc<[u8]> = api_bytes(url, accept).await?.into();
    if response.len() <= MAX_CACHED_RESPONSE {
        if cache.entries.len() == CACHE_LIMIT {
            cache.entries.pop_front();
        }
        let ttl = Duration::from_secs(if librivox { 120 } else { 30 });
        cache
            .entries
            .push_back((url.to_owned(), Instant::now() + ttl, response.clone()));
    }
    Ok(response)
}

async fn json(url: &str) -> Result<Value, String> {
    let bytes = request(url, "application/json").await?;
    serde_json::from_slice(&bytes)
        .map_err(|_| "LibriVox or its archive returned invalid metadata JSON.".into())
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CatalogLanguage {
    id: u64,
    name: String,
}

fn catalog_text(value: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    let value = TAGS.get_or_init(|| Regex::new(r"<[^>]*>").unwrap()).replace_all(value, " ");
    let value = value.replace("&nbsp;", " ");
    let text = quick_xml::escape::unescape(&value).unwrap_or(std::borrow::Cow::Borrowed(&value));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_languages(html: &str) -> Result<Vec<CatalogLanguage>, String> {
    static SELECT: OnceLock<Regex> = OnceLock::new();
    static OPTIONS: OnceLock<Regex> = OnceLock::new();
    let select = SELECT.get_or_init(|| Regex::new(r#"(?is)<select\b[^>]*\s+id="recorded_language"[^>]*>(.*?)</select>"#).unwrap());
    let options = OPTIONS.get_or_init(|| Regex::new(r#"(?is)<option\b[^>]*\s+value="(\d+)"[^>]*>([^<]+)</option>"#).unwrap());
    let body = select.captures(html).and_then(|value| value.get(1))
        .ok_or("LibriVox did not return its language list.")?;
    let mut known = HashSet::new();
    let languages: Vec<_> = options.captures_iter(body.as_str()).filter_map(|value| {
        let id = positive_id(&value[1])?;
        let name = catalog_text(&value[2]);
        (search_value(&name, 80) && known.insert(id)).then_some(CatalogLanguage { id, name })
    }).take(256).collect();
    if languages.is_empty() {
        return Err("LibriVox did not return any catalog languages.".into());
    }
    Ok(languages)
}

pub async fn catalog_languages() -> Result<Vec<CatalogLanguage>, String> {
    let html = request("https://librivox.org/search", "text/html").await?;
    parse_languages(std::str::from_utf8(&html).map_err(|_| "The LibriVox language list is not UTF-8.")?)
}

fn language_results(language: &CatalogLanguage, payload: &Value, current: usize) -> Result<Page, String> {
    static TITLE: OnceLock<Regex> = OnceLock::new();
    static AUTHOR: OnceLock<Regex> = OnceLock::new();
    static PAGE: OnceLock<Regex> = OnceLock::new();
    let title = TITLE.get_or_init(|| Regex::new(r#"(?is)<h3>\s*<a\s+href="([^"]+)"[^>]*>(.*?)</a>\s*</h3>"#).unwrap());
    let author = AUTHOR.get_or_init(|| Regex::new(r#"(?is)<p class="book-author">\s*<a\s[^>]*>([^<]+)"#).unwrap());
    let page_number = PAGE.get_or_init(|| Regex::new(r#"data-page_number="(\d+)""#).unwrap());
    let results = payload["results"].as_str().ok_or("LibriVox did not return a catalog page.")?;
    let mut result = page(
        &Target::Librivox(LibrivoxTarget::Language(language.id)),
        Provider::Librivox,
        format!("LibriVox · {}", language.name),
    );
    result.rights = Some(RIGHTS.into());
    let mut seen = HashSet::new();
    for row in results.split(r#"<li class="catalog-result">"#).skip(1).take(LANGUAGE_PAGE_SIZE) {
        let Some(value) = title.captures(row) else { continue; };
        let Ok(Target::Librivox(LibrivoxTarget::CatalogPage(slug))) = super::target(&value[1]) else { continue; };
        if !seen.insert(slug.clone()) { continue; }
        // The language catalog also lists chapter matches inside an edition.
        let title = &value[2];
        let edition_title = title.strip_suffix(')')
            .and_then(|value| value.rsplit_once(" (in  ").map(|(_, edition)| edition))
            .unwrap_or(title);
        let name: String = catalog_text(edition_title).chars().take(240).collect();
        if name.is_empty() { continue; }
        let author = author.captures(row).map(|value| catalog_text(&value[1])).filter(|value| !value.is_empty());
        result.entries.push(Entry {
            id: format!("librivox:catalog:{slug}"),
            url: canonical(&LibrivoxTarget::CatalogPage(slug)),
            title: match author {
                Some(author) => format!("{name} · {}", author.chars().take(160).collect::<String>()),
                None => name,
            },
            prerelease: false,
            published_at: None,
        });
    }
    if payload["status"] == "SUCCESS" && result.entries.is_empty() {
        return Err("LibriVox returned a catalog layout that could not be read.".into());
    }
    let more = page_number.captures_iter(payload["pagination"].as_str().unwrap_or_default())
        .any(|value| value[1].parse::<usize>().is_ok_and(|page| page > current));
    next_page(&mut result, current, more);
    Ok(result)
}

async fn browse_language(id: u64, current: usize) -> Result<Page, String> {
    let languages = catalog_languages().await?;
    let language = languages.iter().find(|language| language.id == id)
        .ok_or("That language is no longer listed in the LibriVox catalog.")?;
    let mut url = Url::parse("https://librivox.org/search/get_results").expect("constant catalog endpoint");
    url.query_pairs_mut()
        .append_pair("primary_key", &id.to_string())
        .append_pair("search_category", "language")
        .append_pair("search_page", &current.to_string())
        .append_pair("search_order", "catalog_date");
    let payload = json(url.as_str()).await?;
    language_results(language, &payload, current)
}

fn people(values: &Value, name: impl Fn(&Value) -> Option<String>) -> Option<String> {
    let people = values
        .as_array()?
        .iter()
        .filter_map(name)
        .take(20)
        .collect::<Vec<_>>()
        .join(", ");
    (!people.is_empty()).then(|| people.chars().take(1000).collect())
}

fn authors(book: &Value) -> Option<String> {
    people(&book["authors"], |person| {
        let name = [
            field(person, "first_name", 100),
            field(person, "last_name", 100),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        (!name.is_empty()).then_some(name)
    })
}

fn books(payload: &Value) -> Result<&[Value], String> {
    payload["books"]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| "LibriVox did not return a book list.".into())
}

fn search_results(
    target: &LibrivoxTarget,
    title: &str,
    language: Option<&str>,
    books: &[Value],
    current: usize,
) -> Page {
    let mut result = page(
        &Target::Librivox(target.clone()),
        Provider::Librivox,
        format!("LibriVox · {title}"),
    );
    result.rights = Some(RIGHTS.into());
    for book in books.iter().take(SEARCH_SIZE) {
        let book_language = field(book, "language", 80);
        if language.is_some_and(|wanted| {
            !book_language
                .as_ref()
                .is_some_and(|actual| actual.to_lowercase() == wanted.to_lowercase())
        }) {
            continue;
        }
        let Some(id) = numeric(&book["id"]).filter(|id| *id > 0) else {
            continue;
        };
        let Some(title) = field(book, "title", 240) else {
            continue;
        };
        let details = [
            Some(title),
            authors(book),
            book_language,
            Some(format!("#{id}")),
        ];
        result.entries.push(Entry {
            id: format!("librivox:{id}"),
            title: details
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
            url: canonical(&LibrivoxTarget::Edition(id)),
            prerelease: false,
            published_at: None,
        });
    }
    next_page(&mut result, current, books.len() > SEARCH_SIZE);
    if language.is_some() && result.entries.is_empty() && result.next_cursor.is_some() {
        result.warnings.push("This page has no editions in the selected language. Load the next catalog page or change the search.".into());
    }
    result
}

fn catalog_edition(html: &str) -> Option<u64> {
    static RSS_LINK: OnceLock<regex::Regex> = OnceLock::new();
    let regex = RSS_LINK.get_or_init(|| regex::Regex::new(r#"(?i)\bhref\s*=\s*[\x22\x27]((?:https?://(?:www\.)?librivox\.org)?/rss/([0-9]+))/?[\x22\x27]"#).expect("constant RSS link pattern"));
    let mut ids = regex
        .captures_iter(html)
        .filter_map(|hit| positive_id(&hit[2]));
    let id = ids.next()?;
    ids.all(|other| id == other).then_some(id)
}

fn parsed_link(input: &str) -> Option<Url> {
    let mut url = Url::parse(&crate::model::validate_url(input).ok()?).ok()?;
    if url.port().is_some() || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    url.set_scheme("https").ok()?;
    Some(url)
}

fn archive_item(link: &str, route: &str) -> Option<(String, Vec<String>)> {
    let link = parsed_link(link)?;
    if !matches!(link.host_str(), Some("archive.org" | "www.archive.org")) {
        return None;
    }
    let mut parts = link.path_segments()?.filter(|part| !part.is_empty());
    if parts.next()? != route {
        return None;
    }
    let id = percent_decode(parts.next()?).ok()?;
    if !identifier(&id) {
        return None;
    }
    let file = parts
        .map(percent_decode)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if file
        .iter()
        .any(|part| part.contains(['/', '\\']) || matches!(part.as_str(), "." | ".."))
    {
        return None;
    }
    Some((id, file))
}

fn archive_identifier(book: &Value, feed: Option<&Feed>) -> Option<String> {
    book["url_iarchive"]
        .as_str()
        .and_then(|url| archive_item(url, "details"))
        .filter(|(_, path)| path.is_empty())
        .or_else(|| {
            book["url_zip_file"]
                .as_str()
                .and_then(|url| archive_item(url, "download"))
        })
        .map(|(id, _)| id)
        .or_else(|| {
            let sections = book["sections"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|section| section["listen_url"].as_str());
            let enclosures = feed
                .into_iter()
                .flat_map(|feed| &feed.episodes)
                .map(|episode| episode.enclosure.as_str());
            let mut ids = sections
                .chain(enclosures)
                .filter_map(|link| archive_item(link, "download").map(|(id, _)| id));
            let id = ids.next()?;
            ids.all(|other| other == id).then_some(id)
        })
}

fn audio_upload(input: &str) -> Option<(String, String)> {
    let mut url = parsed_link(input)?;
    if !matches!(url.host_str(), Some("librivox.org" | "www.librivox.org")) {
        return None;
    }
    let parts = url
        .path_segments()?
        .map(percent_decode)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if parts.len() < 3
        || parts[0] != "uploads"
        || parts
            .iter()
            .any(|part| part.is_empty() || part.contains(['/', '\\']))
    {
        return None;
    }
    let name = parts.last()?;
    if !mime_for_name(name).starts_with("audio/") {
        return None;
    }
    url.set_host(Some("librivox.org")).ok()?;
    Some((
        safe_download_url(url.as_str(), Provider::Librivox).ok()?,
        name.to_owned(),
    ))
}

async fn rss(id: u64, book: &Value) -> Option<Feed> {
    let link = parsed_link(book["url_rss"].as_str()?)?;
    if !matches!(link.host_str(), Some("librivox.org" | "www.librivox.org"))
        || link.path() != format!("/rss/{id}")
    {
        return None;
    }
    let url = format!("{ORIGIN}/rss/{id}");
    let bytes = request(&url, "application/rss+xml, application/xml")
        .await
        .ok()?;
    parse_feed(std::str::from_utf8(&bytes).ok()?, &url).ok()
}

struct Chapter {
    id: String,
    number: u32,
    title: String,
    reader: Option<String>,
    language: Option<String>,
    link: Option<String>,
    original_name: Option<String>,
    episode: Option<Episode>,
}

fn chapters(book: &Value, feed: Option<&Feed>) -> Vec<Chapter> {
    let sections = book["sections"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let episodes = feed
        .map(|feed| feed.episodes.as_slice())
        .unwrap_or_default();
    if sections.is_empty() {
        return episodes
            .iter()
            .take(PAGE_SIZE * super::MAX_PAGES)
            .enumerate()
            .map(|(index, episode)| Chapter {
                id: episode.id.clone(),
                number: index as u32 + 1,
                title: episode.title.clone(),
                reader: None,
                language: None,
                link: Some(episode.enclosure.clone()),
                original_name: None,
                episode: Some(episode.clone()),
            })
            .collect();
    }
    let mut titles = HashMap::new();
    for title in sections
        .iter()
        .filter_map(|section| field(section, "title", 240))
    {
        *titles.entry(title).or_insert(0) += 1;
    }
    let mut episode_urls = HashMap::new();
    let mut episode_titles = HashMap::new();
    for episode in episodes {
        if let Some(url) = parsed_link(&episode.enclosure) {
            episode_urls.entry(url).or_insert(episode);
        }
        episode_titles
            .entry(episode.title.trim())
            .and_modify(|value| *value = None)
            .or_insert(Some(episode));
    }
    let mut ids = HashSet::new();
    let mut chapters = sections
        .iter()
        .enumerate()
        .filter_map(|(index, section)| {
            let id = numeric(&section["id"]).filter(|id| *id > 0)?;
            if !ids.insert(id) {
                return None;
            }
            let number = numeric(&section["section_number"])
                .and_then(|number| u32::try_from(number).ok())
                .unwrap_or(index as u32 + 1);
            let title = field(section, "title", 240).unwrap_or_else(|| format!("Chapter {number}"));
            let link = section["listen_url"].as_str().map(str::to_owned);
            let episode = link
                .as_deref()
                .and_then(parsed_link)
                .and_then(|link| episode_urls.get(&link).copied())
                .or_else(|| {
                    if titles.get(&title) != Some(&1) {
                        return None;
                    }
                    episode_titles.get(title.as_str()).copied().flatten()
                })
                .cloned();
            Some(Chapter {
                id: id.to_string(),
                number,
                title,
                reader: people(&section["readers"], |reader| {
                    field(reader, "display_name", 200)
                }),
                language: field(section, "language", 80),
                link,
                original_name: section["file_name"].as_str().map(str::to_owned),
                episode,
            })
        })
        .collect::<Vec<_>>();
    chapters.sort_by_key(|chapter| chapter.number);
    chapters
}

struct Archive {
    target: Target,
    metadata: Value,
}

impl Archive {
    fn id(&self) -> &str {
        let Target::Archive { identifier } = &self.target else {
            unreachable!("archive target")
        };
        identifier
    }

    fn asset(&self, file: &Value) -> Result<Option<Asset>, String> {
        archive_asset(file, &self.metadata["metadata"], &self.target)
    }
}

async fn book_archive(book: &Value, feed: Option<&Feed>) -> Result<Option<Archive>, String> {
    let Some(identifier) = archive_identifier(book, feed) else {
        return Ok(None);
    };
    let url = url_path("https://archive.org", &["metadata", &identifier]);
    let metadata = json(&url).await?;
    if archive_restricted(&metadata)
        || !metadata["files"].is_array()
        || !metadata["metadata"].is_object()
    {
        return Err("This edition's Internet Archive files are unavailable or restricted.".into());
    }
    Ok(Some(Archive {
        target: Target::Archive { identifier },
        metadata,
    }))
}

fn chapter_asset(
    chapter: Chapter,
    book: &Value,
    id: u64,
    book_title: &str,
    archive: Option<&Archive>,
    files: &HashMap<&str, &Value>,
) -> Result<Option<Asset>, String> {
    let chapter_id = format!("librivox:{id}:{}", chapter.id);
    let source_url = canonical(&LibrivoxTarget::Edition(id));
    let links = [
        chapter.link.as_deref(),
        chapter
            .episode
            .as_ref()
            .map(|episode| episode.enclosure.as_str()),
    ];
    let mut archived = None;
    if let Some(archive) = archive {
        let paths = links
            .into_iter()
            .flatten()
            .filter_map(|link| archive_item(link, "download"))
            .filter(|(id, _)| id == archive.id())
            .map(|(_, parts)| parts.join("/"));
        for path in paths.chain(chapter.original_name.iter().cloned()) {
            if let Some(file) = files
                .get(path.as_str())
                .filter(|_| mime_for_name(&path).starts_with("audio/"))
            {
                let Some(mut asset) = archive.asset(file)? else {
                    return Ok(None);
                };
                asset.checksum = file["md5"]
                    .as_str()
                    .and_then(|digest| hex_digest(digest, 32))
                    .map(|digest| format!("md5:{digest}"));
                archived = Some(asset);
                break;
            }
        }
    }
    let mut asset = if let Some(asset) = archived {
        asset
    } else {
        let Some((download_url, filename)) = links.into_iter().flatten().find_map(audio_upload)
        else {
            return Ok(None);
        };
        let mime = mime_for_name(&filename).into();
        let version = format!("url-sha256:{:x}", Sha256::digest(download_url.as_bytes()));
        let episode = chapter.episode.as_ref().filter(|episode| {
            audio_upload(&episode.enclosure).is_some_and(|(link, _)| link == download_url)
        });
        Asset {
            id: String::new(),
            source_url: String::new(),
            download_url,
            filename,
            mime,
            size: episode.and_then(|episode| episode.size),
            title: String::new(),
            version: Some(version),
            original: true,
            author: None,
            license: None,
            attribution: None,
            sha256: None,
            platform: None,
            architecture: None,
            published_at: None,
            checksum: None,
            audiobook: None,
        }
    };
    asset.id = chapter_id;
    asset.source_url = source_url;
    let extension = asset
        .filename
        .rsplit('.')
        .next()
        .expect("an audio filename has an extension");
    let section_id = chapter.id.replace(':', "-");
    asset.filename = format!(
        "librivox-{id}-{:04}-{section_id}.{extension}",
        chapter.number
    );
    crate::direct::validate_filename(&asset.filename)?;
    asset.title = chapter.title;
    asset.author = authors(book);
    let upstream = asset.license.take();
    asset.license = Some(RIGHTS.into());
    let catalog_url = book["url_librivox"]
        .as_str()
        .and_then(parsed_link)
        .filter(|url| matches!(url.host_str(), Some("librivox.org" | "www.librivox.org")));
    let mut attribution = format!(
        "LibriVox edition #{id}\nBook: {book_title}\n{ATTRIBUTION_SOURCE}{}\nRights: {RIGHTS}",
        catalog_url
            .as_ref()
            .map(Url::as_str)
            .unwrap_or(&asset.source_url)
    );
    if let Some(upstream) = upstream {
        attribution.push_str(&format!("\nArchive rights metadata: {upstream}"));
    }
    asset.attribution = Some(attribution);
    asset.published_at = chapter
        .episode
        .and_then(|episode| episode.published_at)
        .and_then(|ms| {
            std::time::UNIX_EPOCH
                .checked_add(Duration::from_millis(ms))
                .map(httpdate::fmt_http_date)
        });
    asset.audiobook = Some(AudiobookChapter {
        book_id: format!("librivox:{id}"),
        book_title: book_title.to_owned(),
        chapter: chapter.number,
        reader: chapter.reader,
        language: chapter.language.or_else(|| field(book, "language", 80)),
    });
    Ok(Some(asset))
}

async fn edition(
    id: u64,
    expected_slug: Option<&str>,
    selected: Option<&str>,
) -> Result<Page, String> {
    let url = format!("{CATALOG}?id={id}&extended=1&format=json&limit=1");
    let payload = json(&url).await?;
    let book = books(&payload)?
        .iter()
        .find(|book| numeric(&book["id"]) == Some(id))
        .ok_or("LibriVox did not return the selected edition.")?;
    if let Some(slug) = expected_slug {
        let matches = book["url_librivox"]
            .as_str()
            .and_then(parsed_link)
            .is_some_and(|url| {
                matches!(url.host_str(), Some("librivox.org" | "www.librivox.org"))
                    && url.path().trim_matches('/') == slug
            });
        if !matches {
            return Err("The LibriVox catalog page and edition do not match.".into());
        }
    }
    let title = field(book, "title", 240).ok_or("The LibriVox edition has no title.")?;
    let mut result = page(
        &Target::Librivox(LibrivoxTarget::Edition(id)),
        Provider::Librivox,
        title.clone(),
    );
    result.description = field(book, "description", 6000);
    result.rights = Some(RIGHTS.into());
    let feed = rss(id, book).await;
    let archive = book_archive(book, feed.as_ref()).await?;
    let files: HashMap<_, _> = archive
        .as_ref()
        .into_iter()
        .flat_map(|archive| archive.metadata["files"].as_array().into_iter().flatten())
        .filter_map(|file| Some((file["name"].as_str()?, file)))
        .collect();
    let chapters = chapters(book, feed.as_ref());
    let discovered = chapters.len();
    for chapter in chapters.into_iter().take(PAGE_SIZE * super::MAX_PAGES) {
        if selected.is_some_and(|wanted| wanted != format!("librivox:{id}:{}", chapter.id)) {
            continue;
        }
        if let Some(asset) = chapter_asset(chapter, book, id, &title, archive.as_ref(), &files)? {
            result.assets.push(asset);
        }
    }
    if selected.is_none()
        && (result.assets.len() < discovered
            || numeric(&book["num_sections"])
                .is_some_and(|count| (result.assets.len() as u64) < count))
    {
        result.warnings.push(
            "Some chapters have no verified public audio file. Only available chapters are shown."
                .into(),
        );
    }
    if result.assets.is_empty() {
        return Err(if selected.is_some() {
            "The selected LibriVox chapter is no longer available."
        } else {
            "This LibriVox edition has no verified public audio files."
        }
        .into());
    }
    Ok(result)
}

pub(super) async fn inspect(target: &LibrivoxTarget, cursor: Option<&str>) -> Result<Page, String> {
    let current = numeric_page(cursor)?;
    let mut result = match target {
        LibrivoxTarget::Language(id) => return browse_language(*id, current).await,
        LibrivoxTarget::Search { title, language } => {
            let mut url = Url::parse(CATALOG).expect("constant catalog API URL");
            url.query_pairs_mut()
                .append_pair("format", "json")
                .append_pair("title", &format!("^{title}"))
                .append_pair("limit", &(SEARCH_SIZE + 1).to_string())
                .append_pair("offset", &((current - 1) * SEARCH_SIZE).to_string())
                .append_pair("fields", "id,title,language,authors");
            return match json(url.as_str()).await {
                Ok(payload) => Ok(search_results(
                    target,
                    title,
                    language.as_deref(),
                    books(&payload)?,
                    current,
                )),
                Err(error) if error == PUBLIC_NOT_FOUND => Ok(search_results(
                    target,
                    title,
                    language.as_deref(),
                    &[],
                    current,
                )),
                Err(error) => Err(error),
            };
        }
        LibrivoxTarget::Edition(id) => edition(*id, None, None).await?,
        LibrivoxTarget::CatalogPage(slug) => {
            let html = request(&canonical(target), "text/html").await?;
            let id = catalog_edition(
                std::str::from_utf8(&html)
                    .map_err(|_| "The LibriVox catalog page is not UTF-8.")?,
            )
            .ok_or("The LibriVox catalog page has no unambiguous edition RSS link.")?;
            edition(id, Some(slug), None).await?
        }
    };
    let offset = (current - 1) * PAGE_SIZE;
    let more = result.assets.len() > offset + PAGE_SIZE;
    result.assets = result
        .assets
        .into_iter()
        .skip(offset)
        .take(PAGE_SIZE)
        .collect();
    next_page(&mut result, current, more);
    Ok(result)
}

pub(super) async fn resolve(target: &LibrivoxTarget, asset_id: &str) -> Result<Asset, String> {
    let LibrivoxTarget::Edition(id) = target else {
        return Err("Choose a LibriVox edition before downloading chapters.".into());
    };
    edition(*id, None, Some(asset_id))
        .await?
        .assets
        .into_iter()
        .next()
        .ok_or_else(|| "The selected LibriVox chapter is no longer available.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edition_urls_keep_an_unambiguous_catalog_identity() {
        let target = super::super::target("http://www.librivox.org/rss/00065").unwrap();
        assert_eq!(
            super::super::source_url(&target),
            "https://librivox.org/api/feed/audiobooks/id/65"
        );
        assert_eq!(
            catalog_edition(r#"<a href="https://librivox.org/rss/65">RSS</a>"#),
            Some(65)
        );
        assert_eq!(
            catalog_edition(r#"<a href="https://malicious.test/rss/1">RSS</a>"#),
            None
        );
        assert_eq!(
            catalog_edition(r#"<a href='/rss/1'>RSS</a><a href='/rss/2'>RSS</a>"#),
            None
        );
    }

    #[test]
    fn language_filters_the_bounded_catalog_page_and_keeps_next_page() {
        let target = LibrivoxTarget::Search {
            title: "Tales & Fables".into(),
            language: Some("English".into()),
        };
        let books = (1..=SEARCH_SIZE + 1).map(|id| json!({
            "id": id.to_string(), "title": "Tales & Fables", "language": if id == 2 { "English" } else { "French" },
            "authors": [{"first_name":"Jane", "last_name":"Example"}]
        })).collect::<Vec<_>>();
        let result = search_results(&target, "Tales & Fables", Some("english"), &books, 1);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].title,
            "Tales & Fables · Jane Example · English · #2"
        );
        assert_eq!(
            result.entries[0].url,
            "https://librivox.org/api/feed/audiobooks/id/2"
        );
        assert_eq!(result.next_cursor.as_deref(), Some("2"));
    }

    #[test]
    fn official_language_page_keeps_editions_and_an_explicit_next_page() {
        let languages = parse_languages(r#"
            <select id="recorded_language"><option value="5">Spanish</option>
            <option value="-1">All Except English</option><option value="5">Spanish</option>
            <option value="13">Catalan</option></select>
            <select id="unrelated"><option value="9">Arabic</option></select>
        "#).unwrap();
        assert_eq!(languages, [
            CatalogLanguage { id: 5, name: "Spanish".into() },
            CatalogLanguage { id: 13, name: "Catalan".into() },
        ]);
        let payload = json!({
            "status": "SUCCESS",
            "pagination": "<a data-page_number=\"2\">2</a>",
            "results": r#"
                <li class="catalog-result"><h3><a href="https://librivox.org/cuentos/">Prólogo (in  Cuentos &amp; relatos)</a></h3>
                <p class="book-author"><a href="/author/1">Jane Example <span>(1850)</span></a></p></li>
                <li class="catalog-result"><h3><a href="https://librivox.org/cuentos/">Otra sección (in  Cuentos &amp; relatos)</a></h3></li>
                <li class="catalog-result"><h3><a href="https://librivox.org/cuentos-2/">Cuentos (versión 2)</a></h3></li>
                <li class="catalog-result"><h3><a href="https://example.org/cuentos/">Untrusted</a></h3></li>
            "#,
        });
        let result = language_results(&languages[0], &payload, 1).unwrap();
        assert_eq!(result.source_url, "https://librivox.org/search/?language_id=5");
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[0].title, "Cuentos & relatos · Jane Example");
        assert_eq!(result.entries[1].url, "https://librivox.org/cuentos-2/");
        assert_eq!(result.next_cursor.as_deref(), Some("2"));
        assert_eq!(super::super::followed_search_page_size(&result.source_url), Ok(25));
        assert!(super::super::valid_followed_search_entry(
            &result.source_url, &result.entries[0].url, &result.entries[0].id,
        ));
        assert!(!super::super::valid_followed_search_entry(
            &result.source_url, &result.entries[0].url, &result.entries[1].id,
        ));
        assert!(super::super::target("https://librivox.org/search/?language_id=5&title=Other").is_err());
        assert!(language_results(&languages[0], &json!([]), 1).is_err());
    }

    #[test]
    fn archive_chapters_follow_catalog_order_and_preserve_readers_and_identity() {
        let book = json!({"language":"English","url_librivox":"https://librivox.org/tales/","sections":[
            {"id":"44","section_number":"10","title":"Part 10","listen_url":"http://www.archive.org/download/edition/part10.mp3","readers":[{"display_name":"Reader A"},{"display_name":"Reader B"}]},
            {"id":"45","section_number":"2","title":"Part 2","listen_url":"https://archive.org/download/edition/part02.mp3"},
            {"id":"46","section_number":"0","title":"Preface","listen_url":"https://librivox.org/uploads/reader/preface.mp3"}
        ]});
        let archive = Archive {
            target: Target::Archive {
                identifier: "edition".into(),
            },
            metadata: json!({"metadata":{"rights":"Archive rights note"}}),
        };
        let file = json!({"name":"part10.mp3","size":"3000","mtime":"41","md5":"002e4d5b1c2dc7102f01f41c80ea213d"});
        let files = HashMap::from([("part10.mp3", &file)]);
        let chapters = chapters(&book, None);
        assert_eq!(
            chapters
                .iter()
                .map(|chapter| chapter.number)
                .collect::<Vec<_>>(),
            vec![0, 2, 10]
        );
        let asset = chapter_asset(
            chapters.into_iter().last().unwrap(),
            &book,
            65,
            "Tales",
            Some(&archive),
            &files,
        )
        .unwrap()
        .unwrap();
        assert_eq!(asset.id, "librivox:65:44");
        assert_eq!(asset.filename, "librivox-65-0010-44.mp3");
        assert_eq!(
            asset.download_url,
            "https://archive.org/download/edition/part10.mp3"
        );
        assert_eq!(asset.size, Some(3000));
        assert_eq!(asset.version.as_deref(), Some("41"));
        assert_eq!(
            asset.checksum.as_deref(),
            Some("md5:002e4d5b1c2dc7102f01f41c80ea213d")
        );
        assert_eq!(
            asset.audiobook.unwrap().reader.as_deref(),
            Some("Reader A, Reader B")
        );
        assert!(asset.attribution.unwrap().contains("Archive rights note"));

        let restricted = json!({"name":"part02.mp3","private":"true"});
        let blocked = super::chapters(&book, None).remove(1);
        assert!(chapter_asset(
            blocked,
            &book,
            65,
            "Tales",
            Some(&archive),
            &HashMap::from([("part02.mp3", &restricted)])
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn only_verified_rss_and_public_files_enter_the_selection() {
        let book = json!({"language":"English"});
        let feed = parse_feed(r#"<rss><channel><title>Tales</title><item><title>Preface</title><guid>intro</guid><enclosure url="https://librivox.org/uploads/reader/preface.mp3" type="audio/mpeg" length="123"/></item><item><title>Other</title><enclosure url="https://unrelated.test/other.mp3" type="audio/mpeg"/></item></channel></rss>"#, "https://librivox.org/rss/65").unwrap();
        let assets = chapters(&book, Some(&feed))
            .into_iter()
            .filter_map(|chapter| {
                chapter_asset(chapter, &book, 65, "Tales", None, &HashMap::new()).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].size, Some(123));
        assert!(assets[0].id.starts_with("librivox:65:podcast:"));
        assert!(
            audio_upload("https://librivox.org/uploads/reader/private.mp3?token=secret").is_none()
        );
        assert!(archive_item(
            "https://archive.org/download/edition/folder%2fprivate.mp3",
            "download"
        )
        .is_none());
    }
}
