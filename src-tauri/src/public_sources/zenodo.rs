use super::*;
use std::{collections::{HashSet, VecDeque}, sync::LazyLock, time::Instant};

const RECORDS: &str = "https://zenodo.org/api/records";
const COMMUNITIES: &str = "https://zenodo.org/api/communities";
const CATALOG_PAGE_SIZE: usize = 20;
const CATALOG_PAGE_LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ZenodoTarget {
    Record(u64),
    Doi(String),
    Versions(u64),
    Community(String),
}

fn record_id(value: &str) -> Option<u64> {
    (value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<u64>().ok().filter(|id| *id > 0))
        .flatten()
}

fn doi(value: &str) -> Option<String> {
    let (prefix, name) = value.split_once('/')?;
    let registrant = prefix.strip_prefix("10.")?;
    (value.len() <= 200
        && (4..=9).contains(&registrant.len())
        && registrant.bytes().all(|byte| byte.is_ascii_digit())
        && !name.is_empty()
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-._/():;".contains(&byte)))
        .then(|| value.to_ascii_lowercase())
}

pub(super) fn normalize(input: &str) -> Option<String> {
    let input = input.strip_prefix("zenodo:").unwrap_or(input).trim();
    if let Some(id) = record_id(input) {
        return Some(canonical(&ZenodoTarget::Record(id)));
    }
    doi(input.strip_prefix("doi:").unwrap_or(input).trim())
        .map(|doi| canonical(&ZenodoTarget::Doi(doi)))
}

pub(super) fn parse(url: &url::Url, parts: &[String]) -> Result<ZenodoTarget, String> {
    if matches!(url.host_str(), Some("doi.org" | "dx.doi.org")) {
        return doi(&parts.join("/"))
            .map(ZenodoTarget::Doi)
            .ok_or_else(|| "Use a valid Zenodo record DOI.".into());
    }
    let community = match parts {
        [route, id] if route == "communities" => Some(id),
        [route, id, records] if route == "communities" && records == "records" => Some(id),
        _ => None,
    };
    if let Some(id) = community {
        if url.query().is_some_and(|query| !query.is_empty())
            || id.is_empty()
            || id.len() > 100
            || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err("Use a Zenodo community link without search filters.".into());
        }
        return Ok(ZenodoTarget::Community(id.to_ascii_lowercase()));
    }
    if matches!(parts, [api, route] if api == "api" && route == "records") {
        let mut family = None;
        let mut all_versions = false;
        for (name, value) in url.query_pairs() {
            match name.as_ref() {
                "q" if family.is_none() => {
                    family = value.strip_prefix("conceptrecid:").and_then(record_id);
                    if family.is_none() { break; }
                }
                "all_versions" if !all_versions && value == "true" => all_versions = true,
                _ => return Err("Use a Zenodo history link for a single record family.".into()),
            }
        }
        return family.filter(|_| all_versions).map(ZenodoTarget::Versions)
            .ok_or_else(|| "Use a Zenodo history link for a single record family.".into());
    }
    match parts {
        [route, id] if matches!(route.as_str(), "record" | "records") => record_id(id),
        [api, route, id] if api == "api" && route == "records" => record_id(id),
        _ => None,
    }
    .map(ZenodoTarget::Record)
    .ok_or_else(|| "Use a Zenodo record URL, numeric ID or DOI.".into())
}

pub(super) fn canonical(target: &ZenodoTarget) -> String {
    match target {
        ZenodoTarget::Record(id) => format!("https://zenodo.org/records/{id}"),
        ZenodoTarget::Doi(doi) => url_path("https://doi.org", &doi.split('/').collect::<Vec<_>>()),
        ZenodoTarget::Versions(family) => catalog_url(&format!("conceptrecid:{family}")).to_string(),
        ZenodoTarget::Community(id) => format!("https://zenodo.org/communities/{id}"),
    }
}

struct Cached {
    url: String,
    value: Value,
    inserted: Instant,
}

static CACHE: LazyLock<tokio::sync::Mutex<VecDeque<Cached>>> = LazyLock::new(Default::default);

async fn metadata(url: String) -> Result<Value, String> {
    let mut cache = CACHE.lock().await;
    if let Some(entry) = cache.iter().find(|entry| {
        entry.url == url && entry.inserted.elapsed() < Duration::from_secs(60)
    }) {
        return Ok(entry.value.clone());
    }
    if let Some(last) = cache.back() {
        tokio::time::sleep(Duration::from_secs(2).saturating_sub(last.inserted.elapsed())).await;
    }
    let value = api_json(&url).await?;
    cache.retain(|entry| entry.url != url);
    if cache.len() >= 8 {
        cache.pop_front();
    }
    cache.push_back(Cached { url, value: value.clone(), inserted: Instant::now() });
    Ok(value)
}

fn id_field(value: &Value) -> Option<u64> {
    value.as_u64().filter(|id| *id > 0).or_else(|| value.as_str().and_then(record_id))
}

fn family_id(raw: &Value) -> Option<u64> {
    id_field(&raw["conceptrecid"]).or_else(|| id_field(&raw["parent"]["id"]))
}

fn record_matches(raw: &Value, requested: u64) -> bool {
    id_field(&raw["id"]).is_some_and(|id| id == requested || family_id(raw) == Some(requested))
}

fn catalog_url(query: &str) -> url::Url {
    let mut url = url::Url::parse(RECORDS).expect("constant Zenodo endpoint");
    url.query_pairs_mut().append_pair("q", query)
        .append_pair("all_versions", "true");
    url
}

fn search_url(query: &str, current: usize) -> String {
    let mut url = catalog_url(query);
    url.query_pairs_mut().append_pair("size", &CATALOG_PAGE_SIZE.to_string())
        .append_pair("page", &current.to_string()).append_pair("sort", "mostrecent");
    url.to_string()
}

async fn record(target: &ZenodoTarget) -> Result<Value, String> {
    let (query, expected_doi, expected_concept) = match target {
        ZenodoTarget::Record(id) => {
            match metadata(format!("{RECORDS}/{id}")).await {
                Ok(raw) if record_matches(&raw, *id) => return Ok(raw),
                Ok(_) => return Err("Zenodo returned a different record identifier.".into()),
                Err(error) if error == PUBLIC_NOT_FOUND => (),
                Err(error) => return Err(error),
            }
            (format!("conceptrecid:{id}"), None, Some(*id))
        }
        ZenodoTarget::Doi(value) => {
            if let Some(id) = value.strip_prefix("10.5281/zenodo.").and_then(record_id) {
                match metadata(format!("{RECORDS}/{id}")).await {
                    Ok(raw) if record_matches(&raw, id) && matches_doi(&raw, value) => return Ok(raw),
                    Ok(_) => (),
                    Err(error) if error == PUBLIC_NOT_FOUND => (),
                    Err(error) => return Err(error),
                }
            }
            let escaped = value.replace('/', "\\/");
            (format!("doi:\"{escaped}\" OR conceptdoi:\"{escaped}\""), Some(value.as_str()), None)
        }
        ZenodoTarget::Versions(_) | ZenodoTarget::Community(_) => {
            return Err("Open a specific Zenodo record to review its files.".into());
        }
    };
    let raw = metadata(search_url(&query, 1)).await?;
    let id = raw["hits"]["hits"].as_array().into_iter().flatten()
        .filter(|entry| expected_doi.is_none_or(|doi| matches_doi(entry, doi)))
        .filter(|entry| expected_concept.is_none_or(|id| family_id(entry) == Some(id)))
        .filter_map(|entry| id_field(&entry["id"]))
        .max()
        .ok_or(PUBLIC_NOT_FOUND)?;
    let raw = metadata(format!("{RECORDS}/{id}")).await?;
    if id_field(&raw["id"]) != Some(id)
        || expected_doi.is_some_and(|doi| !matches_doi(&raw, doi))
        || expected_concept.is_some_and(|id| family_id(&raw) != Some(id))
    {
        return Err("Zenodo returned a different record or record family.".into());
    }
    Ok(raw)
}

fn matches_doi(raw: &Value, expected: &str) -> bool {
    [&raw["doi"], &raw["metadata"]["doi"], &raw["conceptdoi"]].iter()
        .any(|value| value.as_str().is_some_and(|value| value.eq_ignore_ascii_case(expected)))
}

fn checksum(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    let (algorithm, digest) = value.split_once(':').unwrap_or(("md5", value));
    let len = match algorithm { "md5" => 32, "sha256" => 64, _ => return None };
    hex_digest(digest, len).map(|digest| format!("{algorithm}:{digest}"))
}

fn files_open(raw: &Value) -> bool {
    let declared = raw["access"]["files"].as_str().or(raw["metadata"]["access_right"].as_str());
    matches!(declared, Some("public" | "open"))
        && !matches!(raw["metadata"]["access_right"].as_str(), Some("restricted" | "closed" | "embargoed"))
        && !truthy(&raw["access"]["embargo"]["active"])
        && raw["files"]["enabled"].as_bool() != Some(false)
}

fn file_list(raw: &Value) -> Vec<&Value> {
    let files = &raw["files"];
    if let Some(files) = files.as_array() {
        return files.iter().collect();
    }
    files["entries"].as_object().into_iter().flat_map(|entries| entries.values()).collect()
}

fn asset(raw: &Value, id: u64, file: &Value) -> Option<Asset> {
    let key = file["key"].as_str().or(file["filename"].as_str())?;
    if key.contains(['/', '\\']) || key.len() > 512 { return None; }
    let name = filename(key)?;
    crate::direct::validate_filename(&name).ok()?;
    let id_text = id.to_string();
    let metadata = &raw["metadata"];
    let checksum = checksum(&file["checksum"]);
    let authors = metadata["creators"].as_array().into_iter().flatten()
        .filter_map(|creator| field(creator, "name", 200)).take(30).collect::<Vec<_>>();
    let size = file["size"].as_u64().or(file["filesize"].as_u64());
    let license = field(&metadata["license"], "id", 100).or_else(|| {
        metadata["rights"].as_array().map(|rights| rights.iter()
            .filter_map(|right| field(right, "id", 100)).take(8).collect::<Vec<_>>().join(", "))
            .filter(|value| !value.is_empty())
    });
    let concept = family_id(raw);
    Some(Asset {
        id: format!("zenodo:{id}:{key}"),
        source_url: canonical(&ZenodoTarget::Record(id)),
        download_url: url_path(RECORDS, &[&id_text, "files", key, "content"]),
        filename: name,
        mime: field(file, "mimetype", 100).unwrap_or_else(|| mime_for_name(key).into()),
        size,
        title: key.into(),
        version: Some(field(metadata, "version", 100).unwrap_or_else(|| format!("Record {id}"))),
        original: true,
        author: (!authors.is_empty()).then(|| authors.join(", ")),
        license,
        attribution: concept.map(|concept| format!("Zenodo record {id} · record family {concept}")),
        sha256: checksum.as_deref().and_then(|value| value.strip_prefix("sha256:")).map(str::to_owned),
        platform: None,
        architecture: None,
        published_at: field(metadata, "publication_date", 40),
        checksum,
        audiobook: None,
    })
}

fn catalog_cursor(page: &mut Page, raw: &Value, count: usize, current: usize) {
    let total = raw["hits"]["total"].as_u64().or_else(|| raw["hits"]["total"]["value"].as_u64());
    let more = total.map_or(count == CATALOG_PAGE_SIZE, |total| total > (current * CATALOG_PAGE_SIZE) as u64);
    if more && current < CATALOG_PAGE_LIMIT {
        page.next_cursor = Some((current + 1).to_string());
    }
}

fn version_page(raw: &Value, family: u64, current: usize) -> Result<Page, String> {
    let hits = raw["hits"]["hits"].as_array().ok_or("Zenodo returned no version list.")?;
    let mut seen = HashSet::new();
    let mut title = None;
    let mut entries = Vec::new();
    for hit in hits.iter().take(CATALOG_PAGE_SIZE) {
        let Some(id) = id_field(&hit["id"]).filter(|id| family_id(hit) == Some(family) && seen.insert(*id)) else { continue; };
        let meta = &hit["metadata"];
        let name = field(meta, "title", 200).unwrap_or_else(|| format!("Zenodo {id}"));
        let version = field(meta, "version", 36).unwrap_or_else(|| id.to_string());
        if title.is_none() { title = Some(name.clone()); }
        entries.push(Entry {
            id: format!("zenodo:{id}"),
            title: format!("{version} · {name}"),
            url: canonical(&ZenodoTarget::Record(id)),
            prerelease: false,
            published_at: field(meta, "publication_date", 40),
        });
    }
    let mut page = page(&Target::Zenodo(ZenodoTarget::Versions(family)), Provider::Zenodo,
        title.unwrap_or_else(|| format!("Zenodo {family}")));
    page.entries = entries;
    catalog_cursor(&mut page, raw, hits.len(), current);
    Ok(page)
}

pub(super) async fn community_info(identifier: &str) -> Result<(String, String), String> {
    let raw = metadata(url_path(COMMUNITIES, &[identifier])).await?;
    let id = raw["id"].as_str().filter(|id| {
        id.len() == 36 && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
        })
    }).ok_or("Zenodo returned an invalid community identifier.")?;
    if raw["access"]["visibility"].as_str() != Some("public")
        || (id != identifier && raw["slug"].as_str() != Some(identifier))
    {
        return Err("Zenodo did not return the requested public community.".into());
    }
    let title = field(&raw["metadata"], "title", 240).unwrap_or_else(|| format!("Zenodo · {identifier}"));
    Ok((id.to_ascii_lowercase(), title))
}

fn community_page(target: &ZenodoTarget, title: String, raw: &Value, current: usize) -> Result<Page, String> {
    let hits = raw["hits"]["hits"].as_array().ok_or("Zenodo returned no community record list.")?;
    let mut result = page(&Target::Zenodo(target.clone()), Provider::Zenodo, title);
    let mut seen = HashSet::new();
    result.entries = hits.iter().take(CATALOG_PAGE_SIZE).filter_map(|record| {
        let id = id_field(&record["id"]).filter(|id| seen.insert(*id))?;
        let meta = &record["metadata"];
        Some(Entry {
            id: format!("zenodo:{id}"),
            title: field(meta, "title", 240).unwrap_or_else(|| format!("Zenodo {id}")),
            url: canonical(&ZenodoTarget::Record(id)),
            prerelease: false,
            published_at: field(meta, "publication_date", 40),
        })
    }).collect();
    catalog_cursor(&mut result, raw, hits.len(), current);
    Ok(result)
}

pub(super) async fn inspect(target: &ZenodoTarget, cursor: Option<&str>) -> Result<Page, String> {
    let current = numeric_page(cursor)?;
    if let ZenodoTarget::Versions(family) = target {
        if current > CATALOG_PAGE_LIMIT { return Err("Zenodo history is limited to 200 versions per exploration.".into()); }
        let raw = metadata(search_url(&format!("conceptrecid:{family}"), current)).await?;
        return version_page(&raw, *family, current);
    }
    if let ZenodoTarget::Community(identifier) = target {
        if current > CATALOG_PAGE_LIMIT { return Err("Zenodo communities are limited to 200 records per exploration.".into()); }
        let (id, title) = community_info(identifier).await?;
        let mut url = url::Url::parse(&url_path(COMMUNITIES, &[&id, "records"]))
            .expect("constant Zenodo endpoint and validated identifier");
        url.query_pairs_mut().append_pair("size", &CATALOG_PAGE_SIZE.to_string())
            .append_pair("page", &current.to_string()).append_pair("sort", "mostrecent");
        let raw = metadata(url.to_string()).await?;
        return community_page(target, title, &raw, current);
    }
    let raw = record(target).await?;
    let id = id_field(&raw["id"]).ok_or("Zenodo returned no record identifier.")?;
    let meta = &raw["metadata"];
    let mut page = page(&Target::Zenodo(target.clone()), Provider::Zenodo,
        field(meta, "title", 240).unwrap_or_else(|| format!("Zenodo record {id}")));
    page.description = field(meta, "description", 1800);
    let family = family_id(&raw);
    page.zenodo = Some(ZenodoRecord {
        record_id: id.to_string(),
        family_id: family.map(|id| id.to_string()),
        version: field(meta, "version", 100),
        versions_url: family.map(|id| canonical(&ZenodoTarget::Versions(id))),
    });
    if !files_open(&raw) {
        page.warnings.push("Zenodo did not declare public file access. Restricted or embargoed files cannot be selected.".into());
        return Ok(page);
    }
    let files = file_list(&raw);
    let start = (current - 1) * PAGE_SIZE;
    page.assets = files.iter().skip(start).take(PAGE_SIZE).filter_map(|file| asset(&raw, id, file)).collect();
    next_page(&mut page, current, start + PAGE_SIZE < files.len());
    Ok(page)
}

pub(super) async fn resolve(target: &ZenodoTarget, id: &str) -> Result<Asset, String> {
    let ZenodoTarget::Record(record_id) = target else {
        return Err("Select a file from a specific Zenodo record version.".into());
    };
    let key = id.strip_prefix(&format!("zenodo:{record_id}:")).ok_or("Invalid Zenodo file identifier.")?;
    let raw = metadata(format!("{RECORDS}/{record_id}")).await?;
    if id_field(&raw["id"]) != Some(*record_id) || !files_open(&raw) {
        return Err("This Zenodo record version no longer exposes its files publicly.".into());
    }
    file_list(&raw).into_iter()
        .find(|file| file["key"].as_str().or(file["filename"].as_str()) == Some(key))
        .and_then(|file| asset(&raw, *record_id, file))
        .ok_or_else(|| "The selected Zenodo file is no longer available.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_never_share_file_identity_and_keep_published_checksum() {
        let mut raw = json!({"conceptrecid":"7", "metadata":{"version":"1.2", "access_right":"open"}});
        let file = json!({"key":"dataset.csv", "size":25, "checksum":"md5:d41d8cd98f00b204e9800998ecf8427e"});
        assert!(files_open(&raw));
        let original = asset(&raw, 8, &file).unwrap();
        raw["metadata"]["version"] = json!("1.3");
        let updated = asset(&raw, 9, &file).unwrap();
        assert_ne!(original.id, updated.id);
        assert_eq!(updated.source_url, "https://zenodo.org/records/9");
        assert_eq!(updated.download_url, "https://zenodo.org/api/records/9/files/dataset.csv/content");
        assert_eq!(original.checksum.as_deref(), Some("md5:d41d8cd98f00b204e9800998ecf8427e"));
        raw["metadata"]["access_right"] = json!("embargoed");
        assert!(!files_open(&raw));
    }

    #[test]
    fn identifiers_and_dois_are_normalized_without_resolving_other_hosts() {
        assert_eq!(normalize("zenodo:123").as_deref(), Some("https://zenodo.org/records/123"));
        assert_eq!(normalize("doi:10.5281/ZENODO.123").as_deref(), Some("https://doi.org/10.5281/zenodo.123"));
        assert_eq!(normalize("10.5281/zenodo.123?q=x"), None);
    }

    #[test]
    fn published_family_id_resolves_without_accepting_an_unrelated_record() {
        let raw = json!({
            "id": 8, "conceptrecid": "7", "doi": "10.5281/zenodo.8", "conceptdoi": "10.5281/zenodo.7"
        });
        assert!(record_matches(&raw, 7));
        assert!(record_matches(&raw, 8));
        assert!(!record_matches(&raw, 9));
        assert!(matches_doi(&raw, "10.5281/zenodo.7"));
        assert!(!matches_doi(&raw, "10.5281/zenodo.9"));
        assert!(!record_matches(&json!({"conceptrecid": "7"}), 7));
    }

    #[test]
    fn history_keeps_only_distinct_published_records_from_the_requested_family() {
        let result = version_page(&json!({"hits": {"total": {"value": 22}, "hits": [
            {"id": 9, "conceptrecid": "7", "metadata": {"title": "Data", "version": "v2"}},
            {"id": 9, "conceptrecid": "7"},
            {"id": 51, "conceptrecid": "50"},
            {"id": 8, "parent": {"id": 7}, "metadata": {"title": "Data", "version": "v1"}}
        ]}}), 7, 1).unwrap();
        assert_eq!(result.source_url, "https://zenodo.org/api/records?q=conceptrecid%3A7&all_versions=true");
        assert_eq!(result.next_cursor.as_deref(), Some("2"));
        assert!(result.assets.is_empty());
        assert_eq!(result.entries.iter().map(|entry| entry.url.as_str()).collect::<Vec<_>>(), [
            "https://zenodo.org/records/9",
            "https://zenodo.org/records/8",
        ]);
        let url = url::Url::parse(&result.source_url).unwrap();
        let path = vec!["api".to_string(), "records".to_string()];
        assert_eq!(parse(&url, &path).unwrap(), ZenodoTarget::Versions(7));
        let unrelated = url::Url::parse("https://zenodo.org/api/records?q=conceptrecid%3A7+OR+*&all_versions=true").unwrap();
        assert!(parse(&unrelated, &path).is_err());
    }

    #[test]
    fn community_pages_are_bounded_and_only_link_to_exact_records() {
        let url = "https://zenodo.org/communities/research-fixture";
        assert_eq!(canonical_followed_search(&format!("{url}/records")).as_deref(), Ok(url));
        assert!(canonical_followed_search(&format!("{url}?q=sample")).is_err());
        assert!(canonical_followed_search("https://zenodo.org/communities/research%2Ffixture").is_err());
        let target = ZenodoTarget::Community("research-fixture".into());
        let raw = json!({"hits": {"total": 240, "hits": [
            {"id": 9, "metadata": {"title": "Open record", "publication_date": "2025-04-03"}},
            {"id": 9},
            {"id": "8", "metadata": {"title": "Restricted files", "access_right": "restricted"}},
            {"id": "invalid", "links": {"self": "https://example.invalid"}}
        ]}});
        let result = community_page(&target, "Research group".into(), &raw, 1).unwrap();
        assert_eq!(result.source_url, url);
        assert_eq!(result.title, "Research group");
        assert_eq!(result.next_cursor.as_deref(), Some("2"));
        assert!(result.assets.is_empty());
        assert_eq!(result.entries.iter().map(|entry| entry.url.as_str()).collect::<Vec<_>>(), [
            "https://zenodo.org/records/9",
            "https://zenodo.org/records/8",
        ]);
        assert!(valid_followed_search_entry(url, &result.entries[0].url, "zenodo:9"));
        assert!(!valid_followed_search_entry(url, &result.entries[0].url, "zenodo:8"));
        assert!(community_page(&target, "Research group".into(), &raw, 10).unwrap().next_cursor.is_none());
    }
}
