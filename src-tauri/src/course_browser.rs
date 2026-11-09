//! Browser-assisted study for providers without a supported learner download API.
use super::{
    digest, label, natural_key, save_course, Course, CourseAsset, CourseLesson, CourseSection,
};
use crate::{library, queue, AppState};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use url::Url;

struct CourseLink {
    provider: &'static str,
    page: Url,
    id: String,
}

fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

fn number(value: &str) -> Option<&str> {
    let id = value.trim_start_matches('0');
    (!id.is_empty() && id.len() <= 20 && id.bytes().all(|byte| byte.is_ascii_digit())).then_some(id)
}

impl CourseLink {
    fn parse(input: &str) -> Option<Self> {
        if input.len() > 4096 || input.chars().any(char::is_control) {
            return None;
        }
        let mut page = Url::parse(input.trim()).ok()?;
        if page.scheme() != "https"
            || page.port().is_some()
            || !page.username().is_empty()
            || page.password().is_some()
        {
            return None;
        }
        let segments: Vec<_> = page.path().trim_matches('/').split('/').collect();
        let (locale, path) = match segments.as_slice() {
            [locale, rest @ ..]
                if locale.len() == 2 && locale.bytes().all(|b| b.is_ascii_lowercase()) =>
            {
                (format!("/{locale}"), rest)
            }
            path => (String::new(), path),
        };
        let (provider, host, key, course_path) = match (page.host_str()?, path) {
            ("app.rocketseat.com.br", ["journey", name, ..]) if locale.is_empty() && slug(name) => {
                (
                    "rocketseat",
                    "app.rocketseat.com.br",
                    (*name).to_owned(),
                    format!("/journey/{name}/contents"),
                )
            }
            ("app.rocketseat.com.br", ["classroom", name, ..]) if locale.is_empty() && slug(name) => {
                (
                    "rocketseat",
                    "app.rocketseat.com.br",
                    format!("classroom:{name}"),
                    format!("/classroom/{name}"),
                )
            }
            ("coursera.org" | "www.coursera.org", ["learn", name, ..])
                if locale.is_empty() && slug(name) =>
            {
                (
                    "coursera",
                    "www.coursera.org",
                    (*name).to_owned(),
                    format!("/learn/{name}/home"),
                )
            }
            ("domestika.org" | "www.domestika.org", ["courses", name, ..]) if slug(name) => {
                let (id, _) = name.split_once('-')?;
                (
                    "domestika",
                    "www.domestika.org",
                    number(id)?.to_owned(),
                    format!("{locale}/courses/{name}"),
                )
            }
            ("skillshare.com" | "www.skillshare.com", ["classes", name, id, ..]) if slug(name) => (
                "skillshare",
                "www.skillshare.com",
                number(id)?.to_owned(),
                format!("{locale}/classes/{name}/{id}"),
            ),
            _ => return None,
        };
        let id = format!("{provider}-{}", digest(&format!("{provider}:{key}")));
        page.set_host(Some(host)).ok()?;
        page.set_path(&course_path);
        page.set_query(None);
        page.set_fragment(None);
        Some(Self { provider, page, id })
    }

    fn selected(provider: &str, input: &str) -> Result<Self, String> {
        Self::parse(input)
            .filter(|link| link.provider == provider)
            .ok_or_else(|| "Enter an HTTPS course link from the selected platform. Rocketseat accepts journey and classroom links.".into())
    }
}

pub(crate) fn is_course_page(input: &str) -> bool {
    CourseLink::parse(input).is_some()
}

pub(super) fn authorized_course(provider: &str, input: &str) -> Result<(String, String), String> {
    let link = CourseLink::selected(provider, input)?;
    Ok((link.id, link.page.to_string()))
}

#[tauri::command]
pub fn open_course_site(provider: String, url: String) -> Result<(), String> {
    let link = CourseLink::selected(&provider, &url)?;
    crate::cloud_storage::browser(link.page.as_str())
}

const OUTLINE_SECTION: &str = "outline-section-";

fn outline_sections(text: &str) -> Result<Vec<CourseSection>, String> {
    if text.len() > 64 * 1024 {
        return Err("The outline is too long. Use up to 64 KiB of titles.".into());
    }
    let mut sections: Vec<CourseSection> = Vec::new();
    let mut ids = HashSet::new();
    let mut lessons = 0;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let heading = line.strip_prefix("# ");
        let title = heading.unwrap_or(line).trim();
        let invalid = || format!("Check outline line {}. Use # followed by a unit title, then one lesson title per line.", index + 1);
        if title.is_empty() || title.starts_with('#') || title.contains("://")
            || title.chars().count() > 240 || title.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        let key = title.to_lowercase();
        if heading.is_some() {
            let id = format!("{OUTLINE_SECTION}{}", digest(&key));
            if !ids.insert(id.clone()) { return Err(format!("Duplicate unit on line {}.", index + 1)); }
            sections.push(CourseSection { id, title: title.into(), lessons: Vec::new() });
        } else {
            let section = sections.last_mut().ok_or_else(invalid)?;
            let id = format!("outline-lesson-{}", digest(&format!("{}:{key}", section.id)));
            if !ids.insert(id.clone()) { return Err(format!("Duplicate lesson on line {}.", index + 1)); }
            section.lessons.push(CourseLesson { id, title: title.into(), learned: false, assets: Vec::new(), notes: Vec::new() });
            lessons += 1;
        }
        if sections.len() > 50 || lessons > 400 { return Err("Use up to 50 units and 400 lessons.".into()); }
    }
    if sections.is_empty() || sections.iter().any(|section| section.lessons.is_empty()) {
        return Err("Enter at least one lesson under each unit.".into());
    }
    Ok(sections)
}

#[tauri::command]
pub fn course_manual_outline(
    state: State<'_, Arc<AppState>>, provider: String, url: String, title: String, text: String, save: bool,
) -> Result<Course, String> {
    if !matches!(provider.as_str(), "rocketseat" | "domestika" | "skillshare") {
        return Err("Choose Rocketseat, Domestika or Skillshare for a manual outline.".into());
    }
    let link = CourseLink::selected(&provider, &url)?;
    let title = label(&title, 240);
    if title.is_empty() { return Err("Enter the course name.".into()); }
    let mut course = Course { id: link.id, title, provider, url: Some(link.page.to_string()),
        slug: None, sections: outline_sections(&text)?, created_at: queue::now() };
    if !save { return Ok(course); }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    if let Some(previous) = repo.data.courses.iter().find(|previous| previous.id == course.id) {
        course.created_at = previous.created_at;
        let prior_lessons: HashMap<_, _> = previous.sections.iter().flat_map(|section| &section.lessons)
            .map(|lesson| (lesson.id.as_str(), lesson)).collect();
        for lesson in course.sections.iter_mut().flat_map(|section| &mut section.lessons) {
            if let Some(previous) = prior_lessons.get(lesson.id.as_str()) { lesson.assets.clone_from(&previous.assets); }
        }
    }
    save_course(repo, course)
}

fn is_course_attachment(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "zip" | "7z" | "rar" | "psd" | "ai" | "svg" | "abr" | "procreate"
    )
}

fn index_material(path: &Path, link: &CourseLink) -> Result<library::LibraryItem, String> {
    if is_course_attachment(path) {
        let mut item = library::index_file(path)?;
        item.source = Some(link.page.origin().ascii_serialization());
        Ok(item)
    } else {
        library::index(path, None, Some(link.page.to_string()), None)
    }
}

fn material_paths(paths: &[PathBuf], attachments_only: bool) -> Result<Vec<PathBuf>, String> {
    let mut paths = library::files_for_import(paths, |path| {
        let kind = library::extension_kind(path);
        if attachments_only && matches!(kind, Some("audio" | "video")) {
            return false;
        }
        kind.is_some() || is_course_attachment(path)
    })?;
    paths.sort_unstable();
    paths.dedup();
    if paths.is_empty() || paths.len() > 200 {
        return Err(if attachments_only {
            "Choose between 1 and 200 documents, captions, images or archives."
        } else {
            "Choose between 1 and 200 supported course files."
        }
        .into());
    }
    Ok(paths)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    id: u32,
    title: String,
    provider: &'static str,
    groups: Vec<ReviewGroup>,
}

#[derive(serde::Serialize)]
struct ReviewGroup {
    title: String,
    files: Vec<ReviewFile>,
}

#[derive(serde::Serialize)]
struct ReviewFile {
    id: usize,
    name: String,
    kind: &'static str,
    size: u64,
}

struct ReviewedFile {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

enum AttachmentTarget {
    Lesson(String),
    Unit(String),
}

const UNIT_RESOURCES: &str = "unit-resources-";

fn unit_resources_id(section: &str) -> String {
    format!("{UNIT_RESOURCES}{section}")
}

struct Plan {
    id: u32,
    link: CourseLink,
    title: String,
    target: Option<AttachmentTarget>,
    files: Vec<ReviewedFile>,
    created: Instant,
}

#[derive(Default)]
pub(crate) struct Reviews {
    next: AtomicU32,
    pending: Mutex<Option<Plan>>,
    dropped: Mutex<Option<(Vec<PathBuf>, Instant)>>,
}

impl Reviews {
    pub(crate) fn remember_drop(&self, paths: &[PathBuf]) {
        *self.dropped.lock().unwrap() = (1..=200)
            .contains(&paths.len())
            .then(|| (paths.to_vec(), Instant::now()));
    }

    fn take_drop(&self, paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
        if !(1..=200).contains(&paths.len()) {
            return Err("Choose between 1 and 200 saved course files.".into());
        }
        let mut dropped = self.dropped.lock().unwrap();
        dropped
            .take_if(|(saved, received)| {
                saved.as_slice() == paths && received.elapsed() < Duration::from_secs(10)
            })
            .map(|(saved, _)| saved)
            .ok_or_else(|| {
                "Drop the course files from Explorer again or use the file picker.".into()
            })
    }
}

fn prepare_review(
    id: u32,
    link: CourseLink,
    title: String,
    paths: &[PathBuf],
    folder: bool,
    target: Option<AttachmentTarget>,
) -> Result<(Review, Plan), String> {
    let root = if folder {
        paths
            .first()
            .map(|path| library::authorized_path(path))
            .transpose()?
    } else {
        None
    };
    let mut groups = BTreeMap::<PathBuf, ReviewGroup>::new();
    let mut files = Vec::new();
    for path in material_paths(paths, target.is_some())? {
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() == 0 {
            continue;
        }
        let parent = path
            .parent()
            .ok_or("The selected file has no parent folder.")?;
        let group = groups.entry(parent.to_owned()).or_insert_with(|| {
            let relative = root
                .as_ref()
                .and_then(|root| parent.strip_prefix(root).ok());
            let title = relative
                .filter(|relative| !relative.as_os_str().is_empty())
                .map(|relative| {
                    relative
                        .iter()
                        .map(|part| part.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_else(|| {
                    parent
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
            ReviewGroup {
                title,
                files: Vec::new(),
            }
        });
        let size = metadata.len();
        group.files.push(ReviewFile {
            id: files.len(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            kind: library::extension_kind(&path).unwrap_or("file"),
            size,
        });
        files.push(ReviewedFile {
            path,
            size,
            modified: metadata.modified().map_err(|error| error.to_string())?,
        });
    }
    if files.is_empty() {
        return Err("No supported course files with content were found.".into());
    }
    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by_cached_key(|group| natural_key(&group.title));
    for group in &mut groups {
        group
            .files
            .sort_by_cached_key(|file| natural_key(&file.name));
    }
    let review = Review {
        id,
        title: title.clone(),
        provider: link.provider,
        groups,
    };
    Ok((
        review,
        Plan {
            id,
            link,
            title,
            target,
            files,
            created: Instant::now(),
        },
    ))
}

fn selected_files(plan: &Plan, ids: &[usize]) -> Result<Vec<library::LibraryItem>, String> {
    if ids.is_empty() || ids.len() > plan.files.len() {
        return Err("Select at least one reviewed course file.".into());
    }
    let mut seen = HashSet::new();
    ids.iter()
        .map(|id| {
            let reviewed = plan
                .files
                .get(*id)
                .filter(|_| seen.insert(*id))
                .ok_or("Choose files from the current course review.")?;
            let item = index_material(&reviewed.path, &plan.link)?;
            if item.path != reviewed.path
                || item.size != reviewed.size
                || fs::metadata(&item.path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    != Some(reviewed.modified)
            {
                return Err(
                    "A course file changed after review. Choose the folder or files again.".into(),
                );
            }
            Ok(item)
        })
        .collect()
}

fn path_identity(path: &Path) -> String {
    let name = path.to_string_lossy();
    if cfg!(windows) {
        name.to_lowercase()
    } else {
        name.into_owned()
    }
}

fn local_asset(item: &library::LibraryItem) -> CourseAsset {
    CourseAsset {
        id: format!("asset-{}", item.id),
        title: label(&item.name, 240),
        kind: item.kind.clone(),
        status: "local".into(),
        restriction: None,
        library_id: Some(item.id.clone()),
        url_ref: None,
        job_ids: Vec::new(),
    }
}

fn material_sections(items: Vec<&library::LibraryItem>) -> Vec<CourseSection> {
    let mut sections = BTreeMap::<String, CourseSection>::new();
    for item in items {
        let Some(parent) = item.path.parent() else {
            continue;
        };
        let title = item
            .path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&item.name);
        let section_id = format!("section-{}", digest(&path_identity(parent)));
        let lesson_id = format!("lesson-{}", digest(&path_identity(&parent.join(title))));
        let section = sections
            .entry(section_id.clone())
            .or_insert_with(|| CourseSection {
                id: section_id,
                title: label(
                    &parent.file_name().unwrap_or_default().to_string_lossy(),
                    240,
                ),
                lessons: Vec::new(),
            });
        if let Some(lesson) = section
            .lessons
            .iter_mut()
            .find(|lesson| lesson.id == lesson_id)
        {
            lesson.assets.push(local_asset(item));
        } else {
            section.lessons.push(CourseLesson {
                id: lesson_id,
                title: label(title, 240),
                learned: false,
                assets: vec![local_asset(item)],
                notes: Vec::new(),
            });
        }
    }
    sections.into_values().collect()
}

fn merge_material_sections(sections: &mut Vec<CourseSection>, added: Vec<CourseSection>) {
    for incoming in added {
        if let Some(section) = sections
            .iter_mut()
            .find(|section| section.id == incoming.id)
        {
            for new_lesson in incoming.lessons {
                if let Some(lesson) = section
                    .lessons
                    .iter_mut()
                    .find(|lesson| lesson.id == new_lesson.id)
                {
                    lesson.assets.extend(new_lesson.assets);
                } else {
                    section.lessons.push(new_lesson);
                }
            }
        } else {
            sections.push(incoming);
        }
    }
    // Keep provider-specific sections, such as the Skillshare downloads, in their original order.
    sections.sort_by_cached_key(|section| {
        let local = section.id.starts_with("section-");
        (
            if section.id.starts_with(OUTLINE_SECTION) { 0 } else if local { 1 } else { 2 },
            local.then(|| (natural_key(&section.title), section.id.clone())),
        )
    });
    for section in sections
        .iter_mut()
        .filter(|section| section.id.starts_with("section-"))
    {
        section
            .lessons
            .sort_by_cached_key(|lesson| (
                lesson.id.starts_with(UNIT_RESOURCES),
                natural_key(&lesson.title),
                lesson.id.clone(),
            ));
        for lesson in &mut section.lessons {
            lesson
                .assets
                .sort_by_cached_key(|asset| (natural_key(&asset.title), asset.id.clone()));
        }
    }
}

fn import_selected(
    repo: &mut crate::store::Repository,
    link: CourseLink,
    title: String,
    selected: Vec<library::LibraryItem>,
    target: Option<&AttachmentTarget>,
) -> Result<Course, String> {
    let mut course = repo
        .data
        .courses
        .iter()
        .find(|course| course.id == link.id)
        .cloned()
        .unwrap_or_else(|| Course {
            id: link.id,
            title,
            provider: link.provider.into(),
            url: None,
            slug: None,
            sections: Vec::new(),
            created_at: queue::now(),
        });
    course.url = Some(link.page.to_string());
    let attached: HashMap<_, _> = course
        .sections
        .iter()
        .flat_map(|section| &section.lessons)
        .flat_map(|lesson| {
            lesson
                .assets
                .iter()
                .filter_map(move |asset| Some((asset.library_id.as_deref()?, lesson.id.as_str())))
        })
        .collect();
    let target_id = target.map(|target| match target {
        AttachmentTarget::Lesson(id) => id.clone(),
        AttachmentTarget::Unit(section) => unit_resources_id(section),
    });
    if let Some(target_id) = &target_id {
        if selected.iter().any(|item| {
            attached
                .get(item.id.as_str())
                .is_some_and(|other| *other != target_id.as_str())
        }) {
            return Err("A selected file already belongs to another lesson or unit in this course.".into());
        }
    }
    let mut included = HashSet::new();
    let additions: Vec<_> = selected
        .iter()
        .filter(|item| {
            !attached.contains_key(item.id.as_str()) && included.insert(item.id.as_str())
        })
        .collect();
    match target {
        Some(AttachmentTarget::Lesson(id)) => {
            let lesson = course.sections.iter_mut().filter(|section| {
                section.id.starts_with("section-") || section.id.starts_with(OUTLINE_SECTION)
                    || link.provider == "coursera" && section.id.starts_with("coursera-module-")
            })
                .flat_map(|section| &mut section.lessons)
                .find(|lesson| lesson.id == *id && !lesson.id.starts_with(UNIT_RESOURCES))
                .ok_or("This lesson is no longer available. Open the course and choose the attachments again.")?;
            lesson.assets.extend(additions.into_iter().map(local_asset));
        }
        Some(AttachmentTarget::Unit(section_id)) => {
            let section = course.sections.iter_mut().find(|section| {
                link.provider == "domestika" && section.id == *section_id
                    && (section.id.starts_with("section-") || section.id.starts_with(OUTLINE_SECTION))
            }).ok_or("This Domestika unit is no longer available. Open the course and choose the resources again.")?;
            let id = unit_resources_id(section_id);
            if let Some(entry) = section.lessons.iter_mut().find(|lesson| lesson.id == id) {
                entry.assets.extend(additions.into_iter().map(local_asset));
            } else {
                section.lessons.push(CourseLesson {
                    id,
                    title: "Unit resources".into(),
                    learned: false,
                    assets: additions.into_iter().map(local_asset).collect(),
                    notes: Vec::new(),
                });
            }
        }
        None => merge_material_sections(&mut course.sections, material_sections(additions)),
    }
    let mut known: HashSet<_> = repo
        .data
        .library
        .iter()
        .map(|item| item.id.clone())
        .collect();
    let previous_len = repo.data.library.len();
    repo.data.library.extend(
        selected
            .into_iter()
            .filter(|item| known.insert(item.id.clone())),
    );
    let saved = save_course(repo, course);
    if saved.is_err() {
        repo.data.library.truncate(previous_len);
    }
    saved
}

#[tauri::command]
pub async fn preview_course_materials(
    app: AppHandle,
    reviews: State<'_, Reviews>,
    provider: String,
    url: String,
    title: String,
    folder: bool,
    lesson_id: Option<String>,
    unit_id: Option<String>,
    dropped_paths: Option<Vec<PathBuf>>,
) -> Result<Option<Review>, String> {
    let link = CourseLink::selected(&provider, &url)?;
    let target = match (lesson_id, unit_id) {
        (Some(id), None) => Some(AttachmentTarget::Lesson(id)),
        (None, Some(id)) if link.provider == "domestika" => Some(AttachmentTarget::Unit(id)),
        (None, None) => None,
        _ => return Err("Choose one valid destination for the course resources.".into()),
    };
    let title = label(&title, 240);
    if title.is_empty() {
        return Err("Enter the name to use for this course.".into());
    }
    let from_drop = dropped_paths.is_some();
    let dropped = dropped_paths
        .map(|paths| reviews.take_drop(&paths))
        .transpose()?;
    let id = reviews.next.fetch_add(1, Ordering::Relaxed);
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let paths = match dropped {
            Some(paths) => paths,
            None => {
                let dialog = app.dialog().file();
                let picked = if folder {
                    dialog
                        .set_title("Choose an authorized course folder")
                        .blocking_pick_folder()
                        .map(|folder| vec![folder])
                } else {
                    dialog
                        .set_title(match &target {
                            Some(AttachmentTarget::Lesson(_)) => "Choose attachments for this lesson",
                            Some(AttachmentTarget::Unit(_)) => "Choose resources for this unit",
                            None => "Choose authorized course files",
                        })
                        .blocking_pick_files()
                };
                let Some(picked) = picked else {
                    return Ok(None);
                };
                if picked.is_empty() || picked.len() > 200 {
                    return Err("Choose between 1 and 200 saved course files.".into());
                }
                picked
                    .into_iter()
                    .map(|file| {
                        file.into_path()
                            .map_err(|_| "Choose files saved on this computer.")
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        let folder = if from_drop {
            paths.len() == 1 && paths[0].is_dir()
        } else {
            folder
        };
        prepare_review(id, link, title, &paths, folder, target).map(Some)
    })
    .await
    .map_err(|_| "Course file review failed.")??;
    let Some((review, plan)) = prepared else {
        return Ok(None);
    };
    *reviews.pending.lock().unwrap() = Some(plan);
    Ok(Some(review))
}

#[tauri::command]
pub fn discard_course_materials(reviews: State<'_, Reviews>, review_id: u32) {
    let mut pending = reviews.pending.lock().unwrap();
    if pending.as_ref().is_some_and(|plan| plan.id == review_id) {
        pending.take();
    }
}

#[tauri::command]
pub async fn import_course_materials(
    state: State<'_, Arc<AppState>>,
    reviews: State<'_, Reviews>,
    review_id: u32,
    ids: Vec<usize>,
) -> Result<Course, String> {
    let plan = {
        let mut pending = reviews.pending.lock().unwrap();
        if !pending.as_ref().is_some_and(|plan| {
            plan.id == review_id && plan.created.elapsed() < Duration::from_secs(600)
        }) {
            return Err("The review expired. Choose the course files again.".into());
        }
        pending.take().ok_or("The course review is unavailable.")?
    };
    let (plan, selected) = tauri::async_runtime::spawn_blocking(move || {
        let selected = selected_files(&plan, &ids)?;
        Ok::<_, String>((plan, selected))
    })
    .await
    .map_err(|_| "Course file indexing failed.")??;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    import_selected(
        repo,
        plan.link,
        plan.title,
        selected,
        plan.target.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn course_drop_only_consumes_the_matching_recent_native_selection_once() {
        let reviews = Reviews::default();
        let files = [PathBuf::from("lesson.pdf"), PathBuf::from("notes.txt")];
        assert!(reviews.take_drop(&files).is_err());
        reviews.remember_drop(&files);
        assert!(reviews.take_drop(&files[..1]).is_err());
        assert_eq!(reviews.take_drop(&files).unwrap(), files);
        assert!(reviews.take_drop(&files).is_err());
        *reviews.dropped.lock().unwrap() =
            Some((files.to_vec(), Instant::now() - Duration::from_secs(10)));
        assert!(reviews.take_drop(&files).is_err());
    }

    #[test]
    fn course_links_remove_session_data_and_keep_stable_ids() {
        let a = CourseLink::selected(
            "coursera",
            "https://coursera.org/learn/apis/lecture/id?token=private#start",
        )
        .unwrap();
        let b =
            CourseLink::selected("coursera", "https://www.coursera.org/learn/apis/home").unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.page.as_str(), "https://www.coursera.org/learn/apis/home");
        let a = CourseLink::selected(
            "skillshare",
            "https://www.skillshare.com/en/classes/New-title/123/projects",
        )
        .unwrap();
        let b = CourseLink::selected(
            "skillshare",
            "https://www.skillshare.com/es/classes/Old-title/123",
        )
        .unwrap();
        assert_eq!(a.id, b.id);
        let a = CourseLink::selected(
            "domestika",
            "https://www.domestika.org/es/courses/24-dibujo/units/25",
        )
        .unwrap();
        let b = CourseLink::selected("domestika", "https://domestika.org/en/courses/24-drawing")
            .unwrap();
        assert_eq!(a.id, b.id);
        assert!(CourseLink::selected(
            "rocketseat",
            "https://app.rocketseat.com.br/journey/discover/contents"
        )
        .is_ok());
        for invalid in [
            "https://coursera.org.evil.test/learn/apis",
            "https://user:pass@www.coursera.org/learn/apis",
            "https://www.coursera.org:8080/learn/apis",
            "http://coursera.org/learn/apis",
            "https://www.coursera.org/learn/",
        ] {
            assert!(!is_course_page(invalid));
        }
        assert!(CourseLink::selected("rocketseat", "https://www.coursera.org/learn/apis").is_err());
    }

    #[test]
    fn rocketseat_classroom_import_keeps_a_separate_stable_course() {
        const CLASSROOM: &str = "https://app.rocketseat.com.br/classroom/git-e-github";
        let link = CourseLink::selected(
            "rocketseat",
            &format!("{CLASSROOM}/?token=private#lesson"),
        )
        .unwrap();
        let journey = CourseLink::selected(
            "rocketseat",
            "https://app.rocketseat.com.br/journey/git-e-github/contents",
        )
        .unwrap();
        assert_eq!(link.page.as_str(), CLASSROOM);
        assert_eq!(
            journey.id,
            format!("rocketseat-{}", digest("rocketseat:git-e-github"))
        );
        assert_ne!(link.id, journey.id);
        assert!(!is_course_page("https://app.rocketseat.com.br.evil.test/classroom/git-e-github"));

        let root = std::env::temp_dir()
            .join(format!("nabnab-rocketseat-classroom-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("01 Exercises.txt");
        let contents = b"Authorized classroom exercise";
        fs::write(&path, contents).unwrap();
        let mut repo = crate::store::Repository::open(&root, &root).unwrap();
        let course_id = link.id.clone();
        for link in [link, CourseLink::selected("rocketseat", CLASSROOM).unwrap()] {
            let (_, plan) = prepare_review(
                1, link, "Git e Github".into(), std::slice::from_ref(&path), false, None,
            )
            .unwrap();
            let selected = selected_files(&plan, &[0]).unwrap();
            let saved = import_selected(&mut repo, plan.link, plan.title, selected, None).unwrap();
            assert_eq!(saved.url.as_deref(), Some(CLASSROOM));
            assert_eq!(saved.sections[0].lessons[0].assets[0].title, "01 Exercises.txt");
            assert_eq!(saved.id, course_id);
        }
        assert_eq!(repo.data.courses.len(), 1);
        assert_eq!(repo.data.library.len(), 1);
        assert!(repo.data.jobs.is_empty());
        assert_eq!(fs::read(path).unwrap(), contents);
        drop(repo);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn attaches_saved_file_to_a_coursera_outline_lesson() {
        let root = std::env::temp_dir()
            .join(format!("nabnab-coursera-attachment-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("Saved handout.txt");
        fs::write(&path, b"Authorized course handout").unwrap();
        let link = CourseLink::selected("coursera", "https://www.coursera.org/learn/fixture").unwrap();
        let mut repo = crate::store::Repository::open(&root, &root).unwrap();
        save_course(&mut repo, Course {
            id: link.id.clone(), title: "Fixture".into(), provider: "coursera".into(),
            url: Some(link.page.to_string()), slug: Some("remote-id".into()), created_at: 1,
            sections: vec![CourseSection {
                id: "coursera-module-fixture".into(), title: "First module".into(),
                lessons: vec![CourseLesson {
                    id: "coursera-lesson-fixture".into(), title: "First lesson".into(),
                    learned: true, notes: Vec::new(), assets: Vec::new(),
                }],
            }],
        }).unwrap();
        let target = AttachmentTarget::Lesson("coursera-lesson-fixture".into());
        let (_, plan) = prepare_review(1, link, "Fixture".into(), &[path], false, Some(target)).unwrap();
        let selected = selected_files(&plan, &[0]).unwrap();
        let saved = import_selected(&mut repo, plan.link, plan.title, selected, plan.target.as_ref()).unwrap();
        let lesson = &saved.sections[0].lessons[0];
        assert!(lesson.learned);
        assert_eq!(lesson.assets[0].title, "Saved handout.txt");
        assert_eq!(lesson.assets[0].library_id.as_deref(), Some(repo.data.library[0].id.as_str()));
        assert!(repo.data.jobs.is_empty());
        drop(repo);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adding_saved_materials_keeps_order_progress_and_identity() {
        let root =
            std::env::temp_dir().join(format!("nabnab-course-browser-{}", std::process::id()));
        let folder = root.join("01 Exercises");
        std::fs::create_dir_all(&folder).unwrap();
        let first = folder.join("2 Practice.txt");
        let attachment = folder.join("2 Practice.zip");
        let later = folder.join("10 Review.txt");
        for path in [&first, &attachment, &later] {
            std::fs::write(path, b"Authorized course fixture").unwrap();
        }
        let mut repo = crate::store::Repository::open(&root, &folder).unwrap();
        let original =
            CourseLink::selected("domestika", "https://domestika.org/en/courses/24-drawing")
                .unwrap();
        let (_, plan) = prepare_review(
            1,
            original,
            "Drawing".into(),
            &[later.clone(), first.clone()],
            false,
            None,
        )
        .unwrap();
        let selected = selected_files(&plan, &[0, 1]).unwrap();
        let saved = import_selected(&mut repo, plan.link, plan.title, selected, None).unwrap();
        let lesson = &mut repo.data.courses[0].sections[0].lessons[0];
        assert_eq!(lesson.title, "2 Practice");
        let lesson_id = lesson.id.clone();
        lesson.learned = true;
        lesson.notes.push(super::super::CourseNote {
            id: "note-1".into(),
            text: "Keep this".into(),
            time: None,
            bookmark_id: None,
        });
        let updated = CourseLink::selected(
            "domestika",
            "https://www.domestika.org/es/courses/24-dibujo",
        )
        .unwrap();
        let nested = folder.join("11 Extras");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("1 Notes.txt"), b"Course notes").unwrap();
        std::fs::write(nested.join("ignore.exe"), b"Unsupported").unwrap();
        std::fs::write(nested.join("3 Optional.txt"), b"Optional reading").unwrap();
        let (review, plan) =
            prepare_review(2, updated, "Dibujo".into(), &[folder], true, None).unwrap();
        assert_eq!(plan.files.len(), 5);
        assert_eq!(review.groups[1].title, "11 Extras");
        assert!(repo
            .data
            .library
            .iter()
            .all(|item| item.name != "1 Notes.txt"));
        let ids: Vec<_> = review
            .groups
            .iter()
            .flat_map(|group| &group.files)
            .filter(|file| file.name != "3 Optional.txt")
            .map(|file| file.id)
            .collect();
        assert!(selected_files(&plan, &[99]).is_err());
        assert!(selected_files(&plan, &[ids[0], ids[0]]).is_err());
        let optional = review.groups[1]
            .files
            .iter()
            .find(|file| file.name == "3 Optional.txt")
            .unwrap();
        std::fs::write(
            nested.join("3 Optional.txt"),
            b"Different material after review",
        )
        .unwrap();
        assert!(selected_files(&plan, &[optional.id])
            .unwrap_err()
            .contains("changed after review"));
        let selected = selected_files(&plan, &ids).unwrap();
        let merged = import_selected(&mut repo, plan.link, plan.title, selected, None).unwrap();
        assert_eq!(merged.id, saved.id);
        assert_eq!(merged.title, "Drawing");
        assert_eq!(merged.sections[0].title, "01 Exercises");
        assert_eq!(merged.sections[0].lessons.len(), 2);
        let lesson = &merged.sections[0].lessons[0];
        assert_eq!(lesson.id, lesson_id);
        assert!(lesson.learned);
        assert_eq!(lesson.notes[0].text, "Keep this");
        assert_eq!(lesson.assets.len(), 2);
        assert_eq!(merged.sections[1].title, "11 Extras");
        assert_eq!(merged.sections[1].lessons[0].title, "1 Notes");
        assert_eq!(repo.data.library.len(), 4);
        assert!(repo
            .data
            .library
            .iter()
            .all(|item| item.name != "3 Optional.txt"));
        assert_eq!(repo.data.courses.len(), 1);

        let handout = nested.join("Materials and references.txt");
        let video = nested.join("Lesson.mp4");
        std::fs::write(&handout, b"Additional material").unwrap();
        std::fs::write(&video, b"A lesson is not an attachment").unwrap();
        let course_url = merged.url.as_deref().unwrap();
        let link = || CourseLink::selected("domestika", course_url).unwrap();
        assert!(prepare_review(
            3,
            link(),
            "Drawing".into(),
            &[video],
            false,
            Some(AttachmentTarget::Lesson(lesson_id.clone()))
        )
        .is_err());
        let (review, plan) = prepare_review(
            3,
            link(),
            "Drawing".into(),
            &[handout],
            false,
            Some(AttachmentTarget::Lesson(lesson_id.clone())),
        )
        .unwrap();
        assert_eq!(review.groups[0].files.len(), 1);
        let attachments = selected_files(&plan, &[0]).unwrap();
        assert!(import_selected(
            &mut repo,
            link(),
            "Drawing".into(),
            attachments.clone(),
            Some(&AttachmentTarget::Lesson("missing-lesson".into()))
        )
        .is_err());
        assert_eq!(repo.data.library.len(), 4);
        let with_attachment = import_selected(
            &mut repo,
            plan.link,
            plan.title,
            attachments.clone(),
            plan.target.as_ref(),
        )
        .unwrap();
        let lesson = &with_attachment.sections[0].lessons[0];
        assert_eq!(lesson.id, lesson_id);
        assert!(lesson.learned);
        assert_eq!(lesson.notes[0].text, "Keep this");
        assert_eq!(lesson.assets.len(), 3);
        assert_eq!(lesson.assets[2].title, "Materials and references.txt");
        let other = &with_attachment.sections[1].lessons[0].id;
        assert!(import_selected(
            &mut repo,
            link(),
            "Drawing".into(),
            attachments.clone(),
            Some(&AttachmentTarget::Lesson(other.clone()))
        )
        .is_err());
        let repeated =
            import_selected(&mut repo, link(), "Drawing".into(), attachments, None).unwrap();
        assert_eq!(repeated.sections[1].lessons.len(), 1);
        assert_eq!(repeated.sections[0].lessons[0].assets.len(), 3);
        assert_eq!(repo.data.library.len(), 5);

        let unit_id = repeated.sections[0].id.clone();
        let unit_resource = nested.join("Unit templates.zip");
        fs::write(&unit_resource, b"Authorized material from another folder").unwrap();
        assert!(prepare_review(
            4, link(), "Drawing".into(), &[nested.join("Lesson.mp4")],
            false, Some(AttachmentTarget::Unit(unit_id.clone())),
        ).is_err());
        let (_, plan) = prepare_review(
            4, link(), "Drawing".into(), &[unit_resource],
            false, Some(AttachmentTarget::Unit(unit_id.clone())),
        ).unwrap();
        assert_eq!(plan.files.len(), 1);
        let files = selected_files(&plan, &[0]).unwrap();
        assert!(import_selected(&mut repo, link(), "Drawing".into(), files.clone(),
            Some(&AttachmentTarget::Unit("section-missing".into()))).is_err());
        assert_eq!(repo.data.library.len(), 5);
        for _ in 0..2 {
            let course = import_selected(&mut repo, link(), "Drawing".into(), files.clone(), plan.target.as_ref()).unwrap();
            let unit = &course.sections[0];
            assert_eq!(unit.lessons.len(), 3);
            assert_eq!(unit.lessons[2].id, unit_resources_id(&unit_id));
            assert_eq!(unit.lessons[2].assets.len(), 1);
            assert_eq!(unit.lessons[2].assets[0].title, "Unit templates.zip");
            assert!(unit.lessons[0].learned);
            assert_eq!(unit.lessons[0].notes[0].text, "Keep this");
        }
        assert!(import_selected(&mut repo, link(), "Drawing".into(), files,
            Some(&AttachmentTarget::Unit(repeated.sections[1].id.clone()))).is_err());
        assert_eq!(repo.data.library.len(), 6);
        assert!(repo.data.jobs.is_empty());
        drop(repo);
        let reopened = crate::store::Repository::open(&root, &nested).unwrap();
        let lesson = &reopened.data.courses[0].sections[0].lessons[0];
        assert_eq!(lesson.id, lesson_id);
        assert_eq!(lesson.assets[2].title, "Materials and references.txt");
        assert_eq!(lesson.notes[0].text, "Keep this");
        assert_eq!(reopened.data.courses[0].sections[0].lessons[2].assets[0].title, "Unit templates.zip");
        drop(reopened);
        let root = std::fs::canonicalize(&root).unwrap();
        assert!(root.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
