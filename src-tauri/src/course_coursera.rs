//! Coursera Business course materials describe the syllabus, not downloadable files.
use super::{
    browser, digest, label, members, save_course, Course, CourseAsset, CourseLesson, CourseSection,
};
use crate::{queue, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tauri::State;

#[derive(Deserialize)]
struct Page {
    elements: Vec<Materials>,
    #[serde(default)]
    paging: Value,
}

#[derive(Deserialize)]
struct Materials {
    id: String,
    modules: Vec<Module>,
}

#[derive(Deserialize)]
struct Module {
    id: String,
    name: String,
    lessons: Vec<Lesson>,
}

#[derive(Deserialize)]
struct Lesson {
    id: String,
    name: String,
    items: Vec<Item>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    name: String,
    item_type: String,
}

#[derive(Serialize)]
pub struct Inspection {
    course: Course,
    partial: bool,
}

fn parse(data: Value, course_id: &str) -> Result<(Vec<CourseSection>, bool), String> {
    let page: Page = serde_json::from_value(data)
        .map_err(|_| "Coursera Business returned an unreadable course outline.")?;
    let partial = !page.paging["next"].is_null()
        || page.paging["links"]
            .as_array()
            .is_some_and(|links| links.iter().any(|link| link["rel"] == "next"));
    let mut sections = Vec::new();
    for course in page.elements {
        if course.id != course_id {
            return Err("Coursera Business returned an outline for a different course.".into());
        }
        for module in course.modules {
            if !members::identifier(&module.id) {
                return Err("Coursera Business returned an invalid module ID.".into());
            }
            let mut lessons = Vec::new();
            for lesson in module.lessons {
                if !members::identifier(&lesson.id)
                    || lesson
                        .items
                        .iter()
                        .any(|item| !members::identifier(&item.id))
                {
                    return Err(
                        "Coursera Business returned an invalid lesson or material ID.".into(),
                    );
                }
                let assets = lesson
                    .items
                    .into_iter()
                    .map(|item| CourseAsset {
                        id: format!(
                            "coursera-item-{}",
                            digest(&format!("{}:{}:{}", module.id, lesson.id, item.id))
                        ),
                        title: label(&item.name, 240),
                        kind: match item.item_type.as_str() {
                            "VIDEO" => "video",
                            "READING" => "reading",
                            "QUIZ" => "quiz",
                            "ASSIGNMENT" | "PROGRAMMING_ASSIGNMENT" | "PEER_REVIEW" => "assignment",
                            _ => "course-item",
                        }
                        .into(),
                        status: "unavailable".into(),
                        restriction: None,
                        library_id: None,
                        url_ref: None,
                        job_ids: Vec::new(),
                    })
                    .collect();
                lessons.push(CourseLesson {
                    id: format!(
                        "coursera-lesson-{}",
                        digest(&format!("{}:{}", module.id, lesson.id))
                    ),
                    title: label(&lesson.name, 240),
                    learned: false,
                    assets,
                    notes: Vec::new(),
                });
            }
            sections.push(CourseSection {
                id: format!("coursera-module-{}", digest(&module.id)),
                title: label(&module.name, 240),
                lessons,
            });
        }
    }
    if !sections.iter().any(|section| !section.lessons.is_empty()) {
        return Err("Coursera Business did not return any lessons for this course.".into());
    }
    Ok((sections, partial))
}

#[tauri::command]
pub async fn inspect_coursera_course(
    state: State<'_, Arc<AppState>>,
    url: String,
    title: String,
    organization: String,
    program: String,
    course_id: String,
    token: String,
) -> Result<Inspection, String> {
    let (id, url) = browser::authorized_course("coursera", &url)?;
    let title = label(&title, 240);
    let (organization, program, course_id, token) = (
        organization.trim(),
        program.trim(),
        course_id.trim(),
        token.trim(),
    );
    if title.is_empty()
        || ![organization, program, course_id]
            .into_iter()
            .all(members::identifier)
        || !members::valid_access_token(token)
    {
        return Err(
            "Enter the course name, three Coursera Business IDs and an authorized access token."
                .into(),
        );
    }
    let endpoint = format!("https://api.coursera.com/ent/api/businesses.v1/{organization}/programs/{program}/courseMaterials/{course_id}");
    let (sections, partial) = parse(
        members::get_json("coursera", token, &endpoint, "", "").await?,
        course_id,
    )?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let course = store_outline(
        repo,
        Course {
            id,
            title,
            provider: "coursera".into(),
            url: Some(url),
            slug: Some(course_id.to_owned()),
            sections,
            created_at: queue::now(),
        },
    )?;
    Ok(Inspection { course, partial })
}

fn store_outline(
    repo: &mut crate::store::Repository,
    mut course: Course,
) -> Result<Course, String> {
    let previous = repo.data.courses.iter().find(|old| old.id == course.id);
    if let Some(previous) = previous {
        if previous.slug.is_some() && previous.slug != course.slug {
            return Err(
                "This saved course is linked to a different Coursera Business course ID.".into(),
            );
        }
        course.created_at = previous.created_at;
        let old_lessons: HashMap<_, _> = previous
            .sections
            .iter()
            .flat_map(|section| &section.lessons)
            .map(|lesson| (&lesson.id, lesson))
            .collect();
        for lesson in course
            .sections
            .iter_mut()
            .flat_map(|section| &mut section.lessons)
        {
            if let Some(old) = old_lessons.get(&lesson.id) {
                lesson.assets.extend(
                    old.assets
                        .iter()
                        .filter(|asset| asset.status == "local")
                        .cloned(),
                );
            }
        }
    }
    save_course(repo, course)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn authorized_outline_keeps_order_study_and_local_materials() {
        let response = json!({
            "elements": [{"id":"course-id","modules":[
                {"id":"mod-2","name":"First in the syllabus","lessons":[
                    {"id":"lesson","name":"Start here","items":[
                        {"id":"clip","name":"Lecture","itemType":"VIDEO","deeplinkUrl":"https://www.coursera.org/programs/example?attemptSSOLogin=true"},
                        {"id":"text","name":"Handout","itemType":"READING","downloadUrl":"https://example.test/unrecognized.pdf"}
                    ]}
                ]},
                {"id":"mod-1","name":"Second in the syllabus","lessons":[
                    {"id":"next","name":"Review","items":[]}
                ]}
            ]}],
            "paging":{"next":"sample"}
        });
        assert!(parse(response.clone(), "different-id").is_err());
        let (sections, partial) = parse(response, "course-id").unwrap();
        assert!(partial);
        assert_eq!(sections[0].title, "First in the syllabus");
        assert_eq!(sections[1].title, "Second in the syllabus");
        let items = &sections[0].lessons[0].assets;
        assert_eq!(
            items
                .iter()
                .map(|item| item.kind.as_str())
                .collect::<Vec<_>>(),
            ["video", "reading"]
        );
        assert!(items
            .iter()
            .all(|item| item.status == "unavailable" && item.url_ref.is_none()));
        let root =
            std::env::temp_dir().join(format!("nabnab-coursera-outline-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut repo = crate::store::Repository::open(&root, &root).unwrap();
        let outline = Course {
            id: "coursera-fixture".into(),
            title: "Course".into(),
            provider: "coursera".into(),
            url: Some("https://www.coursera.org/learn/fixture/home".into()),
            slug: Some("course-id".into()),
            sections,
            created_at: 17,
        };
        store_outline(&mut repo, outline.clone()).unwrap();
        let lesson = &mut repo.data.courses[0].sections[0].lessons[0];
        lesson.learned = true;
        lesson.notes.push(super::super::CourseNote {
            id: "note".into(),
            text: "Keep this note".into(),
            time: None,
            bookmark_id: None,
        });
        lesson.assets.push(CourseAsset {
            id: "local-attachment".into(),
            title: "Saved notes.pdf".into(),
            kind: "document".into(),
            status: "local".into(),
            restriction: None,
            library_id: Some("local-file".into()),
            url_ref: None,
            job_ids: Vec::new(),
        });
        let refreshed = store_outline(&mut repo, outline.clone()).unwrap();
        let lesson = &refreshed.sections[0].lessons[0];
        assert!(lesson.learned);
        assert_eq!(lesson.notes[0].text, "Keep this note");
        assert_eq!(lesson.assets[2].library_id.as_deref(), Some("local-file"));
        assert_eq!(refreshed.created_at, 17);
        assert!(repo.data.jobs.is_empty());
        let mut other_course = outline;
        other_course.slug = Some("other".into());
        assert!(store_outline(&mut repo, other_course).is_err());
        drop(repo);
        std::fs::remove_dir_all(root).unwrap();
    }
}
