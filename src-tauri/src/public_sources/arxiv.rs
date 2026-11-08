use super::{api_bytes, percent_decode, Asset, Entry, Page, Provider};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::{
    collections::{HashSet, VecDeque},
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};
use url::Url;

const SEARCH_PAGE: usize = 20;
const VERSION_PAGE: u32 = 10;
const MAX_SEARCH_PAGES: usize = 10;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const CACHE_AGE: Duration = Duration::from_secs(5 * 60);
const REQUEST_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identifier {
    paper: String,
    version: Option<u32>,
}

impl Identifier {
    pub(super) fn matches_search_entry(&self, entry_id: &str) -> bool {
        entry_id
            .strip_prefix("arxiv:")
            .and_then(Self::parse)
            .is_some_and(|entry| {
                entry.version.is_some()
                    && entry.paper == self.paper
                    && (self.version.is_none() || self.version == entry.version)
            })
    }

    fn parse(value: &str) -> Option<Self> {
        let (paper, version) = match value.rsplit_once('v') {
            Some((paper, version)) if digits(version) => {
                if version.starts_with('0') {
                    return None;
                }
                (paper, Some(version.parse::<u32>().ok()?))
            }
            _ => (value, None),
        };
        let (date, sequence, length) = if let Some((archive, number)) = paper.split_once('/') {
            if archive.is_empty()
                || archive.len() > 40
                || !archive.as_bytes()[0].is_ascii_alphabetic()
                || !archive
                    .bytes()
                    .all(|b| b.is_ascii_alphabetic() || b"-.".contains(&b))
                || number.len() != 7
                || !digits(number)
            {
                return None;
            }
            (&number[..4], &number[4..], 3..=3)
        } else {
            let (date, sequence) = paper.split_once('.')?;
            (date, sequence, 4..=5)
        };
        if date.len() != 4
            || !digits(date)
            || !(1..=12).contains(&date[2..].parse::<u32>().ok()?)
            || !length.contains(&sequence.len())
            || !digits(sequence)
            || sequence.bytes().all(|b| b == b'0')
        {
            return None;
        }
        Some(Self {
            paper: paper.into(),
            version,
        })
    }

    fn value(&self) -> String {
        match self.version {
            Some(version) => format!("{}v{version}", self.paper),
            None => self.paper.clone(),
        }
    }

    fn url(&self) -> String {
        format!("https://arxiv.org/abs/{}", self.value())
    }

    fn asset_id(&self) -> String {
        format!("arxiv:{}", self.value())
    }

    fn unversioned(&self) -> Self {
        Self {
            paper: self.paper.clone(),
            version: None,
        }
    }
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SearchField {
    Title,
    Author,
    Category,
}

impl SearchField {
    fn name(&self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Author => "author",
            Self::Category => "category",
        }
    }

    fn expression(&self, query: &str) -> String {
        match self {
            Self::Title => format!("ti:\"{query}\""),
            Self::Author => format!("au:\"{query}\""),
            Self::Category => format!("cat:{query}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ArxivTarget {
    Article(Identifier),
    Search { field: SearchField, query: String },
}

pub(super) fn normalize(input: &str) -> Option<String> {
    let input = input.trim();
    let id = match input.split_once(':') {
        Some((prefix, id)) if prefix.eq_ignore_ascii_case("arxiv") => id.trim(),
        _ => input,
    };
    Some(Identifier::parse(id)?.url())
}

fn official_url(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("arxiv.org" | "www.arxiv.org" | "export.arxiv.org")
    ) && matches!(url.scheme(), "https" | "http")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

fn search(field: SearchField, query: &str) -> Result<ArxivTarget, String> {
    let query = query.trim();
    let valid = query.len() <= 200
        && query.chars().any(char::is_alphanumeric)
        && !query
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '\\'))
        && (field != SearchField::Category
            || (query.len() <= 40
                && query
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b))));
    if !valid {
        return Err(
            "Enter a title, author or a category such as cs.AI (up to 200 bytes, without quotes)."
                .into(),
        );
    }
    Ok(ArxivTarget::Search {
        field,
        query: query.to_owned(),
    })
}

pub(super) fn parse(url: &Url, parts: &[String]) -> Result<ArxivTarget, String> {
    let invalid =
        "Enter an arXiv identifier, abstract/PDF link or a title, author or category search.";
    if !official_url(url) {
        return Err(invalid.into());
    }
    match parts {
        [route] if route == "search" => {
            let mut field = None;
            let mut query = None;
            for (key, value) in url.query_pairs() {
                match key.as_ref() {
                    "field" | "searchtype" if field.is_none() => field = Some(value),
                    "query" if query.is_none() => query = Some(value),
                    "field" | "searchtype" | "query" => return Err(invalid.into()),
                    _ => {}
                }
            }
            let field = match field.as_deref() {
                Some("title") => SearchField::Title,
                Some("author") => SearchField::Author,
                Some("category") => SearchField::Category,
                _ => return Err(invalid.into()),
            };
            search(field, &query.ok_or(invalid)?)
        }
        [route, category, scope]
            if route == "list" && matches!(scope.as_str(), "new" | "recent") =>
        {
            search(SearchField::Category, category)
        }
        [route, id @ ..]
            if matches!(route.as_str(), "abs" | "pdf" | "html") && (1..=2).contains(&id.len()) =>
        {
            let id = id.join("/");
            let value = if route == "pdf" {
                id.strip_suffix(".pdf").unwrap_or(&id)
            } else {
                &id
            };
            Identifier::parse(value)
                .map(ArxivTarget::Article)
                .ok_or_else(|| invalid.into())
        }
        _ => Err(invalid.into()),
    }
}

pub(super) fn canonical(target: &ArxivTarget) -> String {
    match target {
        ArxivTarget::Article(id) => id.url(),
        ArxivTarget::Search { field, query } => {
            let mut url = Url::parse("https://arxiv.org/search/").expect("constant arXiv URL");
            url.query_pairs_mut()
                .append_pair("field", field.name())
                .append_pair("query", query);
            url.to_string()
        }
    }
}

struct CachedFeed {
    url: String,
    bytes: Vec<u8>,
    expires: Instant,
}

struct RequestState {
    last_started: Option<Instant>,
    feeds: VecDeque<CachedFeed>,
}

static REQUESTS: Mutex<RequestState> = Mutex::const_new(RequestState {
    last_started: None,
    feeds: VecDeque::new(),
});

async fn fetch(query: &[(&str, &str)]) -> Result<Feed, String> {
    let mut endpoint =
        Url::parse("https://export.arxiv.org/api/query").expect("constant arXiv API");
    endpoint
        .query_pairs_mut()
        .extend_pairs(query.iter().copied());
    let mut requests = REQUESTS.lock().await;
    requests.feeds.retain(|feed| feed.expires > Instant::now());
    if let Some(cached) = requests
        .feeds
        .iter()
        .find(|feed| feed.url == endpoint.as_str())
    {
        return parse_atom(&cached.bytes);
    }
    if let Some(started) = requests.last_started {
        tokio::time::sleep_until(started + REQUEST_INTERVAL).await;
    }
    requests.last_started = Some(Instant::now());
    let bytes = api_bytes(endpoint.as_str(), "application/atom+xml").await?;
    let feed = parse_atom(&bytes)?;
    if bytes.len() <= MAX_CACHE_BYTES {
        while requests.feeds.len() >= 16
            || requests
                .feeds
                .iter()
                .map(|feed| feed.bytes.len())
                .sum::<usize>()
                + bytes.len()
                > MAX_CACHE_BYTES
        {
            requests.feeds.pop_front();
        }
        requests.feeds.push_back(CachedFeed {
            url: endpoint.to_string(),
            bytes,
            expires: Instant::now() + CACHE_AGE,
        });
    }
    Ok(feed)
}

struct Article {
    id: Identifier,
    title: String,
    summary: String,
    authors: Vec<String>,
    date: Option<String>,
    license: Option<String>,
    pdf: Option<String>,
}

impl Article {
    fn entry(&self, url: String) -> Entry {
        Entry {
            id: self.id.asset_id(),
            title: self.title.clone(),
            url,
            prerelease: false,
            published_at: self.date.clone(),
        }
    }

    fn asset(&self) -> Option<Asset> {
        let version = self.id.version?;
        Some(Asset {
            id: self.id.asset_id(),
            source_url: self.id.url(),
            download_url: self.pdf.clone()?,
            filename: format!("arxiv-{}.pdf", self.id.value().replace('/', "-")),
            mime: "application/pdf".into(),
            size: None,
            title: self.title.clone(),
            version: Some(format!("v{version}")),
            original: true,
            author: (!self.authors.is_empty()).then(|| self.authors.join(", ")),
            license: self.license.clone(),
            attribution: None,
            sha256: None,
            platform: None,
            architecture: None,
            published_at: self.date.clone(),
            checksum: None,
            audiobook: None,
        })
    }
}

#[derive(Default)]
struct RawArticle {
    id: String,
    title: String,
    summary: String,
    authors: Vec<String>,
    published: String,
    updated: String,
    license: String,
    pdf: Vec<String>,
}

fn attr(event: &BytesStart<'_>, key: &str) -> Option<String> {
    event.attributes().filter_map(Result::ok).find_map(|value| {
        (value.key.as_ref() == key)
            .then(|| {
                value
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
            .flatten()
    })
}

impl RawArticle {
    fn link(&mut self, event: &BytesStart<'_>) {
        let Some(href) = attr(event, "href") else {
            return;
        };
        match attr(event, "rel").as_deref() {
            Some("related")
                if attr(event, "title").as_deref() == Some("pdf")
                    || attr(event, "type").as_deref() == Some("application/pdf") =>
            {
                self.pdf.push(href);
            }
            Some("license") if self.license.is_empty() => self.license = href,
            _ => {}
        }
    }

    fn article(self) -> Option<Article> {
        let id = linked_id(self.id.trim(), "abs")?;
        id.version?;
        let title = plain(&self.title, 1024);
        if title.is_empty() {
            return None;
        }
        let pdf = self.pdf.into_iter().find_map(|url| {
            (linked_id(&url, "pdf")? == id).then(|| format!("https://arxiv.org/pdf/{}", id.value()))
        });
        let date = [self.updated, self.published]
            .into_iter()
            .find_map(|value| {
                let text = plain(&value, 64);
                (!text.is_empty()).then_some(text)
            });
        let license = plain(&self.license, 512);
        Some(Article {
            id,
            title,
            summary: plain(&self.summary, 6000),
            authors: self
                .authors
                .iter()
                .map(|value| plain(value, 256))
                .filter(|value| !value.is_empty())
                .collect(),
            date,
            license: (!license.is_empty()).then_some(license),
            pdf,
        })
    }
}

fn plain(text: &str, max: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max)
        .collect()
}

fn linked_id(link: &str, route: &str) -> Option<Identifier> {
    let url = Url::parse(link).ok()?;
    if !official_url(&url) || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    let id = url
        .path()
        .strip_prefix('/')?
        .strip_prefix(route)?
        .strip_prefix('/')?;
    let id = percent_decode(id).ok()?;
    Identifier::parse(if route == "pdf" {
        id.strip_suffix(".pdf").unwrap_or(&id)
    } else {
        &id
    })
}

struct Feed {
    total: usize,
    articles: Vec<Article>,
}

fn parse_atom(bytes: &[u8]) -> Result<Feed, String> {
    let invalid = "arXiv did not return valid Atom metadata.";
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().expand_empty_elements = true;
    let mut path: Vec<String> = Vec::new();
    let mut root_seen = false;
    let mut articles = Vec::new();
    let mut raw = RawArticle::default();
    let mut total = String::new();
    loop {
        let chunk = match reader.read_event().map_err(|_| invalid)? {
            Event::Start(event) => {
                let name = event.local_name();
                let tag = name.as_ref();
                match (path.len(), tag) {
                    (0, "feed") if !root_seen => root_seen = true,
                    (0 | 24.., _) => return Err(invalid.into()),
                    (2, "author") if path[1] == "entry" => raw.authors.push(String::new()),
                    (2, "link") if path[1] == "entry" => raw.link(&event),
                    _ => {}
                }
                path.push(tag.to_owned());
                continue;
            }
            Event::End(_) => {
                if path.len() == 2 && path[1] == "entry" {
                    if articles.len() >= SEARCH_PAGE {
                        return Err("The arXiv metadata exceeds the requested page limit.".into());
                    }
                    if let Some(article) = std::mem::take(&mut raw).article() {
                        articles.push(article);
                    }
                }
                path.pop();
                continue;
            }
            Event::Text(event) => event.xml10_content().into_owned(),
            Event::CData(event) => event.as_ref().to_owned(),
            Event::GeneralRef(event) => {
                let name = event.xml10_content();
                quick_xml::escape::unescape(&format!("&{name};"))
                    .map_err(|_| invalid)?
                    .into_owned()
            }
            Event::DocType(_) => return Err(invalid.into()),
            Event::Eof => break,
            _ => continue,
        };
        let field = match (path.len(), path.last().map(String::as_str)) {
            (2, Some("totalResults")) => &mut total,
            (3, Some("id")) if path[1] == "entry" => &mut raw.id,
            (3, Some("title")) if path[1] == "entry" => &mut raw.title,
            (3, Some("summary")) if path[1] == "entry" => &mut raw.summary,
            (3, Some("published")) if path[1] == "entry" => &mut raw.published,
            (3, Some("updated")) if path[1] == "entry" => &mut raw.updated,
            (3, Some("license")) if path[1] == "entry" => &mut raw.license,
            (4, Some("name")) if path[1] == "entry" && path[2] == "author" => {
                let Some(author) = raw.authors.last_mut() else {
                    return Err(invalid.into());
                };
                author
            }
            _ => continue,
        };
        field.push_str(&chunk);
    }
    if !root_seen || !path.is_empty() {
        return Err(invalid.into());
    }
    Ok(Feed {
        total: total.trim().parse().unwrap_or(articles.len()),
        articles,
    })
}

fn page(target: &ArxivTarget, title: String) -> Page {
    Page {
        provider: Provider::Arxiv,
        source_url: canonical(target),
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

async fn one_article(id: &Identifier) -> Result<Article, String> {
    let feed = fetch(&[("id_list", &id.value()), ("max_results", "1")]).await?;
    feed.articles
        .into_iter()
        .find(|article| article.id.paper == id.paper && (id.version.is_none() || article.id == *id))
        .ok_or_else(|| "The selected arXiv article or version was not found.".into())
}

pub(super) async fn inspect(target: &ArxivTarget, cursor: Option<&str>) -> Result<Page, String> {
    match target {
        ArxivTarget::Search { field, query } => {
            let current = cursor
                .unwrap_or("1")
                .parse::<usize>()
                .ok()
                .filter(|number| (1..=MAX_SEARCH_PAGES).contains(number))
                .ok_or(
                    "arXiv searches are limited to 10 pages. Refine the query for more results.",
                )?;
            let start = (current - 1) * SEARCH_PAGE;
            let feed = fetch(&[
                ("search_query", &field.expression(query)),
                ("start", &start.to_string()),
                ("max_results", &SEARCH_PAGE.to_string()),
                ("sortBy", "lastUpdatedDate"),
                ("sortOrder", "descending"),
            ])
            .await?;
            let mut result = page(target, format!("arXiv · {query}"));
            let mut seen = HashSet::new();
            result.entries = feed
                .articles
                .iter()
                .filter(|article| seen.insert(&article.id.paper))
                .map(|article| article.entry(article.id.unversioned().url()))
                .collect();
            if feed.total > start + SEARCH_PAGE {
                if current < MAX_SEARCH_PAGES {
                    result.next_cursor = Some((current + 1).to_string());
                } else {
                    result.warnings.push("Only the first 200 arXiv results are available. Refine the search to find more.".into());
                }
            }
            Ok(result)
        }
        ArxivTarget::Article(id) => {
            let latest = one_article(id).await?;
            let mut result = page(target, latest.title.clone());
            result.description = (!latest.summary.is_empty()).then(|| latest.summary.clone());
            result.rights = latest.license.clone();
            if id.version.is_some() {
                if cursor.is_some() {
                    return Err("An exact arXiv version has no additional pages.".into());
                }
                result.assets.extend(latest.asset());
                result.entries.push(Entry {
                    id: format!("arxiv:{}", id.paper),
                    title: "View all arXiv versions".into(),
                    url: id.unversioned().url(),
                    prerelease: false,
                    published_at: None,
                });
            } else {
                let version = latest.id.version.expect("Atom article carries a version");
                let newest = match cursor {
                    None => version,
                    Some(cursor) => cursor
                        .parse::<u32>()
                        .ok()
                        .filter(|number| *number > 0 && *number < version)
                        .ok_or("Invalid arXiv version page. Open the article again.")?,
                };
                let oldest = newest.saturating_sub(VERSION_PAGE - 1).max(1);
                let values: Vec<_> = (oldest..=newest)
                    .rev()
                    .filter(|number| *number != version)
                    .map(|number| format!("{}v{number}", id.paper))
                    .collect();
                let mut articles = if values.is_empty() {
                    vec![]
                } else {
                    fetch(&[
                        ("id_list", &values.join(",")),
                        ("max_results", &values.len().to_string()),
                    ])
                    .await?
                    .articles
                };
                if newest == version {
                    articles.push(latest);
                }
                articles.retain(|article| {
                    article.id.paper == id.paper
                        && article
                            .id
                            .version
                            .is_some_and(|n| (oldest..=newest).contains(&n))
                });
                articles.sort_by(|a, b| b.id.version.cmp(&a.id.version));
                articles.dedup_by(|a, b| a.id == b.id);
                for article in articles {
                    if let Some(asset) = article.asset() {
                        result.assets.push(asset);
                    } else {
                        let mut entry = article.entry(article.id.url());
                        entry.title = format!(
                            "{} · v{} · metadata only",
                            entry.title,
                            article.id.version.unwrap_or_default()
                        );
                        result.entries.push(entry);
                    }
                }
                if oldest > 1 {
                    result.next_cursor = Some((oldest - 1).to_string());
                }
            }
            if result.assets.is_empty() {
                result
                    .warnings
                    .push("The official metadata does not list a PDF for these versions.".into());
            }
            Ok(result)
        }
    }
}

pub(super) async fn resolve(target: &ArxivTarget, asset_id: &str) -> Result<Asset, String> {
    let ArxivTarget::Article(id) = target else {
        return Err("Select a specific arXiv article version.".into());
    };
    if id.version.is_none() || id.asset_id() != asset_id {
        return Err("Select a specific arXiv article version.".into());
    }
    one_article(id)
        .await?
        .asset()
        .ok_or_else(|| "This arXiv version no longer offers a PDF.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARTICLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
    <feed xmlns="http://www.w3.org/2005/Atom" xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">
      <opensearch:totalResults>2</opensearch:totalResults>
      <entry>
        <id>http://arxiv.org/abs/1710.02185v2</id><title>Tests &amp; Data</title>
        <published>2017-10-05T12:00:00Z</published><updated>2018-01-10T10:00:00Z</updated>
        <summary>Abstract only.</summary>
        <author><name>One Author</name></author><author><name>Two Authors</name></author>
        <link rel="related" title="pdf" type="application/pdf" href="http://arxiv.org/pdf/1710.02185v2"/>
        <link rel="license" href="https://creativecommons.org/licenses/by/4.0/"/>
      </entry>
      <entry>
        <id>https://arxiv.org/abs/1710.02185v1</id><title>Earlier version</title>
        <published>2017-10-05T12:00:00Z</published>
        <link rel="related" title="pdf" href="https://arxiv.org/pdf/1710.02185v1.pdf"/>
      </entry>
    </feed>"#;

    #[test]
    fn metadata_and_pdf_identity_are_bound_to_each_version() {
        let feed = parse_atom(ARTICLE.as_bytes()).unwrap();
        let versions: Vec<_> = feed.articles.iter().filter_map(Article::asset).collect();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, "arxiv:1710.02185v2");
        assert_eq!(versions[0].title, "Tests & Data");
        assert_eq!(
            versions[0].author.as_deref(),
            Some("One Author, Two Authors")
        );
        assert_eq!(
            versions[0].published_at.as_deref(),
            Some("2018-01-10T10:00:00Z")
        );
        assert_eq!(
            versions[0].license.as_deref(),
            Some("https://creativecommons.org/licenses/by/4.0/")
        );
        assert_eq!(
            versions[0].download_url,
            "https://arxiv.org/pdf/1710.02185v2"
        );
        assert_eq!(versions[0].source_url, "https://arxiv.org/abs/1710.02185v2");
        assert_eq!(versions[1].filename, "arxiv-1710.02185v1.pdf");
        assert_ne!(versions[0].filename, versions[1].filename);
        assert_eq!(feed.total, 2);
    }

    #[tokio::test]
    async fn previous_version_remains_selectable_when_latest_advances() {
        let latest_url = "https://export.arxiv.org/api/query?id_list=1710.02185&max_results=1";
        let older_url = "https://export.arxiv.org/api/query?id_list=1710.02185v1&max_results=1";
        {
            let mut requests = REQUESTS.lock().await;
            for url in [latest_url, older_url] {
                requests.feeds.push_back(CachedFeed {
                    url: url.into(),
                    bytes: ARTICLE.as_bytes().to_vec(),
                    expires: Instant::now() + Duration::from_secs(30),
                });
            }
        }
        let target = ArxivTarget::Article(Identifier::parse("1710.02185").unwrap());
        let page = inspect(&target, None).await.unwrap();
        let versions: Vec<_> = page.assets.iter().map(|asset| asset.id.as_str()).collect();
        assert_eq!(versions, ["arxiv:1710.02185v2", "arxiv:1710.02185v1"]);
        assert_eq!(page.description.as_deref(), Some("Abstract only."));
        let previous = ArxivTarget::Article(Identifier::parse("1710.02185v1").unwrap());
        assert_eq!(
            resolve(&previous, &page.assets[1].id)
                .await
                .unwrap()
                .filename,
            "arxiv-1710.02185v1.pdf"
        );
        assert!(resolve(&previous, &page.assets[0].id).await.is_err());
        REQUESTS
            .lock()
            .await
            .feeds
            .retain(|feed| feed.url != latest_url && feed.url != older_url);
    }

    #[test]
    fn refuses_pdf_for_another_version_or_origin_and_keeps_metadata() {
        for invalid in [
            "https://arxiv.org/pdf/1710.02185v1",
            "https://arxiv.org/pdf/1710.02185",
            "https://arxiv.org.evil.test/pdf/1710.02185v2",
            "https://user@arxiv.org/pdf/1710.02185v2",
            "https://arxiv.org/pdf/1710.02185v2?token=secret",
        ] {
            let xml = ARTICLE.replace("http://arxiv.org/pdf/1710.02185v2", invalid);
            let article = &parse_atom(xml.as_bytes()).unwrap().articles[0];
            assert_eq!(article.title, "Tests & Data");
            assert!(
                article.asset().is_none(),
                "unexpected PDF accepted: {invalid}"
            );
        }
        let xml = ARTICLE.replace(
            "title=\"pdf\" type=\"application/pdf\"",
            "title=\"abstract\"",
        );
        assert!(parse_atom(xml.as_bytes()).unwrap().articles[0]
            .asset()
            .is_none());
    }

    #[test]
    fn supports_legacy_ids_and_literal_field_scoped_searches() {
        assert_eq!(
            normalize("arXiv:math.GT/0309136v2").as_deref(),
            Some("https://arxiv.org/abs/math.GT/0309136v2")
        );
        assert!(normalize("arXiv:1710.02185v0").is_none());
        assert!(normalize("arXiv:1710.02185/other").is_none());
        let url =
            Url::parse("https://arxiv.org/search/?searchtype=author&query=Anne+Example").unwrap();
        let target = parse(&url, &["search".into()]).unwrap();
        assert_eq!(
            canonical(&target),
            "https://arxiv.org/search/?field=author&query=Anne+Example"
        );
        let ArxivTarget::Search { field, query } = target else {
            panic!("expected a search");
        };
        assert_eq!(field.expression(&query), "au:\"Anne Example\"");
        let invalid =
            Url::parse("https://arxiv.org/search/?field=title&query=token%22+OR+all%3A%2A")
                .unwrap();
        assert!(parse(&invalid, &["search".into()]).is_err());
    }
}
