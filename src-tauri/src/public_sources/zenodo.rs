use super::*;
use std::{collections::VecDeque, sync::LazyLock, time::Instant};

const RECORDS: &str = "https://zenodo.org/api/records";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ZenodoTarget {
    Record(u64),
    Doi(String),
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

fn search_url(query: &str, current: usize) -> String {
    let mut url = url::Url::parse(RECORDS).expect("constant Zenodo endpoint");
    url.query_pairs_mut().append_pair("q", query)
        .append_pair("all_versions", "true").append_pair("size", "20")
        .append_pair("page", &current.to_string());
    url.to_string()
}

async fn record(target: &ZenodoTarget) -> Result<Value, String> {
    let (query, expected_doi, expected_concept) = match target {
        ZenodoTarget::Record(id) => {
            match metadata(format!("{RECORDS}/{id}")).await {
                Ok(raw) if id_field(&raw["id"]) == Some(*id) => return Ok(raw),
                Ok(_) => return Err("Zenodo returned a different record identifier.".into()),
                Err(error) if error == PUBLIC_NOT_FOUND => (),
                Err(error) => return Err(error),
            }
            (format!("conceptrecid:{id}"), None, Some(*id))
        }
        ZenodoTarget::Doi(value) => {
            if let Some(id) = value.strip_prefix("10.5281/zenodo.").and_then(record_id) {
                match metadata(format!("{RECORDS}/{id}")).await {
                    Ok(raw) if id_field(&raw["id"]) == Some(id) && matches_doi(&raw, value) => return Ok(raw),
                    Ok(_) => (),
                    Err(error) if error == PUBLIC_NOT_FOUND => (),
                    Err(error) => return Err(error),
                }
            }
            let escaped = value.replace('/', "\\/");
            (format!("doi:\"{escaped}\" OR conceptdoi:\"{escaped}\""), Some(value.as_str()), None)
        }
    };
    let raw = metadata(search_url(&query, 1)).await?;
    let id = raw["hits"]["hits"].as_array().into_iter().flatten()
        .filter(|entry| expected_doi.is_none_or(|doi| matches_doi(entry, doi)))
        .filter(|entry| expected_concept.is_none_or(|id| id_field(&entry["conceptrecid"]) == Some(id)))
        .filter_map(|entry| id_field(&entry["id"]))
        .max()
        .ok_or(PUBLIC_NOT_FOUND)?;
    let raw = metadata(format!("{RECORDS}/{id}")).await?;
    if id_field(&raw["id"]) != Some(id)
        || expected_doi.is_some_and(|doi| !matches_doi(&raw, doi))
        || expected_concept.is_some_and(|id| id_field(&raw["conceptrecid"]) != Some(id))
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
    let concept = id_field(&raw["conceptrecid"]);
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

pub(super) async fn inspect(target: &ZenodoTarget, cursor: Option<&str>) -> Result<Page, String> {
    let current = numeric_page(cursor)?;
    let raw = record(target).await?;
    let id = id_field(&raw["id"]).ok_or("Zenodo returned no record identifier.")?;
    let meta = &raw["metadata"];
    let mut page = page(&Target::Zenodo(target.clone()), Provider::Zenodo,
        field(meta, "title", 240).unwrap_or_else(|| format!("Zenodo record {id}")));
    page.description = field(meta, "description", 1800);
    let identity = field(&raw, "doi", 200).unwrap_or_else(|| id.to_string());
    let family = field(&raw, "conceptdoi", 200)
        .or_else(|| id_field(&raw["conceptrecid"]).map(|id| id.to_string()));
    page.warnings.push(match family {
        Some(family) => format!("Record version {identity} · Record family {family}. Files are bound to this version."),
        None => format!("Record version {identity}. Files are bound to this version."),
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
}
