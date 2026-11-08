use super::{
    browser, digest, label, save_course, Course, CourseAsset, CourseLesson, CourseSection,
};
use crate::{direct, file_sources::PublicSelection, queue, sessions, store::Repository, AppState};
use serde::Serialize;
use std::{collections::HashSet, path::Path, sync::Arc};
use tauri::State;
use url::Url;

pub(crate) const SKILLSHARE_LIMIT: u64 = 104 * 1024 * 1024;
const RESOURCE_SECTION: &str = "skillshare-resources";

pub(crate) struct Resource {
    pub url: Url,
    pub filename: String,
    id: String,
    revision: String,
}

impl Resource {
    fn identity(&self, course: &str) -> String {
        format!("skillshare:{course}:{}:{}", self.id, self.revision)
    }
}

pub(crate) fn parse(input: &str) -> Result<Resource, String> {
    const INVALID: &str = "Use an HTTPS attachment link from Skillshare's Download Resources list.";
    if input.len() > 2048 || input.chars().any(char::is_control) {
        return Err(INVALID.into());
    }
    let url = Url::parse(input.trim()).map_err(|_| INVALID)?;
    if url.scheme() != "https"
        || url.host_str() != Some("static.skillshare.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(INVALID.into());
    }
    let parts: Vec<_> = url.path_segments().ok_or(INVALID)?.collect();
    let ["uploads", "attachment", id, revision, filename] = parts.as_slice() else {
        return Err(INVALID.into());
    };
    if id.is_empty()
        || id.len() > 20
        || !id.bytes().all(|byte| byte.is_ascii_digit())
        || !(8..=64).contains(&revision.len())
        || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(INVALID.into());
    }
    let id = id.to_string();
    let revision = revision.to_string();
    let filename = crate::public_sources::percent_decode(filename)?;
    direct::validate_filename(&filename)?;
    let extension = Path::new(&filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "pdf"
            | "zip"
            | "7z"
            | "rar"
            | "docx"
            | "xlsx"
            | "pptx"
            | "txt"
            | "csv"
            | "psd"
            | "ai"
            | "abr"
            | "procreate"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
    ) {
        return Err(
            "This Skillshare link does not name a supported document, image or archive attachment."
                .into(),
        );
    }
    Ok(Resource {
        url,
        filename,
        id,
        revision,
    })
}

pub(crate) fn validate_selection(selection: &PublicSelection) -> Result<Resource, String> {
    let resource = parse(&selection.source_url)?;
    let course = selection
        .asset_id
        .split(':')
        .nth(1)
        .ok_or("Invalid Skillshare attachment identity.")?;
    let valid_course = course
        .strip_prefix("skillshare-")
        .is_some_and(|id| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if !valid_course
        || resource.identity(course) != selection.asset_id
        || resource.filename != selection.filename
        || Some(&resource.revision) != selection.version.as_ref()
        || selection.audiobook.is_some()
        || selection.sha256.is_some()
        || selection.checksum.is_some()
        || selection.size.is_some()
    {
        return Err("The Skillshare attachment does not match the reviewed resource.".into());
    }
    Ok(resource)
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResourceCheck {
    Supported { filename: String },
    Duplicate { filename: String },
    Unsupported,
}

#[tauri::command]
pub fn review_skillshare_resources(links: Vec<String>) -> Result<Vec<ResourceCheck>, String> {
    if links.is_empty() || links.len() > 30 {
        return Err("Paste 1–30 Skillshare attachment links.".into());
    }
    let mut seen = HashSet::new();
    Ok(links
        .iter()
        .map(|link| match parse(link) {
            Ok(Resource { id, revision, filename, .. }) => {
                if seen.insert((id, revision)) {
                    ResourceCheck::Supported { filename }
                } else {
                    ResourceCheck::Duplicate { filename }
                }
            }
            Err(_) => ResourceCheck::Unsupported,
        })
        .collect())
}

#[tauri::command]
pub fn add_skillshare_resources(
    state: State<Arc<AppState>>,
    url: String,
    title: String,
    links: Vec<String>,
    lesson_id: Option<String>,
) -> Result<Course, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    register(repo, &url, &title, &links, lesson_id.as_deref())
}

fn register(
    repo: &mut Repository,
    url: &str,
    title: &str,
    links: &[String],
    lesson_id: Option<&str>,
) -> Result<Course, String> {
    let (id, url) = browser::authorized_course("skillshare", url)?;
    let title = label(title, 240);
    if title.is_empty() || links.is_empty() || links.len() > 30 {
        return Err("Enter a course name and 1–30 resource links.".into());
    }
    let mut seen = HashSet::new();
    let mut resources = Vec::new();
    for (index, link) in links.iter().enumerate() {
        let resource = parse(link).map_err(|error| format!("Resource {}: {error}", index + 1))?;
        if seen.insert(resource.identity(&id)) {
            resources.push(resource);
        }
    }
    let mut course = repo
        .data
        .courses
        .iter()
        .find(|course| course.id == id)
        .cloned()
        .unwrap_or(Course {
            id,
            title,
            provider: "skillshare".into(),
            url: Some(url),
            slug: None,
            sections: Vec::new(),
            created_at: queue::now(),
        });
    let target = lesson_id
        .map(|id| {
            course
                .sections
                .iter()
                .enumerate()
                .find_map(|(section_index, section)| {
                    if section.id == RESOURCE_SECTION {
                        return None;
                    }
                    let lesson_index = section.lessons.iter().position(|lesson| lesson.id == id)?;
                    Some((section_index, lesson_index))
                })
                .ok_or("Choose an existing lesson in this Skillshare course.")
        })
        .transpose()?;
    for (section_index, section) in course.sections.iter().enumerate() {
        for (lesson_index, lesson) in section.lessons.iter().enumerate() {
            if target == Some((section_index, lesson_index))
                || target.is_none() && section.id == RESOURCE_SECTION
            {
                continue;
            }
            if let Some(asset) = lesson.assets.iter().find(|asset| seen.contains(&asset.id)) {
                return Err(format!(
                    "{} is already listed under {}.",
                    asset.title, lesson.title
                ));
            }
        }
    }
    let section_index = match target {
        Some((section_index, _)) => section_index,
        None => match course
            .sections
            .iter()
            .position(|section| section.id == RESOURCE_SECTION)
        {
            Some(index) => index,
            None => {
                course.sections.push(CourseSection {
                    id: RESOURCE_SECTION.into(),
                    title: "Recursos de Skillshare".into(),
                    lessons: Vec::new(),
                });
                course.sections.len() - 1
            }
        },
    };
    let section = &mut course.sections[section_index];
    for resource in resources {
        let asset_id = resource.identity(&course.id);
        let url_ref = sessions::seal_url(
            repo.root(),
            &format!("course-{}", digest(resource.url.as_str())),
            resource.url.as_str(),
        )?;
        let lesson_index = if let Some((_, index)) = target {
            index
        } else {
            let id = format!("resource-{}", digest(&asset_id));
            match section.lessons.iter().position(|lesson| lesson.id == id) {
                Some(index) => index,
                None => {
                    section.lessons.push(CourseLesson {
                        id,
                        title: resource.filename.clone(),
                        learned: false,
                        notes: Vec::new(),
                        assets: Vec::new(),
                    });
                    section.lessons.len() - 1
                }
            }
        };
        let assets = &mut section.lessons[lesson_index].assets;
        if let Some(asset) = assets.iter_mut().find(|asset| asset.id == asset_id) {
            asset.title = resource.filename;
            asset.url_ref = Some(url_ref);
            continue;
        }
        assets.push(CourseAsset {
            id: asset_id,
            title: resource.filename,
            kind: "attachment".into(),
            status: "downloadable".into(),
            restriction: None,
            library_id: None,
            url_ref: Some(url_ref),
            job_ids: Vec::new(),
        });
    }
    save_course(repo, course)
}

pub(super) fn jobs_for<'a>(
    repo: &'a Repository,
    asset_id: &'a str,
) -> impl Iterator<Item = &'a queue::Job> {
    repo.data.jobs.iter().filter(move |job| matches!(&job.source, queue::Source::PublicFile { selection } if selection.asset_id == asset_id))
}

pub(super) fn enqueue(
    repo: &mut Repository,
    course: &Course,
    ids: &[String],
    request: &str,
) -> Result<Vec<queue::Job>, String> {
    let mut tasks = Vec::new();
    let mut collection = label(&course.title, 80);
    while collection.len() > 120 {
        collection.pop();
    }
    for asset in course
        .sections
        .iter()
        .flat_map(|section| &section.lessons)
        .flat_map(|lesson| &lesson.assets)
        .filter(|asset| asset.status == "downloadable" && ids.contains(&asset.id))
    {
        if jobs_for(repo, &asset.id).any(|job| {
            job.client_request != request
                && (job.state.active()
                    || matches!(
                        job.state,
                        queue::JobState::Pending | queue::JobState::Paused
                    )
                    || job.state == queue::JobState::Completed
                        && job.outputs.iter().any(|path| path.is_file()))
        }) {
            continue;
        }
        let reference = asset
            .url_ref
            .as_deref()
            .ok_or("Add the authorized Skillshare resource link again.")?;
        let resource = parse(sessions::resolve_url(repo.root(), reference)?.as_str())?;
        if asset.id != resource.identity(&course.id) {
            return Err("The resource identity changed. Review the link again.".into());
        }
        let selection = PublicSelection {
            source_url: resource.url.to_string(),
            asset_id: asset.id.clone(),
            filename: resource.filename,
            sha256: None,
            version: Some(resource.revision),
            size: None,
            author: None,
            license: None,
            attribution: None,
            collection: collection.clone(),
            published_at: None,
            checksum: None,
            audiobook: None,
        };
        tasks.push((queue::Source::PublicFile { selection }, asset.title.clone()));
    }
    if tasks.is_empty() {
        return Err("The selection has no new downloadable Skillshare attachments.".into());
    }
    queue::enqueue_tasks(repo, tasks, request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skillshare_review_labels_duplicates_and_unsupported_lines() {
        let base = "https://static.skillshare.com/uploads/attachment/12345678";
        let reviewed = review_skillshare_resources(vec![
            format!("{base}/90abcdef/Guide.pdf"),
            format!("{base}/90abcdef/Renamed.pdf"),
            format!("{base}/90abcdef/Guide.pdf?token=private"),
            format!("{base}/90abcdee/Updated.pdf"),
        ])
        .unwrap();
        assert_eq!(
            serde_json::to_value(reviewed).unwrap(),
            serde_json::json!([
                {"kind": "supported", "filename": "Guide.pdf"},
                {"kind": "duplicate", "filename": "Renamed.pdf"},
                {"kind": "unsupported"},
                {"kind": "supported", "filename": "Updated.pdf"}
            ])
        );
    }

    #[test]
    fn skillshare_resource_links_accept_only_direct_supported_attachments() {
        let base = "https://static.skillshare.com/uploads/attachment/12345678/90abcdef/";
        let course = format!("skillshare-{}", "a".repeat(64));
        let resource = parse(&format!("{base}Gu%C3%ADa%20de%20clase.pdf")).unwrap();
        assert_eq!(resource.filename, "Guía de clase.pdf");
        assert_eq!(
            resource.identity(&course),
            format!("skillshare:{course}:12345678:90abcdef")
        );
        assert_eq!(
            parse(&format!("{base}renamed.pdf"))
                .unwrap()
                .identity(&course),
            resource.identity(&course)
        );
        for name in [
            "chapter.mp4",
            "script.exe",
            "file.html",
            "CON.pdf",
            "bad%2Ffile.pdf",
            "file.pdf?token=secret",
            "file.pdf#preview",
        ] {
            assert!(parse(&format!("{base}{name}")).is_err(), "accepted {name}");
        }
        assert!(parse("https://static.skillshare.com.example.org/uploads/attachment/12345678/90abcdef/file.pdf").is_err());
        assert!(parse("https://www.skillshare.com/classes/Design/12345").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn skillshare_course_queues_selected_attachments_and_preserves_study() {
        use std::{fs, time::SystemTime};
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let root = parent.join(format!(
            "nabnab-skillshare-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        {
            let mut repo = Repository::open(&root, &root).unwrap();
            let class = "https://www.skillshare.com/classes/Fixture/123456";
            let first =
                "https://static.skillshare.com/uploads/attachment/12345678/90abcdef/Guide.pdf"
                    .to_owned();
            let second =
                "https://static.skillshare.com/uploads/attachment/12345679/90abcdef/Templates.zip"
                    .to_owned();
            let course =
                register(&mut repo, class, "Fixture", &[first.clone(), second], None).unwrap();
            let id = course.sections[0].lessons[0].assets[0].id.clone();
            assert_eq!(repo.data.jobs.len(), 0);
            let jobs = enqueue(
                &mut repo,
                &course,
                std::slice::from_ref(&id),
                "selected-fixture",
            )
            .unwrap();
            assert_eq!(jobs.len(), 1);
            let queue::Source::PublicFile { selection } = &jobs[0].source else {
                panic!("expected a direct attachment job")
            };
            assert_eq!(selection.filename, "Guide.pdf");
            assert_eq!(selection.source_url, first);
            assert_eq!(
                enqueue(
                    &mut repo,
                    &course,
                    std::slice::from_ref(&id),
                    "selected-fixture"
                )
                .unwrap()[0]
                    .id,
                jobs[0].id
            );
            assert!(enqueue(
                &mut repo,
                &course,
                std::slice::from_ref(&id),
                "duplicate-fixture"
            )
            .is_err());
            let lesson = &mut repo.data.courses[0].sections[0].lessons[0];
            lesson.learned = true;
            lesson.notes.push(super::super::CourseNote {
                id: "note-fixture".into(),
                text: "Keep this note".into(),
                time: None,
                bookmark_id: None,
            });
            let refreshed = register(
                &mut repo,
                class,
                "Fixture",
                std::slice::from_ref(&first),
                None,
            )
            .unwrap();
            assert_eq!(refreshed.sections[0].lessons.len(), 2);
            assert!(refreshed.sections[0].lessons[0].learned);
            assert_eq!(
                refreshed.sections[0].lessons[0].notes[0].text,
                "Keep this note"
            );
            repo.data.courses[0].sections.push(CourseSection {
                id: "unit-2".into(),
                title: "Unit 2".into(),
                lessons: vec![CourseLesson {
                    id: "lesson-2".into(),
                    title: "Build the design".into(),
                    learned: true,
                    notes: refreshed.sections[0].lessons[0].notes.clone(),
                    assets: vec![CourseAsset {
                        id: "local-video".into(),
                        title: "Lesson.mp4".into(),
                        kind: "video".into(),
                        status: "local".into(),
                        restriction: None,
                        library_id: None,
                        url_ref: None,
                        job_ids: Vec::new(),
                    }],
                }],
            });
            let third =
                "https://static.skillshare.com/uploads/attachment/12345680/90abcdef/Reference.pdf"
                    .to_owned();
            let links = std::slice::from_ref(&third);
            assert!(register(
                &mut repo,
                class,
                "Fixture",
                links,
                Some("other-course-lesson")
            )
            .is_err());
            assert!(register(
                &mut repo,
                class,
                "Fixture",
                links,
                Some(&refreshed.sections[0].lessons[0].id)
            )
            .is_err());
            assert!(
                register(&mut repo, class, "Fixture", &[first], Some("lesson-2"))
                    .unwrap_err()
                    .contains("already listed")
            );
            let attached = register(&mut repo, class, "Fixture", links, Some("lesson-2")).unwrap();
            assert_eq!(
                attached.sections[1].lessons[0].assets[1].title,
                "Reference.pdf"
            );
            let attached = register(&mut repo, class, "Fixture", links, Some("lesson-2")).unwrap();
            assert_eq!(attached.sections[0].lessons.len(), 2);
            let lesson = &attached.sections[1].lessons[0];
            assert!(lesson.learned);
            assert_eq!(lesson.notes[0].text, "Keep this note");
            assert_eq!(
                lesson
                    .assets
                    .iter()
                    .map(|asset| asset.title.as_str())
                    .collect::<Vec<_>>(),
                ["Lesson.mp4", "Reference.pdf"]
            );
            assert!(register(&mut repo, class, "Fixture", links, None)
                .unwrap_err()
                .contains("Build the design"));
            assert_eq!(repo.data.jobs.len(), 1);
            let queued = enqueue(
                &mut repo,
                &attached,
                std::slice::from_ref(&lesson.assets[1].id),
                "lesson-resource",
            )
            .unwrap();
            assert_eq!(queued.len(), 1);
            let queue::Source::PublicFile { selection } = &queued[0].source else {
                panic!("expected a direct attachment job")
            };
            assert_eq!(selection.source_url, third);
            drop(repo);
            let reopened = Repository::open(&root, &root).unwrap();
            assert_eq!(jobs_for(&reopened, &id).next().unwrap().id, jobs[0].id);
            assert_eq!(
                reopened.data.courses[0].sections[1].lessons[0].assets[1].title,
                "Reference.pdf"
            );
        }
        let resolved = root.canonicalize().unwrap();
        assert_eq!(resolved.parent(), Some(parent.as_path()));
        fs::remove_dir_all(resolved).unwrap();
    }
}
