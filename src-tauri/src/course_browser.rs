//! Browser-assisted study for providers without a supported learner download API.
use super::{
    digest, label, natural_key, save_course, Course, CourseAsset, CourseLesson, CourseSection,
};
use crate::{library, queue, AppState};
use std::{
    collections::{BTreeMap, HashSet},
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
                    *name,
                    format!("/journey/{name}/contents"),
                )
            }
            ("coursera.org" | "www.coursera.org", ["learn", name, ..])
                if locale.is_empty() && slug(name) =>
            {
                (
                    "coursera",
                    "www.coursera.org",
                    *name,
                    format!("/learn/{name}/home"),
                )
            }
            ("domestika.org" | "www.domestika.org", ["courses", name, ..]) if slug(name) => {
                let (id, _) = name.split_once('-')?;
                (
                    "domestika",
                    "www.domestika.org",
                    number(id)?,
                    format!("{locale}/courses/{name}"),
                )
            }
            ("skillshare.com" | "www.skillshare.com", ["classes", name, id, ..]) if slug(name) => (
                "skillshare",
                "www.skillshare.com",
                number(id)?,
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
            .ok_or_else(|| "Enter an HTTPS course link from the selected platform. For Rocketseat, use the journey URL.".into())
    }
}

pub(crate) fn is_course_page(input: &str) -> bool {
    CourseLink::parse(input).is_some()
}

pub(super) fn skillshare_course(input: &str) -> Result<(String, String), String> {
    let link = CourseLink::selected("skillshare", input)?;
    Ok((link.id, link.page.to_string()))
}

#[tauri::command]
pub fn open_course_site(provider: String, url: String) -> Result<(), String> {
    let link = CourseLink::selected(&provider, &url)?;
    crate::cloud_storage::browser(link.page.as_str())
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

fn material_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut paths = library::files_for_import(paths, |path| {
        library::extension_kind(path).is_some() || is_course_attachment(path)
    })?;
    paths.sort_unstable();
    paths.dedup();
    if paths.is_empty() || paths.len() > 200 {
        return Err("Choose between 1 and 200 supported course files.".into());
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

struct Plan {
    id: u32,
    link: CourseLink,
    title: String,
    files: Vec<ReviewedFile>,
    created: Instant,
}

#[derive(Default)]
pub(crate) struct Reviews {
    next: AtomicU32,
    pending: Mutex<Option<Plan>>,
}

fn prepare_review(
    id: u32,
    link: CourseLink,
    title: String,
    paths: &[PathBuf],
    folder: bool,
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
    for path in material_paths(paths)? {
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

fn material_sections(items: impl IntoIterator<Item = library::LibraryItem>) -> Vec<CourseSection> {
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
        let asset = CourseAsset {
            id: format!("asset-{}", item.id),
            title: label(&item.name, 240),
            kind: item.kind,
            status: "local".into(),
            restriction: None,
            library_id: Some(item.id),
            url_ref: None,
            job_ids: Vec::new(),
        };
        if let Some(lesson) = section
            .lessons
            .iter_mut()
            .find(|lesson| lesson.id == lesson_id)
        {
            lesson.assets.push(asset);
        } else {
            section.lessons.push(CourseLesson {
                id: lesson_id,
                title: label(title, 240),
                learned: false,
                assets: vec![asset],
                notes: Vec::new(),
            });
        }
    }
    let mut sections: Vec<_> = sections.into_values().collect();
    sections.sort_by_cached_key(|section| (natural_key(&section.title), section.id.clone()));
    for section in &mut sections {
        section
            .lessons
            .sort_by_cached_key(|lesson| (natural_key(&lesson.title), lesson.id.clone()));
        for lesson in &mut section.lessons {
            lesson
                .assets
                .sort_by_cached_key(|asset| (natural_key(&asset.title), asset.id.clone()));
        }
    }
    sections
}

fn import_selected(
    repo: &mut crate::store::Repository,
    link: CourseLink,
    title: String,
    selected: Vec<library::LibraryItem>,
) -> Result<Course, String> {
    let previous = repo.data.courses.iter().find(|course| course.id == link.id);
    let prior_ids: HashSet<_> = previous
        .into_iter()
        .flat_map(|course| &course.sections)
        .flat_map(|section| &section.lessons)
        .flat_map(|lesson| &lesson.assets)
        .filter_map(|asset| asset.library_id.as_deref())
        .collect();
    let mut included = HashSet::new();
    let items = repo
        .data
        .library
        .iter()
        .filter(|item| prior_ids.contains(item.id.as_str()))
        .cloned()
        .chain(selected.iter().cloned())
        .filter(|item| included.insert(item.id.clone()));
    let course = Course {
        id: link.id,
        title: previous.map(|course| course.title.clone()).unwrap_or(title),
        provider: link.provider.into(),
        url: Some(link.page.to_string()),
        slug: None,
        sections: material_sections(items),
        created_at: previous
            .map(|course| course.created_at)
            .unwrap_or_else(queue::now),
    };
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
) -> Result<Option<Review>, String> {
    let link = CourseLink::selected(&provider, &url)?;
    let title = label(&title, 240);
    if title.is_empty() {
        return Err("Enter the name to use for this course.".into());
    }
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file();
        if folder {
            dialog
                .set_title("Choose an authorized course folder")
                .blocking_pick_folder()
                .map(|folder| vec![folder])
        } else {
            dialog
                .set_title("Choose authorized course files")
                .blocking_pick_files()
        }
    })
    .await
    .map_err(|_| "The course file picker failed.")?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    if picked.is_empty() || picked.len() > 200 {
        return Err("Choose between 1 and 200 saved course files.".into());
    }
    let paths = picked
        .into_iter()
        .map(|file| {
            file.into_path()
                .map_err(|_| "Choose files saved on this computer.")
        })
        .collect::<Result<Vec<PathBuf>, _>>()?;
    let id = reviews.next.fetch_add(1, Ordering::Relaxed);
    let (review, plan) = tauri::async_runtime::spawn_blocking(move || {
        prepare_review(id, link, title, &paths, folder)
    })
    .await
    .map_err(|_| "Course file review failed.")??;
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
    import_selected(repo, plan.link, plan.title, selected)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        )
        .unwrap();
        let selected = selected_files(&plan, &[0, 1]).unwrap();
        let saved = import_selected(&mut repo, plan.link, plan.title, selected).unwrap();
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
        let (review, plan) = prepare_review(2, updated, "Dibujo".into(), &[folder], true).unwrap();
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
        let merged = import_selected(&mut repo, plan.link, plan.title, selected).unwrap();
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
        drop(repo);
        let root = std::fs::canonicalize(&root).unwrap();
        assert!(root.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
