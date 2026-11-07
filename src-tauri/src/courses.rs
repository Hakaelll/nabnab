//! Course downloads only use URLs explicitly returned as downloadable by a provider.
use crate::{library, options, queue, sessions, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
#[path = "course_members.rs"]
pub mod members;
#[path = "course_browser.rs"]
pub mod browser;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Course {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub url: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    pub sections: Vec<CourseSection>,
    pub created_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseSection {
    pub id: String,
    pub title: String,
    pub lessons: Vec<CourseLesson>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseLesson {
    pub id: String,
    pub title: String,
    pub learned: bool,
    pub assets: Vec<CourseAsset>,
    #[serde(default)]
    pub notes: Vec<CourseNote>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseAsset {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub restriction: Option<String>,
    pub library_id: Option<String>,
    pub url_ref: Option<String>,
    pub job_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseNote {
    pub id: String,
    pub text: String,
    pub time: Option<f64>,
    pub bookmark_id: Option<String>,
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn label(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}
fn visible_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 192
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
}

impl Course {
    pub fn validate(&self) -> Result<(), String> {
        if !visible_id(&self.id)
            || self.title.is_empty()
            || self.title.chars().count() > 240
            || !matches!(
                self.provider.as_str(),
                "local" | "udemy" | "hotmart" | "kiwify"
                    | "rocketseat" | "coursera" | "domestika" | "skillshare"
            )
            || self.sections.len() > 1000
        {
            return Err("The saved course has invalid metadata.".into());
        }
        let mut ids = HashSet::new();
        let mut count = 0;
        for section in &self.sections {
            if !visible_id(&section.id)
                || !ids.insert(section.id.clone())
                || section.title.len() > 2048
            {
                return Err("The saved course has an invalid section.".into());
            }
            for lesson in &section.lessons {
                count += 1;
                if count > 10000
                    || !visible_id(&lesson.id)
                    || !ids.insert(lesson.id.clone())
                    || lesson.title.len() > 4096
                    || lesson.assets.len() > 100
                    || lesson.notes.len() > 100
                {
                    return Err("The saved course has an invalid lesson.".into());
                }
                for asset in &lesson.assets {
                    if !visible_id(&asset.id)
                        || !ids.insert(asset.id.clone())
                        || asset.title.len() > 4096
                        || !matches!(
                            asset.status.as_str(),
                            "local" | "downloadable" | "drm" | "unavailable"
                        )
                        || asset
                            .url_ref
                            .as_deref()
                            .is_some_and(|v| !sessions::valid_url_reference(v))
                    {
                        return Err("The saved course has an invalid asset.".into());
                    }
                }
                for note in &lesson.notes {
                    if !visible_id(&note.id)
                        || note.text.chars().count() > 4000
                        || note.time.is_some_and(|t| !t.is_finite() || t < 0.0)
                    {
                        return Err("The saved course has an invalid note.".into());
                    }
                }
            }
        }
        Ok(())
    }
}

fn natural_key(name: &str) -> String {
    let mut key = String::new();
    let mut digits = String::new();
    for character in name.to_lowercase().chars().chain(std::iter::once('\0')) {
        if character.is_ascii_digit() {
            digits.push(character);
        } else {
            if !digits.is_empty() {
                key.push_str(&format!(
                    "{:020}",
                    digits.parse::<u64>().unwrap_or(u64::MAX)
                ));
                digits.clear();
            }
            key.push(character);
        }
    }
    key
}

fn local_course(root: &Path, items: &[library::LibraryItem]) -> Result<Course, String> {
    let id = format!("course-{}", digest(&root.to_string_lossy().to_lowercase()));
    let mut sections = BTreeMap::<String, Vec<&library::LibraryItem>>::new();
    for item in items.iter().filter(|item| item.path.starts_with(root)) {
        let relative = item
            .path
            .strip_prefix(root)
            .map_err(|_| "Invalid local course path.")?;
        let section = relative
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Lessons".into());
        sections.entry(section).or_default().push(item);
    }
    let mut course_sections = Vec::new();
    for (title, mut items) in sections {
        items.sort_by(|a, b| natural_key(&a.name).cmp(&natural_key(&b.name)));
        let mut lessons = Vec::new();
        for item in items.iter().filter(|item| {
            matches!(item.kind.as_str(), "video" | "audio" | "document")
                || item.kind == "file"
                    && item
                        .path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|extension| {
                            matches!(extension.to_lowercase().as_str(), "pdf" | "epub")
                        })
        }) {
            let mut assets = vec![CourseAsset {
                id: format!("asset-{}", item.id),
                title: item.name.clone(),
                kind: item.kind.clone(),
                status: "local".into(),
                restriction: None,
                library_id: Some(item.id.clone()),
                url_ref: None,
                job_ids: Vec::new(),
            }];
            for attachment in items.iter().filter(|other| {
                other.id != item.id
                    && other.path.file_stem() == item.path.file_stem()
                    && matches!(other.kind.as_str(), "subtitle" | "image")
            }) {
                assets.push(CourseAsset {
                    id: format!("attachment-{}-{}", item.id, attachment.id),
                    title: attachment.name.clone(),
                    kind: attachment.kind.clone(),
                    status: "local".into(),
                    restriction: None,
                    library_id: Some(attachment.id.clone()),
                    url_ref: None,
                    job_ids: Vec::new(),
                });
            }
            lessons.push(CourseLesson {
                id: format!("lesson-{}", item.id),
                title: item
                    .path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| item.name.clone()),
                learned: false,
                assets,
                notes: Vec::new(),
            });
        }
        if !lessons.is_empty() {
            course_sections.push(CourseSection {
                id: format!("section-{}", digest(&title)),
                title,
                lessons,
            });
        }
    }
    course_sections.sort_by_key(|section| natural_key(&section.title));
    if course_sections.is_empty() {
        return Err("This folder contains no supported audio, video or document lessons.".into());
    }
    Ok(Course {
        id,
        title: label(
            &root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Local course".into()),
            240,
        ),
        provider: "local".into(),
        url: None,
        slug: None,
        sections: course_sections,
        created_at: queue::now(),
    })
}

fn merge_study(previous: &Course, current: &mut Course) {
    for lesson in current
        .sections
        .iter_mut()
        .flat_map(|section| &mut section.lessons)
    {
        if let Some(old) = previous
            .sections
            .iter()
            .flat_map(|section| &section.lessons)
            .find(|old| old.id == lesson.id)
        {
            lesson.learned = old.learned;
            lesson.notes = old.notes.clone();
            for asset in &mut lesson.assets {
                if let Some(prior) = old.assets.iter().find(|prior| prior.id == asset.id) {
                    asset.library_id = asset
                        .library_id
                        .clone()
                        .or_else(|| prior.library_id.clone());
                    asset.job_ids = prior.job_ids.clone();
                }
            }
        }
    }
    for old_section in &previous.sections {
        if let Some(section) = current
            .sections
            .iter_mut()
            .find(|section| section.id == old_section.id)
        {
            for old_lesson in &old_section.lessons {
                if !section
                    .lessons
                    .iter()
                    .any(|lesson| lesson.id == old_lesson.id)
                {
                    section.lessons.push(old_lesson.clone());
                }
            }
        } else {
            current.sections.push(old_section.clone());
        }
    }
}

fn save_course(repo: &mut crate::store::Repository, mut course: Course) -> Result<Course,String> {
    let index = repo.data.courses.iter().position(|old| old.id == course.id);
    let previous = index.map(|index| repo.data.courses[index].clone());
    if let Some(previous) = &previous { merge_study(previous,&mut course); }
    course.validate()?;
    if let Some(index) = index { repo.data.courses[index] = course.clone(); } else { repo.data.courses.push(course.clone()); }
    if let Err(error) = repo.save() {
        if let (Some(index),Some(previous)) = (index,previous) { repo.data.courses[index] = previous; } else { repo.data.courses.pop(); }
        return Err(error);
    }
    Ok(course)
}

#[tauri::command]
pub fn list_courses(state: State<Arc<AppState>>) -> Result<Vec<Course>, String> {
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Open the app first.")?;
    let mut courses = repo.data.courses.clone();
    for asset in courses
        .iter_mut()
        .flat_map(|c| &mut c.sections)
        .flat_map(|s| &mut s.lessons)
        .flat_map(|l| &mut l.assets)
    {
        if asset.library_id.is_none() {
            asset.library_id = asset
                .job_ids
                .iter()
                .filter_map(|id| {
                    repo.data
                        .jobs
                        .iter()
                        .find(|job| job.id == *id && job.state == queue::JobState::Completed)
                })
                .flat_map(|job| &job.outputs)
                .find_map(|path| {
                    repo.data
                        .library
                        .iter()
                        .find(|item| item.path == *path && item.path.is_file())
                        .map(|item| item.id.clone())
                });
        }
    }
    Ok(courses)
}

#[tauri::command]
pub async fn import_local_course(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Course, String> {
    let root = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .ok_or("Cancelled.")?
            .into_path()
            .map_err(|_| "Choose a local course folder.".to_owned())
    })
    .await
    .map_err(|_| "The course folder picker failed.")??;
    let root = library::authorized_path(&root)?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    library::import_paths(repo, &[root.clone()])?;
    let course = local_course(&root, &repo.data.library)?;
    save_course(repo,course)
}

#[tauri::command]
pub fn course_learned(
    state: State<Arc<AppState>>,
    course_id: String,
    lesson_id: String,
    learned: bool,
) -> Result<(), String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let index = repo.data.courses.iter().position(|course| course.id == course_id).ok_or("Course not found.")?;
    let previous = repo.data.courses[index].clone();
    let lesson = repo
        .data
        .courses
        .iter_mut()
        .find(|course| course.id == course_id)
        .and_then(|course| {
            course
                .sections
                .iter_mut()
                .flat_map(|section| &mut section.lessons)
                .find(|lesson| lesson.id == lesson_id)
        })
        .ok_or("Course lesson not found.")?;
    lesson.learned = learned;
    if let Err(error) = repo.save() { repo.data.courses[index] = previous; return Err(error); }
    Ok(())
}

#[tauri::command]
pub fn save_course_note(
    state: State<Arc<AppState>>,
    course_id: String,
    lesson_id: String,
    text: String,
    time: Option<f64>,
    note_id: Option<String>,
    remove: bool,
) -> Result<(), String> {
    let text = text.trim();
    if text.chars().count() > 4000 || time.is_some_and(|t| !t.is_finite() || t < 0.0) {
        return Err("Use a note of at most 4000 characters and a valid media time.".into());
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let index = repo
        .data
        .courses
        .iter()
        .position(|course| course.id == course_id)
        .ok_or("Course not found.")?;
    let course_before = repo.data.courses[index].clone();
    let lesson = repo.data.courses[index]
        .sections
        .iter_mut()
        .flat_map(|section| &mut section.lessons)
        .find(|lesson| lesson.id == lesson_id)
        .ok_or("Course lesson not found.")?;
    let id = note_id.unwrap_or_else(|| format!("note-{}-{}", queue::now(), lesson.notes.len()));
    if !visible_id(&id) || id.len() > 180 {
        return Err("Invalid course note identifier.".into());
    }
    let previous = lesson.notes.iter().find(|note| note.id == id).cloned();
    let assets = lesson.assets.clone();
    let note_count = lesson.notes.len();
    let bookmark_id = time.map(|_| format!("course-{}", id));
    if !remove && !text.is_empty() && note_count >= 100 && previous.is_none() {
        return Err("Use at most 100 notes per lesson.".into());
    }
    let local_id = assets.iter().find_map(|asset| asset.library_id.clone().or_else(|| asset.job_ids.iter().filter_map(|id| repo.data.jobs.iter().find(|job| job.id == *id && job.state == queue::JobState::Completed)).flat_map(|job| &job.outputs).find_map(|path| repo.data.library.iter().find(|item| item.path == *path).map(|item| item.id.clone()))));
    if let Some(item) = local_id
        .as_ref()
        .and_then(|id| repo.data.library.iter().find(|item| item.id == *id))
    {
        if time.is_some_and(|time| item.duration.is_some_and(|duration| time > duration)) {
            return Err("The note time is outside the lesson duration.".into());
        }
    }
    let lesson = repo.data.courses[index]
        .sections
        .iter_mut()
        .flat_map(|section| &mut section.lessons)
        .find(|lesson| lesson.id == lesson_id)
        .ok_or("Course lesson not found.")?;
    lesson.notes.retain(|note| note.id != id);
    if !remove && !text.is_empty() {
        lesson.notes.push(CourseNote {
            id,
            text: text.to_owned(),
            time,
            bookmark_id: bookmark_id.clone(),
        });
    }
    let mut library_before = None;
    if let Some(item) =
        local_id.and_then(|id| repo.data.library.iter_mut().find(|item| item.id == id))
    {
        library_before = Some(item.clone());
        if let Some(prior) = previous.and_then(|note| note.bookmark_id) {
            item.details
                .bookmarks
                .retain(|bookmark| bookmark.id != prior);
        }
        if !remove && !text.is_empty() {
            if let (Some(id), Some(time)) = (bookmark_id, time) {
                item.details
                    .bookmarks
                    .push(crate::library_features::Bookmark {
                        id,
                        name: label(text, 80),
                        time,
                    });
            }
        }
    }
    if let Err(error) = repo.save() {
        repo.data.courses[index] = course_before;
        if let Some(previous) = library_before {
            if let Some(item) = repo
                .data
                .library
                .iter_mut()
                .find(|item| item.id == previous.id)
            {
                *item = previous;
            }
        }
        return Err(error);
    }
    Ok(())
}

fn course_location(input: &str) -> Result<(String, Option<String>), String> {
    let normalized = crate::model::validate_url(input)?;
    let parsed = url::Url::parse(&normalized).map_err(|_| "Use a complete course URL.")?;
    let host = parsed.host_str().unwrap_or_default();
    if host == "www.udemy.com" || host == "udemy.com" {
        let parts: Vec<_> = parsed
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|p| !p.is_empty())
            .collect();
        let slug = match parts.as_slice() {
            ["course", slug, ..] => *slug,
            [slug] => *slug,
            _ => return Err("Open a Udemy course URL, not its home page.".into()),
        };
        if slug.len() > 240
            || !slug
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return Err("Invalid Udemy course slug.".into());
        }
        return Ok((
            format!("https://www.udemy.com/course/{slug}/"),
            slug.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| slug.to_owned()),
        ));
    }
    if host == "hotmart.com" || host.ends_with(".hotmart.com") {
        return Err("Hotmart course access is not connected yet. Its seller API does not establish an authorized student curriculum download route. Import an already downloaded course folder meanwhile.".into());
    }
    if host == "kiwify.com.br"
        || host.ends_with(".kiwify.com.br")
        || host == "kiwify.com"
        || host.ends_with(".kiwify.com")
    {
        return Err("Kiwify course access is not connected yet. The documented public API covers seller products and sales, not downloadable student lessons. Import an already downloaded course folder meanwhile.".into());
    }
    Err("Choose a Udemy course URL. Hotmart and Kiwify require verified independent student download adapters.".into())
}

fn udemy_course_id(html: &str) -> Option<String> {
    for marker in [
        "data-course-id=\"",
        "data-course-id='",
        "&quot;courseId&quot;:",
        "\"courseId\":",
    ] {
        let Some(rest) = html.split_once(marker).map(|(_, rest)| rest.trim_start()) else {
            continue;
        };
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(20)
            .collect();
        if !digits.is_empty() {
            return Some(digits);
        }
    }
    None
}

fn udemy_cookie_header(file: &sessions::CookieFile, path: &str) -> Result<String, String> {
    let bytes = fs::read(&file.path).map_err(|_| "Could not read the authorized Udemy session.")?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("The Udemy session exceeds its size limit.".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "Invalid authorized Udemy session.")?;
    let now = queue::now() / 1000;
    let mut cookies = Vec::new();
    for line in text.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 7 {
            continue;
        }
        let domain = fields[0].trim_start_matches('.');
        if !((domain == "udemy.com" && fields[1] == "TRUE") || domain == "www.udemy.com")
            || !path.starts_with(fields[2])
        {
            continue;
        }
        let expires = fields[4].parse::<u64>().unwrap_or(0);
        if expires > 0 && expires < now {
            continue;
        }
        if fields[5]
            .bytes()
            .any(|b| b.is_ascii_control() || b";= ".contains(&b))
            || fields[6].bytes().any(|b| b.is_ascii_control() || b == b';')
        {
            return Err("The Udemy session contains invalid cookie values.".into());
        }
        cookies.push(format!("{}={}", fields[5], fields[6]));
    }
    if cookies.is_empty() {
        return Err("Import a valid Udemy session in Settings before opening the course.".into());
    }
    Ok(cookies.join("; "))
}

async fn udemy_get(url: &str, cookies: &sessions::CookieFile) -> Result<String, String> {
    let parsed =
        url::Url::parse(&crate::model::validate_url(url)?).map_err(|_| "Invalid Udemy request.")?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("www.udemy.com")
        || parsed.port_or_known_default() != Some(443)
    {
        return Err("Udemy sessions cannot be forwarded to another origin.".into());
    }
    let addresses: Vec<_> = tokio::time::timeout(
        Duration::from_secs(8),
        tokio::net::lookup_host(("www.udemy.com", 443)),
    )
    .await
    .map_err(|_| "Udemy DNS lookup timed out.")?
    .map_err(|_| "Could not resolve Udemy.")?
    .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !crate::providers::is_public_ip(address.ip()))
    {
        return Err("Udemy resolved to a reserved network destination.".into());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs("www.udemy.com", &addresses)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Could not prepare the Udemy request.")?;
    let cookie_header = udemy_cookie_header(cookies, parsed.path())?;
    let mut request = client
        .get(parsed.clone())
        .header(reqwest::header::COOKIE, &cookie_header)
        .header("X-Udemy-Snail-Case", "true")
        .header("X-Requested-With", "XMLHttpRequest")
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    for cookie in cookie_header.split("; ") {
        if let Some(token) = cookie.strip_prefix("access_token=") {
            request = request
                .header("X-Udemy-Bearer-Token", token)
                .header("X-Udemy-Authorization", format!("Bearer {token}"));
        }
        if let Some(client_id) = cookie.strip_prefix("client_id=") {
            request = request.header("X-Udemy-Client-Id", client_id);
        }
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| "The authorized Udemy request failed.")?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() { 401 | 403 => "Udemy denied access. Refresh the authorized session or check enrollment and download permissions. No enrollment action was performed.".into(), 429 => "Udemy requested a pause. Retry later; no lesson downloads were queued.".into(), _ => format!("Udemy returned HTTP {}. No account actions were performed.",response.status().as_u16()) });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The Udemy response was interrupted.")?
    {
        if bytes.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 {
            return Err("The Udemy response exceeds 8 MiB.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "Udemy returned unreadable course data.".into())
}

fn asset_download(asset: &Value) -> Option<String> {
    if asset["course_is_drmed"] == true
        || asset["has_drm"] == true
        || asset["is_downloadable"] == false
    {
        return None;
    }
    let downloads = asset["download_urls"].as_object()?;
    let mut candidates = Vec::new();
    for values in downloads.values().filter_map(Value::as_array) {
        for value in values {
            let Some(url) = value["file"]
                .as_str()
                .or_else(|| value["url"].as_str())
                .or_else(|| value["src"].as_str())
            else {
                continue;
            };
            if let Ok(url) = crate::model::validate_url(url) {
                let rank = value["label"]
                    .as_str()
                    .and_then(|v| v.trim_end_matches('p').parse::<u32>().ok())
                    .unwrap_or(0);
                candidates.push((rank, url));
            }
        }
    }
    candidates.sort_by_key(|(rank, _)| *rank);
    candidates.pop().map(|(_, url)| url)
}

fn parse_udemy_course(
    value: &Value,
    course_id: &str,
    source_url: &str,
) -> Result<(Course, Vec<(String, String)>), String> {
    let entries = value["results"]
        .as_array()
        .ok_or("Udemy did not return an accessible curriculum.")?;
    let mut sections = vec![CourseSection {
        id: "section-introduction".into(),
        title: "Introduction".into(),
        lessons: Vec::new(),
    }];
    let mut downloads = Vec::new();
    let mut lesson_order = 0;
    for entry in entries.iter().take(10000) {
        let id = entry["id"]
            .as_u64()
            .map(|v| v.to_string())
            .or_else(|| entry["id"].as_str().map(str::to_owned))
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
        let Some(id) = id else {
            continue;
        };
        if entry["_class"] == "chapter" {
            sections.push(CourseSection {
                id: format!("section-{id}"),
                title: label(entry["title"].as_str().unwrap_or("Section"), 240),
                lessons: Vec::new(),
            });
            continue;
        }
        if entry["_class"] != "lecture" {
            continue;
        }
        lesson_order += 1;
        let position = entry["object_index"].as_u64().unwrap_or(lesson_order);
        let title = format!(
            "{position:03} {}",
            label(entry["title"].as_str().unwrap_or("Lesson"), 500)
        );
        let mut assets = Vec::new();
        let main = std::iter::once(&entry["asset"]);
        let additional = entry["supplementary_assets"]
            .as_array()
            .map(|v| v.as_slice())
            .unwrap_or_default();
        for (index, asset) in main.chain(additional.iter()).enumerate() {
            if asset.is_null() {
                continue;
            }
            let asset_id = asset["id"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_else(|| index.to_string());
            let asset_id = format!("asset-{id}-{asset_id}");
            let drm = asset["course_is_drmed"] == true || asset["has_drm"] == true;
            let download = asset_download(asset);
            let kind = match asset["asset_type"].as_str().unwrap_or_default() {
                "Video" => "video",
                "Audio" => "audio",
                _ => "file",
            };
            if let Some(url) = &download {
                downloads.push((asset_id.clone(), url.clone()));
            }
            assets.push(CourseAsset {
                id: asset_id,
                title: if index == 0 {
                    title.clone()
                } else {
                    label(
                        asset["title"]
                            .as_str()
                            .or_else(|| asset["filename"].as_str())
                            .unwrap_or("Attachment"),
                        500,
                    )
                },
                kind: kind.into(),
                status: if drm {
                    "drm"
                } else if download.is_some() {
                    "downloadable"
                } else {
                    "unavailable"
                }
                .into(),
                restriction: entry["nabnab_error"].as_str().map(str::to_owned),
                library_id: None,
                url_ref: None,
                job_ids: Vec::new(),
            });
        }
        sections
            .last_mut()
            .ok_or("Missing course section.")?
            .lessons
            .push(CourseLesson {
                id: format!("lesson-{id}"),
                title,
                learned: false,
                assets,
                notes: Vec::new(),
            });
    }
    sections.retain(|section| !section.lessons.is_empty());
    if sections.is_empty() {
        return Err("Udemy returned no accessible lessons in this curriculum page.".into());
    }
    Ok((
        Course {
            id: format!("udemy-{course_id}"),
            title: label(value["title"].as_str().unwrap_or("Udemy course"), 240),
            provider: "udemy".into(),
            url: Some(source_url.into()),
            slug: None,
            sections,
            created_at: queue::now(),
        },
        downloads,
    ))
}

#[tauri::command]
pub async fn inspect_course(
    state: State<'_, Arc<AppState>>,
    url: String,
    limit: usize,
) -> Result<Course, String> {
    if !(1..=1000).contains(&limit) {
        return Err("Explore between 1 and 1000 course lessons at a time.".into());
    }
    let (course_url, numeric_id) = course_location(&url)?;
    let config = crate::config_root(&state)?;
    let cookies = sessions::materialize(&config, &course_url)?
        .ok_or("Import a Udemy session in Settings before opening an enrolled course.")?;
    let course_id = if let Some(id) = numeric_id {
        id
    } else {
        udemy_course_id(&udemy_get(&course_url,&cookies).await?).ok_or("Could not identify this enrolled Udemy course. A CAPTCHA or changed course page may require an updated authorized session.")?
    };
    let mut api = url::Url::parse(&format!(
        "https://www.udemy.com/api-2.0/courses/{course_id}/cached-subscriber-curriculum-items/"
    ))
    .map_err(|_| "Invalid curriculum request.")?;
    api.query_pairs_mut()
        .append_pair("page_size", &limit.min(100).to_string())
        .append_pair("fields[chapter]", "title,object_index")
        .append_pair(
            "fields[lecture]",
            "title,object_index,asset,supplementary_assets",
        )
        .append_pair(
            "fields[asset]",
            "title,filename,asset_type,download_urls,is_downloadable,course_is_drmed",
        );
    let mut results = Vec::new();
    let mut next = Some(api.to_string());
    let mut complete = false;
    for _ in 0..20 {
        let Some(page) = next.take() else {
            complete = true;
            break;
        };
        let value: Value = serde_json::from_str(&udemy_get(&page, &cookies).await?)
            .map_err(|_| "Udemy returned an unreadable curriculum.")?;
        let entries = value["results"]
            .as_array()
            .ok_or("Udemy did not return an accessible enrolled curriculum.")?;
        for entry in entries {
            if entry["_class"] == "lecture"
                && results
                    .iter()
                    .filter(|entry: &&Value| entry["_class"] == "lecture")
                    .count()
                    >= limit
            {
                break;
            }
            results.push(entry.clone());
        }
        next = value["next"].as_str().map(str::to_owned);
        if next.is_none() {
            complete = true;
            break;
        }
        if results
            .iter()
            .filter(|entry| entry["_class"] == "lecture")
            .count()
            >= limit
        {
            break;
        }
    }
    for entry in results
        .iter_mut()
        .filter(|entry| entry["_class"] == "lecture")
    {
        if entry["asset"]["download_urls"].is_object() || entry["asset"]["course_is_drmed"] == true
        {
            continue;
        }
        let Some(lecture_id) = entry["id"].as_u64() else {
            continue;
        };
        let mut endpoint = url::Url::parse(&format!("https://www.udemy.com/api-2.0/users/me/subscribed-courses/{course_id}/lectures/{lecture_id}/")).map_err(|_| "Invalid lecture request.")?;
        endpoint
            .query_pairs_mut()
            .append_pair("fields[lecture]", "title,asset,supplementary_assets")
            .append_pair(
                "fields[asset]",
                "title,filename,asset_type,download_urls,is_downloadable,course_is_drmed",
            );
        match udemy_get(endpoint.as_str(), &cookies).await {
            Ok(raw) => {
                if let Ok(detail) = serde_json::from_str::<Value>(&raw) {
                    if detail["asset"].is_object() {
                        entry["asset"] = detail["asset"].clone();
                    }
                    if detail["supplementary_assets"].is_array() {
                        entry["supplementary_assets"] = detail["supplementary_assets"].clone();
                    }
                }
            }
            Err(error) => {
                let limited = error.contains("requested a pause");
                entry["nabnab_error"] = Value::String(error);
                if limited {
                    break;
                }
            }
        }
    }
    let value = serde_json::json!({"results":results, "title":format!("Udemy {course_id}{}",if complete { "" } else { " · partial curriculum" })});
    let (mut course, downloads) = parse_udemy_course(&value, &course_id, &course_url)?;
    for (asset_id, url) in downloads {
        let reference = sessions::seal_url(
            &config,
            &format!("course-{}-{:x}", asset_id, Sha256::digest(url.as_bytes())),
            &url,
        )?;
        if let Some(asset) = course
            .sections
            .iter_mut()
            .flat_map(|s| &mut s.lessons)
            .flat_map(|l| &mut l.assets)
            .find(|asset| asset.id == asset_id)
        {
            asset.url_ref = Some(reference);
        }
    }
    course.validate()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    save_course(repo,course)
}

#[tauri::command]
pub fn queue_course_assets(
    app: AppHandle,
    state: State<Arc<AppState>>,
    course_id: String,
    asset_ids: Vec<String>,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    if asset_ids.is_empty() || asset_ids.len() > 200 {
        return Err("Choose between 1 and 200 course materials.".into());
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let course = repo
        .data
        .courses
        .iter()
        .find(|course| course.id == course_id)
        .cloned()
        .ok_or("Course not found.")?;
    let mut prepared = Vec::new();
    let mut link = Vec::new();
    for (section, lesson, asset) in course
        .sections
        .iter()
        .flat_map(|s| {
            s.lessons
                .iter()
                .flat_map(move |l| l.assets.iter().map(move |a| (s, l, a)))
        })
        .filter(|(_, _, a)| asset_ids.contains(&a.id))
    {
        if asset.status != "downloadable" {
            continue;
        }
        let active = asset.job_ids.iter().any(|id| {
            repo.data.jobs.iter().any(|job| {
                job.id == *id
                    && (job.state.active()
                        || matches!(
                            job.state,
                            queue::JobState::Pending | queue::JobState::Paused
                        )
                        || job.state == queue::JobState::Completed
                            && !job.outputs.is_empty()
                            && job.outputs.iter().all(|path| path.is_file()))
            })
        });
        if active
            || asset.library_id.as_ref().is_some_and(|id| {
                repo.data
                    .library
                    .iter()
                    .any(|item| item.id == *id && item.path.is_file())
            })
        {
            continue;
        }
        let reference = asset
            .url_ref
            .as_ref()
            .ok_or("Refresh the course to obtain this material's authorized download URL.")?;
        let url = sessions::resolve_url(repo.root(), reference)?
            .as_str()
            .to_owned();
        let content_id = options::content_identity(&url)?;
        let context = options::Context {
            url: url.clone(),
            source_url: course.url.clone().unwrap_or_default(),
            title: asset.title.clone(),
            creator: String::new(),
            list: course.title.clone(),
            content_id: content_id.clone(),
            platform: course.provider.clone(),
            content_type: asset.kind.clone(),
            tags: Vec::new(),
        };
        let mut resolved = options::resolve(repo, &context, None, None)?;
        resolved.values.format = crate::model::Format::Source;
        resolved.values.height = None;
        resolved.values.audio_bitrate = None;
        resolved.values.collection = Some(label(&course.title, 80));
        resolved.values.relative_folder = format!("Courses/{}/{}", course.id, section.id);
        resolved.values.filename_template = format!("{} {{id}}", label(&lesson.title, 120));
        prepared.push(options::PreparedDownload {
            url,
            title: asset.title.clone(),
            content_id: content_id.clone(),
            options: resolved,
            source_id: None,
            group_id: Some(course.id.clone()),
        });
        link.push((asset.id.clone(), content_id));
    }
    if prepared.is_empty() {
        return Err("The selection has no new downloadable materials. Local, protected and active materials were kept.".into());
    }
    let jobs = queue::enqueue_prepared(repo, prepared, &client_request)?;
    let course = repo
        .data
        .courses
        .iter_mut()
        .find(|course| course.id == course_id)
        .ok_or("Course not found.")?;
    for asset in course
        .sections
        .iter_mut()
        .flat_map(|s| &mut s.lessons)
        .flat_map(|l| &mut l.assets)
    {
        if let Some((_, identity)) = link.iter().find(|(id, _)| *id == asset.id) {
            asset.job_ids = jobs
                .iter()
                .filter(|job| job.content_id.as_deref() == Some(identity))
                .map(|job| job.id.clone())
                .collect();
        }
    }
    repo.save()?;
    drop(guard);
    for job in &jobs {
        let _ = app.emit("job-update", job);
    }
    state.runtime.notify.notify_one();
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_curriculum() -> Value {
        serde_json::json!({"title":"Real fixture structure","results":[
            {"_class":"chapter","id":10,"title":"Section"},
            {"_class":"lecture","id":20,"title":"Allowed","object_index":1,"asset":{"id":30,"asset_type":"Video","download_urls":{"Video":[{"label":"360","file":"https://cdn.example.org/low.mp4"},{"label":"1080","file":"https://cdn.example.org/high.mp4?token=fixture"}]}}},
            {"_class":"lecture","id":21,"title":"Stream only","asset":{"id":31,"asset_type":"Video","stream_urls":{"Video":[{"file":"https://cdn.example.org/play.mp4"}]}}},
            {"_class":"lecture","id":22,"title":"Protected","asset":{"id":32,"asset_type":"Video","course_is_drmed":true,"download_urls":{"Video":[{"file":"https://cdn.example.org/protected.mp4"}]}}}
        ]})
    }

    #[test]
    fn udemy_curriculum_only_offers_explicit_authorized_downloads() {
        let (course,downloads) = parse_udemy_course(&fixture_curriculum(),"123","https://www.udemy.com/course/fixture/").unwrap();
        assert_eq!(course.sections.len(),1);
        assert_eq!(course.sections[0].lessons.len(),3);
        assert_eq!(course.sections[0].lessons[0].id,"lesson-20");
        assert_eq!(course.sections[0].lessons[0].assets[0].status,"downloadable");
        assert_eq!(course.sections[0].lessons[1].assets[0].status,"unavailable");
        assert_eq!(course.sections[0].lessons[2].assets[0].status,"drm");
        assert_eq!(downloads,vec![("asset-20-30".into(),"https://cdn.example.org/high.mp4?token=fixture".into())]);
        assert!(!serde_json::to_string(&course).unwrap().contains("token=fixture"));
    }

    #[test]
    fn course_refresh_keeps_study_notes_and_removed_lessons() {
        let (mut original,_) = parse_udemy_course(&fixture_curriculum(),"123","https://www.udemy.com/course/fixture/").unwrap();
        original.sections[0].lessons[0].learned = true;
        original.sections[0].lessons[0].notes.push(CourseNote { id:"note-1".into(),text:"A study note".into(),time:Some(3.0),bookmark_id:Some("course-note-1".into()) });
        let (mut refreshed,_) = parse_udemy_course(&fixture_curriculum(),"123","https://www.udemy.com/course/fixture/").unwrap();
        refreshed.sections[0].lessons.pop();
        merge_study(&original,&mut refreshed);
        assert!(refreshed.sections[0].lessons[0].learned);
        assert_eq!(refreshed.sections[0].lessons[0].notes[0].text,"A study note");
        assert_eq!(refreshed.sections[0].lessons[2].id,"lesson-22");
    }

    #[test]
    fn local_course_groups_sections_and_sorts_numbered_files() {
        let root = std::env::temp_dir().join(format!("nabnab-course-order-{}",queue::now()));
        fs::create_dir_all(root.join("Section 2")).unwrap();
        let paths = [root.join("Section 2/Lesson 10.mp4"),root.join("Section 2/Lesson 2.mp4")];
        for path in &paths { fs::write(path,b"fixture").unwrap(); }
        let items:Vec<_> = paths.iter().map(|path| library::index(path,None,None,None).unwrap()).collect();
        let root = fs::canonicalize(&root).unwrap();
        let course = local_course(&root,&items).unwrap();
        assert_eq!(course.sections.len(),1);
        assert_eq!(course.sections[0].title,"Section 2");
        assert_eq!(course.sections[0].lessons[0].title,"Lesson 2");
        assert_eq!(course.sections[0].lessons[1].title,"Lesson 10");
        assert!(!course.sections[0].lessons[0].learned);
        course.validate().unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn course_locations_reject_lookalikes_and_strip_nonessential_tokens() {
        assert!(course_location("https://udemy.com.attacker.net/course/a").is_err());
        let (url,id) = course_location("https://udemy.com/course/123/?token=private#lecture/4").unwrap();
        assert_eq!(url,"https://www.udemy.com/course/123/");
        assert_eq!(id.as_deref(),Some("123"));
        assert_eq!(udemy_course_id("<div data-course-id=\"456\">"),Some("456".into()));
    }
}
