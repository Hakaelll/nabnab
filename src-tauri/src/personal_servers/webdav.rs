use super::{cancelled, valid_relative, Entry, Stamp};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use reqwest::{header, redirect::Policy, Certificate, Client, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet, fs::OpenOptions, io::Write, net::IpAddr, path::Path, time::Duration,
};
use tokio::sync::watch;
use url::Url;

const DAV_BODY: &str = r#"<?xml version="1.0" encoding="UTF-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getetag/><d:getcontenttype/></d:prop></d:propfind>"#;
const MAX_XML: usize = 4 * 1024 * 1024;
const MAX_CERT_PEM: usize = 8192;
const MAX_ENTRIES: usize = 2_000;
const MAX_DOWNLOAD: u64 = 128 * 1024 * 1024 * 1024;
const CHANGED: &str = "The WebDAV file changed. Refresh the folder and select it again.";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Config {
    pub root: String,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_pem: Option<String>,
}

fn certificate(pem: &str) -> Result<Certificate, String> {
    let message = "Paste one valid PEM certificate from the WebDAV server administrator.";
    let encoded = pem.strip_prefix("-----BEGIN CERTIFICATE-----\n")
        .and_then(|value| value.strip_suffix("\n-----END CERTIFICATE-----"))
        .ok_or(message)?;
    if pem.len() > MAX_CERT_PEM || encoded.is_empty()
        || !encoded.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"+/=\n".contains(&byte))
    {
        return Err(message.into());
    }
    Certificate::from_pem(pem.as_bytes()).map_err(|_| message.into())
}

pub(super) fn normalize(mut config: Config) -> Result<Config, String> {
    if config.username.is_empty()
        || config.username.len() > 512
        || config.username.contains(':')
        || config.username.chars().any(char::is_control)
        || config.password.is_empty()
        || config.password.len() > 4096
        || config.password.chars().any(char::is_control)
    {
        return Err("Enter a WebDAV username and app password without control characters.".into());
    }
    let root = config.root.trim();
    let mut url = endpoint(root)?;
    let root_path = decoded_path(raw_path(root).ok_or("Enter an HTTPS WebDAV folder URL.")?)?;
    url.set_path("/");
    append_path(&mut url, &root_path, true)?;
    config.root = url.to_string();
    if let Some(pem) = &mut config.certificate_pem {
        if pem.len() > MAX_CERT_PEM {
            return Err("The WebDAV certificate exceeds 8 KiB.".into());
        }
        *pem = pem.trim().replace("\r\n", "\n");
        certificate(pem)?;
    }
    Ok(config)
}

fn endpoint(value: &str) -> Result<Url, String> {
    if value.len() > 4096 || value.contains('\\') || value.chars().any(char::is_control) {
        return Err("The WebDAV URL is too long or contains invalid characters.".into());
    }
    let url = Url::parse(value).map_err(|_| "Enter an HTTPS WebDAV folder URL.")?;
    if url.scheme() != "https"
        || !value.starts_with("https://")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return Err(
            "Use an HTTPS WebDAV folder URL without credentials, a query or a fragment.".into(),
        );
    }
    Ok(url)
}

fn raw_path(url: &str) -> Option<&str> {
    let authority = url.strip_prefix("https://")?;
    (!authority.is_empty() && !authority.starts_with('/')).then(|| {
        authority
            .find('/')
            .map_or("/", |offset| &authority[offset..])
    })
}

fn decode_segment(segment: &str) -> Option<String> {
    let mut result = Vec::new();
    let mut bytes = segment.bytes();
    while let Some(byte) = bytes.next() {
        result.push(if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    String::from_utf8(result).ok()
}

fn decoded_path(path: &str) -> Result<Vec<String>, String> {
    let path = path
        .strip_prefix('/')
        .ok_or("The WebDAV path is not absolute.")?;
    if path.is_empty() {
        return Ok(vec![]);
    }
    let parts = path
        .strip_suffix('/')
        .unwrap_or(path)
        .split('/')
        .map(decode_segment)
        .collect::<Option<Vec<_>>>()
        .ok_or("The WebDAV path is not valid UTF-8 percent encoding.")?;
    if parts.len() > 64 || !parts.chunks(32).all(valid_relative) {
        return Err(
            "The WebDAV path contains an unsafe segment or exceeds the depth limit.".into(),
        );
    }
    Ok(parts)
}

fn append_path(url: &mut Url, relative: &[String], directory: bool) -> Result<(), String> {
    if relative.is_empty() {
        return Ok(());
    }
    if !valid_relative(relative) {
        return Err(
            "The WebDAV path is outside the allowed folder depth or contains an unsafe segment."
                .into(),
        );
    }
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| "Invalid WebDAV folder URL.")?;
    segments.pop_if_empty().extend(relative);
    if directory {
        segments.push("");
    }
    Ok(())
}

fn href_path(root: &Url, root_parts: &[String], href: &str) -> Result<Vec<String>, String> {
    if href.len() > 4096 || href.contains(['?', '#', '\\']) || href.chars().any(char::is_control) {
        return Err("Invalid WebDAV response path.".into());
    }
    let path = if href.starts_with('/') && !href.starts_with("//") {
        href
    } else {
        let absolute = endpoint(href)?;
        if absolute.origin() != root.origin() {
            return Err("The WebDAV response refers to a different server.".into());
        }
        raw_path(href).ok_or("Invalid WebDAV response URL.")?
    };
    let parts = decoded_path(path)?;
    parts
        .strip_prefix(root_parts)
        .filter(|relative| valid_relative(relative))
        .map(<[String]>::to_vec)
        .ok_or_else(|| "The WebDAV response refers outside the authorized root.".into())
}

fn allowed_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast(),
        IpAddr::V6(ip) => {
            !ip.is_unspecified()
                && !ip.is_multicast()
                && ip
                    .to_ipv4_mapped()
                    .is_none_or(|ip| allowed_address(ip.into()))
        }
    }
}

async fn client(root: &Url, config: &Config) -> Result<Client, String> {
    let mut builder = Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(24 * 60 * 60))
        .user_agent(concat!("nabnab/", env!("CARGO_PKG_VERSION")));
    if let Some(pem) = config.certificate_pem.as_deref() {
        builder = builder.tls_certs_only([certificate(pem)?]);
    }
    match root.host().ok_or("The WebDAV URL has no host.")? {
        url::Host::Domain(host) => {
            let port = root
                .port_or_known_default()
                .ok_or("The WebDAV URL has no HTTPS port.")?;
            let addresses = tokio::time::timeout(
                Duration::from_secs(8),
                tokio::net::lookup_host((host, port)),
            )
            .await
            .map_err(|_| "The WebDAV server lookup timed out.")?
            .map_err(|_| "The configured WebDAV host could not be resolved.")?;
            let approved: Vec<_> = addresses.take(33).collect();
            if approved.is_empty()
                || approved.len() > 32
                || approved
                    .iter()
                    .any(|address| !allowed_address(address.ip()))
            {
                return Err("The configured WebDAV host has no usable address.".into());
            }
            builder = builder.resolve_to_addrs(host, &approved);
        }
        url::Host::Ipv4(ip) if !allowed_address(ip.into()) => {
            return Err("Invalid WebDAV address.".into())
        }
        url::Host::Ipv6(ip) if !allowed_address(ip.into()) => {
            return Err("Invalid WebDAV address.".into())
        }
        _ => {}
    }
    builder
        .build()
        .map_err(|_| "Could not prepare a secure WebDAV client.".into())
}

fn request(client: &Client, config: &Config, method: Method, url: Url) -> reqwest::RequestBuilder {
    client
        .request(method, url)
        .basic_auth(&config.username, Some(&config.password))
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("Apply-To-Redirect-Ref", "T")
}

async fn send(request: reqwest::RequestBuilder, wanted: StatusCode) -> Result<Response, String> {
    let response = request.send().await.map_err(|_| "Could not connect to the WebDAV server. Check its address, availability and HTTPS certificate.")?;
    let status = response.status();
    if status == wanted {
        return Ok(response);
    }
    Err(match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "The WebDAV server denied access. Check the username, app password and folder permissions.".into(),
        StatusCode::PRECONDITION_FAILED => CHANGED.into(),
        _ if status.is_redirection() => "The WebDAV server redirected the request. Configure its final HTTPS folder URL.".into(),
        _ => format!("The WebDAV server returned HTTP {}.", status.as_u16()),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Multi,
    Response,
    Href,
    Status,
    Propstat,
    Prop,
    Resource,
    Collection,
    Size,
    Modified,
    Etag,
    Mime,
    Other,
}
use Node::*;

fn node(namespace: ResolveResult<'_>, name: &[u8]) -> Node {
    if !matches!(namespace, ResolveResult::Bound(value) if value.as_ref() == "DAV:") {
        return Other;
    }
    match name {
        b"multistatus" => Multi,
        b"response" => Response,
        b"href" => Href,
        b"status" => Status,
        b"propstat" => Propstat,
        b"prop" => Prop,
        b"resourcetype" => Resource,
        b"collection" => Collection,
        b"getcontentlength" => Size,
        b"getlastmodified" => Modified,
        b"getetag" => Etag,
        b"getcontenttype" => Mime,
        _ => Other,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Directory,
    Unsupported,
}

#[derive(Default)]
struct Props {
    kind: Option<Kind>,
    size: Option<u64>,
    modified: Option<String>,
    etag: Option<String>,
    mime: Option<String>,
}

impl Props {
    fn merge(&mut self, other: Self) {
        self.kind = match (self.kind, other.kind) {
            (Some(previous), Some(next)) if previous != next => Some(Kind::Unsupported),
            (previous, next) => next.or(previous),
        };
        self.size = other.size.or(self.size);
        self.modified = other.modified.or_else(|| self.modified.take());
        self.etag = other.etag.or_else(|| self.etag.take());
        self.mime = other.mime.or_else(|| self.mime.take());
    }
}

#[derive(Default)]
struct Resource {
    href: String,
    props: Props,
    success: Option<bool>,
}

fn http_ok(value: &str) -> bool {
    let mut words = value.split_whitespace();
    words.next().is_some_and(|word| word.starts_with("HTTP/"))
        && words
            .next()
            .and_then(|word| word.parse::<u16>().ok())
            .is_some_and(|code| (200..300).contains(&code))
}

fn etag_value(value: &str) -> Option<String> {
    let quoted = value.strip_prefix("W/").unwrap_or(value);
    let opaque = quoted.strip_prefix('"')?.strip_suffix('"')?;
    (value.len() <= 512
        && opaque
            .bytes()
            .all(|byte| byte == b'!' || (b'#'..=b'~').contains(&byte)))
    .then(|| value.to_owned())
}

fn modified_value(value: &str) -> Option<String> {
    httpdate::parse_http_date(value)
        .ok()
        .map(httpdate::fmt_http_date)
}

#[derive(Default)]
struct Xml {
    stack: Vec<Node>,
    text: String,
    resources: Vec<Resource>,
    current: Resource,
    propstat: Props,
    propstat_ok: bool,
    saw_root: bool,
}

impl Xml {
    fn open(&mut self, name: Node) -> Result<(), String> {
        if self.stack.is_empty() {
            if self.saw_root || name != Multi {
                return Err("The WebDAV server returned an invalid multistatus document.".into());
            }
            self.saw_root = true;
        }
        if self.stack.len() >= 24 {
            return Err("The WebDAV XML nesting limit was exceeded.".into());
        }
        if matches!(
            self.stack.last(),
            Some(Href | Status | Size | Modified | Etag | Mime)
        ) {
            return Err("A WebDAV text property contained nested markup.".into());
        }
        if self.stack.as_slice() == [Multi, Response, Propstat, Prop, Resource] {
            if name == Collection && self.propstat.kind != Some(Kind::Unsupported) {
                self.propstat.kind = Some(Kind::Directory);
            } else {
                // Redirect references and unrecognized resource types are not browsable files or folders.
                self.propstat.kind = Some(Kind::Unsupported);
            }
        }
        if self.stack.as_slice() == [Multi, Response, Propstat, Prop] && name == Resource {
            self.propstat.kind = Some(if self.propstat.kind.is_none() {
                Kind::File
            } else {
                Kind::Unsupported
            });
        }
        self.stack.push(name);
        self.text.clear();
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<(), String> {
        if !matches!(
            self.stack.last(),
            Some(Href | Status | Size | Modified | Etag | Mime)
        ) {
            return Ok(());
        }
        if self.text.len().saturating_add(text.len()) > 4096 {
            return Err("A WebDAV metadata field exceeded its size limit.".into());
        }
        self.text.push_str(text);
        Ok(())
    }

    fn close(&mut self) -> Result<(), String> {
        let text = self.text.trim();
        match self.stack.as_slice() {
            [Multi, Response, Href] => self.current.href = text.to_owned(),
            [Multi, Response, Status] => self.current.success = Some(http_ok(text)),
            [Multi, Response, Propstat, Status] => self.propstat_ok = http_ok(text),
            [Multi, Response, Propstat, Prop, Size] => self.propstat.size = text.parse().ok(),
            [Multi, Response, Propstat, Prop, Modified] => {
                self.propstat.modified = modified_value(text)
            }
            [Multi, Response, Propstat, Prop, Etag] => self.propstat.etag = etag_value(text),
            [Multi, Response, Propstat, Prop, Mime]
                if !text.is_empty() && text.len() <= 240 && !text.chars().any(char::is_control) =>
            {
                self.propstat.mime = Some(text.to_owned())
            }
            [Multi, Response, Propstat] => {
                let props = std::mem::take(&mut self.propstat);
                if std::mem::take(&mut self.propstat_ok) {
                    self.current.props.merge(props);
                }
            }
            [Multi, Response] => {
                if self.resources.len() > MAX_ENTRIES {
                    return Err(
                        "The WebDAV folder contains too many entries. Choose a narrower root."
                            .into(),
                    );
                }
                self.resources.push(std::mem::take(&mut self.current));
            }
            _ => {}
        }
        self.stack.pop();
        self.text.clear();
        Ok(())
    }
}

fn parse_xml(bytes: &[u8]) -> Result<Vec<Resource>, String> {
    let xml =
        std::str::from_utf8(bytes).map_err(|_| "The WebDAV server did not return UTF-8 XML.")?;
    let mut parser = NsReader::from_str(xml.trim_start_matches('\u{feff}'));
    let mut output = Xml::default();
    loop {
        let (namespace, event) = parser
            .read_resolved_event()
            .map_err(|_| "The WebDAV server returned malformed XML.")?;
        match event {
            Event::Start(tag) => {
                output.open(node(namespace, tag.local_name().as_ref().as_bytes()))?
            }
            Event::Empty(tag) => {
                output.open(node(namespace, tag.local_name().as_ref().as_bytes()))?;
                output.close()?;
            }
            Event::End(_) => output.close()?,
            Event::Text(text) => output.text(&text.xml10_content())?,
            Event::CData(text) => output.text(&text.xml10_content())?,
            Event::GeneralRef(reference) => {
                let name = reference.xml10_content();
                let entity = format!("&{name};");
                let decoded = quick_xml::escape::unescape(&entity)
                    .map_err(|_| "The WebDAV XML contains an unsupported entity.")?;
                output.text(&decoded)?;
            }
            Event::DocType(_) => return Err("WebDAV XML with a DTD is not allowed.".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if !output.saw_root || !output.stack.is_empty() {
        return Err("The WebDAV server returned incomplete XML.".into());
    }
    Ok(output.resources)
}

fn listing(
    bytes: &[u8],
    root: &Url,
    relative: &[String],
    directory: bool,
) -> Result<Vec<Entry>, String> {
    let root_parts = decoded_path(root.path())?;
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut found_target = false;
    for resource in parse_xml(bytes)? {
        if resource.success == Some(false) {
            continue;
        }
        let props = resource.props;
        let Some(kind @ (Kind::File | Kind::Directory)) = props.kind else {
            continue;
        };
        let path = href_path(root, &root_parts, &resource.href)?;
        if !seen.insert(path.clone()) {
            return Err("The WebDAV server returned duplicate file paths.".into());
        }
        if path == relative {
            if (kind == Kind::Directory) != directory {
                return Err("The selected WebDAV resource changed type.".into());
            }
            found_target = true;
            if directory {
                continue;
            }
        } else if !directory || !path.starts_with(relative) || path.len() != relative.len() + 1 {
            return Err(
                "The WebDAV server returned a path outside the requested folder level.".into(),
            );
        }
        entries.push(Entry {
            path,
            directory: kind == Kind::Directory,
            stamp: Stamp {
                size: props.size,
                modified: props.modified,
                etag: props.etag,
            },
            mime: props.mime,
        });
    }
    if !found_target {
        return Err(
            "The configured WebDAV resource was not present in the server response.".into(),
        );
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(entries)
}

async fn properties(
    client: &Client,
    config: &Config,
    root: &Url,
    relative: &[String],
    directory: bool,
) -> Result<Vec<Entry>, String> {
    let mut url = root.clone();
    append_path(&mut url, relative, directory)?;
    let method = Method::from_bytes(b"PROPFIND").expect("constant WebDAV method");
    let mut response = send(
        request(client, config, method, url)
            .timeout(Duration::from_secs(45))
            .header("Depth", if directory { "1" } else { "0" })
            .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(DAV_BODY),
        StatusCode::MULTI_STATUS,
    )
    .await?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_XML as u64)
    {
        return Err("The WebDAV listing exceeded 4 MiB.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The WebDAV listing transfer failed.")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_XML {
            return Err("The WebDAV listing exceeded 4 MiB.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    listing(&bytes, root, relative, directory)
}

pub(super) async fn browse(
    config: &Config,
    relative: &[String],
    cancel: watch::Receiver<bool>,
) -> Result<Vec<Entry>, String> {
    let root = endpoint(&config.root)?;
    tokio::select! { biased;
        _ = cancelled(cancel) => Err(crate::engine::CANCELLED.into()),
        result = async { properties(&client(&root, config).await?, config, &root, relative, true).await } => result,
    }
}

fn precondition(stamp: &Stamp) -> Result<(header::HeaderName, &str), String> {
    if let Some(etag) = stamp
        .etag
        .as_deref()
        .filter(|tag| !tag.starts_with("W/") && etag_value(tag).is_some())
    {
        return Ok((header::IF_MATCH, etag));
    }
    if let Some(modified) = stamp
        .modified
        .as_deref()
        .filter(|value| stamp.size.is_some() && modified_value(value).is_some())
    {
        return Ok((header::IF_UNMODIFIED_SINCE, modified));
    }
    Err("This WebDAV server supplied no verifiable file version. You can keep a reference or refresh its metadata before copying.".into())
}

async fn check_version(
    client: &Client,
    config: &Config,
    root: &Url,
    relative: &[String],
    expected: &Stamp,
) -> Result<(), String> {
    let entries = properties(client, config, root, relative, false).await?;
    if entries
        .first()
        .is_some_and(|entry| &entry.stamp == expected)
    {
        Ok(())
    } else {
        Err(CHANGED.into())
    }
}

fn verify_response(
    headers: &header::HeaderMap,
    content_length: Option<u64>,
    expected: &Stamp,
) -> Result<(), String> {
    if headers.contains_key(header::CONTENT_RANGE)
        || headers
            .get(header::CONTENT_ENCODING)
            .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err("The WebDAV server returned a partial or encoded file response.".into());
    }
    if content_length.is_some_and(|actual| {
        actual > MAX_DOWNLOAD || expected.size.is_some_and(|size| size != actual)
    }) {
        return Err(CHANGED.into());
    }
    let (validator, _) = precondition(expected)?;
    if let Some(etag) = &expected.etag {
        let actual = headers.get(header::ETAG);
        if actual.is_some_and(|actual| actual.as_bytes() != etag.as_bytes())
            || actual.is_none() && validator == header::IF_MATCH
        {
            return Err(CHANGED.into());
        }
    }
    if let Some(modified) = &expected.modified {
        let actual = headers.get(header::LAST_MODIFIED);
        if actual.is_some_and(|actual| {
            actual.to_str().ok().and_then(modified_value).as_ref() != Some(modified)
        }) || actual.is_none() && validator == header::IF_UNMODIFIED_SINCE
        {
            return Err(CHANGED.into());
        }
    }
    Ok(())
}

pub(super) async fn download(
    config: &Config,
    relative: &[String],
    expected: &Stamp,
    destination: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    if relative.is_empty() || !valid_relative(relative) {
        return Err("Select an individual WebDAV file inside the authorized root.".into());
    }
    if expected.size.is_some_and(|size| size > MAX_DOWNLOAD) {
        return Err("The WebDAV file exceeds the 128 GiB copy limit.".into());
    }
    let (condition, validator) = precondition(expected)?;
    let root = endpoint(&config.root)?;
    let mut url = root.clone();
    append_path(&mut url, relative, false)?;
    let prepare = async {
        let client = client(&root, config).await?;
        check_version(&client, config, &root, relative, expected).await?;
        let response = send(
            request(&client, config, Method::GET, url).header(condition, validator),
            StatusCode::OK,
        )
        .await?;
        verify_response(response.headers(), response.content_length(), expected)?;
        Ok::<_, String>((client, response))
    };
    let (client, mut response) = tokio::select! { biased;
        _ = cancelled(cancel.clone()) => return Err(crate::engine::CANCELLED.into()),
        result = prepare => result?,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "Could not create the new local WebDAV copy.")?;
    let copy = async {
        let mut written = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The WebDAV file transfer was interrupted.")?
        {
            written = written
                .checked_add(chunk.len() as u64)
                .filter(|length| {
                    *length <= MAX_DOWNLOAD && expected.size.is_none_or(|size| *length <= size)
                })
                .ok_or("The WebDAV file exceeded its inspected size or the copy limit.")?;
            file.write_all(&chunk)
                .map_err(|_| "Could not write the local WebDAV copy.")?;
        }
        if expected.size.is_some_and(|size| size != written) {
            return Err(CHANGED.into());
        }
        check_version(&client, config, &root, relative, expected).await?;
        file.sync_all()
            .map_err(|_| "Could not finalize the local WebDAV copy.".into())
    };
    let result = tokio::select! { biased;
        _ = cancelled(cancel) => Err(crate::engine::CANCELLED.into()),
        result = copy => result,
    };
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "https://files.example.com:8443/dav/My%20Folder/";
    const SELF: &str = r#"<d:response><d:href>https://files.example.com:8443/dav/My%20Folder/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#;

    fn config(root: &str) -> Config {
        Config {
            root: root.into(),
            username: "user".into(),
            password: "explicit-app-password".into(),
            certificate_pem: None,
        }
    }

    fn xml(responses: &str) -> String {
        format!(r#"<d:multistatus xmlns:d="DAV:" xmlns:x="other">{responses}</d:multistatus>"#)
    }

    fn file(href: &str) -> String {
        format!(
            r#"<d:response><d:href>{href}</d:href><d:propstat><d:prop>
        <d:resourcetype/><d:getcontentlength>81</d:getcontentlength><d:getetag>&quot;version-1&quot;</d:getetag>
        <d:getlastmodified>Wed, 21 Oct 2015 07:28:00 GMT</d:getlastmodified><d:getcontenttype>application/pdf</d:getcontenttype>
        </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
        <d:propstat><d:prop><d:getcontentlength>4000</d:getcontentlength></d:prop><d:status>HTTP/1.1 403 Forbidden</d:status></d:propstat>
        <d:propstat><d:prop><x:getcontentlength>2000</x:getcontentlength></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
        </d:response>"#
        )
    }

    #[test]
    fn normalizes_only_credential_free_https_root_and_preserves_folder_slash() {
        let normalized = normalize(config(ROOT.trim_end_matches('/'))).unwrap();
        assert_eq!(normalized.root, ROOT);
        let mut url = endpoint(&normalized.root).unwrap();
        append_path(&mut url, &[], true).unwrap();
        assert_eq!(url.as_str(), ROOT);
        append_path(&mut url, &["café & notes".into()], true).unwrap();
        assert_eq!(
            decoded_path(url.path()).unwrap(),
            ["dav", "My Folder", "café & notes"]
        );
        assert!(url.path().ends_with('/'));
        for root in [
            "http://files.example.com/dav/",
            "https://user:secret@files.example.com/dav/",
            "https://files.example.com/dav/?token=secret",
            "https://files.example.com/dav/#section",
            "https://files.example.com/dav/%2e%2e/private/",
            "https://files.example.com/dav/%2fprivate/",
            "https://files.example.com/dav/%5cprivate/",
            "https:///files.example.com/dav/",
        ] {
            assert!(normalize(config(root)).is_err(), "accepted {root}");
        }
    }

    #[test]
    fn reads_only_successful_dav_properties_and_skips_redirect_references() {
        let link = r#"<d:response><d:href>/dav/My%20Folder/redirect</d:href><d:propstat><d:prop><d:resourcetype><d:redirectref/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#;
        let response = xml(&format!(
            "{SELF}{}{link}",
            file("/dav/My%20Folder/caf%C3%A9%20%26%20book.pdf")
        ));
        let entries = listing(response.as_bytes(), &endpoint(ROOT).unwrap(), &[], true).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.path, ["café & book.pdf"]);
        assert!(!entry.directory);
        assert_eq!(entry.mime.as_deref(), Some("application/pdf"));
        assert_eq!(entry.stamp.size, Some(81));
        assert_eq!(entry.stamp.etag.as_deref(), Some("\"version-1\""));
        assert_eq!(
            precondition(&entry.stamp).unwrap(),
            (header::IF_MATCH, "\"version-1\"")
        );
    }

    #[test]
    fn rejects_paths_beyond_the_exact_origin_root_and_depth() {
        let root = endpoint(ROOT).unwrap();
        for href in [
            "https://other.example.com:8443/dav/My%20Folder/a.pdf",
            "https://files.example.com:443/dav/My%20Folder/a.pdf",
            "//files.example.com:8443/dav/My%20Folder/a.pdf",
            "/dav/My%20Folder2/a.pdf",
            "/dav/My%20Folder/../a.pdf",
            "/dav/My%20Folder/a%2Fb.pdf",
            "/dav/My%20Folder/a%5Cb.pdf",
            "/dav/My%20Folder/sub/a.pdf",
        ] {
            let response = xml(&format!("{SELF}{}", file(href)));
            assert!(
                listing(response.as_bytes(), &root, &[], true).is_err(),
                "accepted {href}"
            );
        }
        let malicious = r#"<!DOCTYPE d:multistatus [<!ENTITY local SYSTEM "file:///C:/secret">]><d:multistatus xmlns:d="DAV:">&local;</d:multistatus>"#;
        assert!(listing(malicious.as_bytes(), &root, &[], true).is_err());
    }

    #[test]
    fn needs_a_strong_etag_or_size_and_last_modified_to_copy() {
        let mut stamp = Stamp {
            size: None,
            modified: None,
            etag: Some("W/\"version\"".into()),
        };
        assert!(precondition(&stamp).is_err());
        stamp.modified = Some("Wed, 21 Oct 2015 07:28:00 GMT".into());
        assert!(precondition(&stamp).is_err());
        stamp.size = Some(81);
        assert_eq!(precondition(&stamp).unwrap().0, header::IF_UNMODIFIED_SINCE);
        stamp.etag = Some("\"version\"".into());
        assert_eq!(precondition(&stamp).unwrap().0, header::IF_MATCH);
    }

    #[test]
    fn a_get_requires_the_inspected_validator_and_length() {
        let stamp = Stamp {
            size: Some(81),
            modified: None,
            etag: Some("\"version-1\"".into()),
        };
        let mut headers = header::HeaderMap::new();
        assert_eq!(
            verify_response(&headers, Some(81), &stamp).unwrap_err(),
            CHANGED
        );
        headers.insert(
            header::ETAG,
            header::HeaderValue::from_static("\"version-2\""),
        );
        assert_eq!(
            verify_response(&headers, Some(81), &stamp).unwrap_err(),
            CHANGED
        );
        headers.insert(
            header::ETAG,
            header::HeaderValue::from_static("\"version-1\""),
        );
        assert_eq!(
            verify_response(&headers, Some(80), &stamp).unwrap_err(),
            CHANGED
        );
        assert!(verify_response(&headers, Some(81), &stamp).is_ok());
        headers.insert(
            header::CONTENT_RANGE,
            header::HeaderValue::from_static("bytes 0-79/81"),
        );
        assert!(verify_response(&headers, Some(81), &stamp).is_err());
    }

    #[tokio::test]
    async fn cancelled_browse_stops_before_dns_or_connection() {
        let (_, cancel) = watch::channel(true);
        assert_eq!(
            browse(&config(ROOT), &[], cancel).await.unwrap_err(),
            crate::engine::CANCELLED
        );
    }
}
