//! Coursera Business course materials describe the syllabus, not downloadable files.
use super::{
    browser, digest, label, members, save_course, Course, CourseAsset, CourseLesson, CourseSection,
    OutlineStatus,
};
use crate::{queue, AppState};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};
use tauri::State;
use url::Url;

pub(super) const MAX_OUTLINE_PAGES: u8 = 8;

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

fn parse(data: Value, course_id: &str) -> Result<(Vec<CourseSection>, Value), String> {
    let page: Page = serde_json::from_value(data)
        .map_err(|_| "Coursera Business returned an unreadable course outline.")?;
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
                    source_url: None,
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
    Ok((sections, page.paging))
}

fn next_page(current: &Url, link: &str, start: u32) -> Option<(Url, u32)> {
    if link.len() > 2048 || link.chars().any(char::is_control) {
        return None;
    }
    let next = current.join(link).ok()?;
    if next.origin() != current.origin()
        || next.path() != current.path()
        || !next.username().is_empty()
        || next.password().is_some()
        || next.fragment().is_some()
    {
        return None;
    }
    let mut next_start = None;
    let mut limit = None;
    for (key, value) in next.query_pairs() {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let value = value.parse::<u32>().ok()?;
        match key.as_ref() {
            "start" if next_start.is_none() && value > start => next_start = Some(value),
            "limit" if limit.is_none() && (1..=1000).contains(&value) => limit = Some(value),
            _ => return None,
        }
    }
    Some((next, next_start?))
}

fn append_sections(outline: &mut Vec<CourseSection>, incoming: Vec<CourseSection>) -> bool {
    let mut advanced = false;
    for section in incoming {
        let Some(existing) = outline.iter_mut().find(|item| item.id == section.id) else {
            outline.push(section);
            advanced = true;
            continue;
        };
        for lesson in section.lessons {
            let Some(existing) = existing
                .lessons
                .iter_mut()
                .find(|item| item.id == lesson.id)
            else {
                existing.lessons.push(lesson);
                advanced = true;
                continue;
            };
            for asset in lesson.assets {
                if existing.assets.iter().all(|item| item.id != asset.id) {
                    existing.assets.push(asset);
                    advanced = true;
                }
            }
        }
    }
    advanced
}

async fn read_outline<F>(
    mut url: Url,
    course_id: &str,
    mut fetch: impl FnMut(Url) -> F,
) -> Result<(Vec<CourseSection>, OutlineStatus), String>
where
    F: Future<Output = Result<Value, String>>,
{
    let mut sections = Vec::new();
    let mut start = 0;
    let mut status = OutlineStatus {
        pages: 0,
        partial: false,
    };
    for page in 1..=MAX_OUTLINE_PAGES {
        let (incoming, paging) = parse(fetch(url.clone()).await?, course_id)?;
        let advanced = append_sections(&mut sections, incoming);
        let link = paging["links"]
            .as_array()
            .and_then(|links| links.iter().find(|link| link["rel"] == "next"));
        status = OutlineStatus {
            pages: page,
            partial: !paging["next"].is_null() || link.is_some() || page > 1 && !advanced,
        };
        if page == MAX_OUTLINE_PAGES || page > 1 && !advanced {
            break;
        }
        let Some((next, offset)) = link
            .and_then(|link| link["href"].as_str())
            .and_then(|link| next_page(&url, link, start))
        else {
            break;
        };
        (url, start) = (next, offset);
    }
    if !sections.iter().any(|section| !section.lessons.is_empty()) {
        return Err("Coursera Business did not return any lessons for this course.".into());
    }
    Ok((sections, status))
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
) -> Result<Course, String> {
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
    let endpoint = Url::parse(&endpoint).map_err(|_| "Invalid Coursera course API URL.")?;
    let (sections, status) = tokio::time::timeout(
        Duration::from_secs(60),
        read_outline(endpoint, course_id, |url| async move {
            members::get_json("coursera", token, url.as_str(), "", "").await
        }),
    )
    .await
    .map_err(|_| "Coursera Business did not finish loading the outline within 60 seconds.")??;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    store_outline(
        repo,
        Course {
            id,
            title,
            provider: "coursera".into(),
            url: Some(url),
            slug: Some(course_id.to_owned()),
            sections,
            created_at: queue::now(),
            outline_status: Some(status),
        },
    )
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
    use std::future::ready;

    fn endpoint() -> Url {
        Url::parse("https://api.coursera.com/ent/api/businesses.v1/org/programs/program/courseMaterials/course-id").unwrap()
    }

    #[tokio::test]
    async fn paginated_outline_keeps_order_study_and_local_materials() {
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
            "paging":{"next":"2", "links":[{"rel":"next","href":"?start=2&limit=2"}]}
        });
        assert!(parse(response.clone(), "different-id").is_err());
        let next = json!({ "elements": [{"id":"course-id","modules":[
            {"id":"mod-2","name":"First in the syllabus","lessons":[{"id":"lesson","name":"Start here","items":[
                {"id":"text","name":"Handout","itemType":"READING"}, {"id":"quiz","name":"Check understanding","itemType":"QUIZ"}
            ]}]},
            {"id":"mod-3","name":"Last in the syllabus","lessons":[{"id":"last","name":"Finish","items":[]}]}
        ]}], "paging":{} });
        let mut requested = Vec::new();
        let (sections, status) = read_outline(endpoint(), "course-id", |url| {
            requested.push(url.query().map(str::to_owned));
            ready(Ok(if url.query().is_none() {
                response.clone()
            } else {
                next.clone()
            }))
        })
        .await
        .unwrap();
        assert_eq!(requested, [None, Some("start=2&limit=2".into())]);
        assert_eq!(status.pages, 2);
        assert!(!status.partial);
        assert_eq!(sections[0].title, "First in the syllabus");
        assert_eq!(sections[1].title, "Second in the syllabus");
        assert_eq!(sections[2].title, "Last in the syllabus");
        let items = &sections[0].lessons[0].assets;
        assert_eq!(
            items
                .iter()
                .map(|item| item.kind.as_str())
                .collect::<Vec<_>>(),
            ["video", "reading", "quiz"]
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
            outline_status: Some(status),
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
        assert_eq!(lesson.assets[3].library_id.as_deref(), Some("local-file"));
        assert_eq!(refreshed.created_at, 17);
        assert_eq!(refreshed.outline_status.unwrap().pages, 2);
        assert!(repo.data.jobs.is_empty());
        let mut other_course = outline;
        other_course.slug = Some("other".into());
        assert!(store_outline(&mut repo, other_course).is_err());
        drop(repo);
        let reopened = crate::store::Repository::open(&root, &root).unwrap();
        assert_eq!(reopened.data.courses[0].outline_status.unwrap().pages, 2);
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn continuations_keep_the_same_course_and_forward_offset() {
        let endpoint = endpoint();
        assert_eq!(next_page(&endpoint, "?start=5", 2).unwrap().1, 5);
        for link in [
            "https://example.org/course?start=5",
            "//api.coursera.com:444/course?start=5",
            "/ent/api/businesses.v1/org/programs/program/courseMaterials/other?start=5",
            "?start=5&token=secret",
            "?start=5&start=9",
            "?start=2",
            "?start=1",
            "?start=5#fragment",
            "?start=5&limit=0",
            "?start=not-a-number",
            "?limit=100",
        ] {
            assert!(
                next_page(&endpoint, link, 2).is_none(),
                "Unexpected continuation: {link}"
            );
        }
    }

    #[tokio::test]
    async fn unsupported_continuations_repeated_pages_and_limits_leave_a_partial_outline() {
        let response = |id: usize, paging: Value| {
            json!({"elements":[{"id":"course-id","modules":[
            {"id":format!("module-{id}"),"name":"Module","lessons":[{"id":"lesson","name":"Lesson","items":[]}]}
        ]}], "paging": paging})
        };
        let link = |start| json!({"links":[{"rel":"next","href":format!("?start={start}")}]});

        let (_, unknown) = read_outline(endpoint(), "course-id", |url| {
            assert!(url.query().is_none());
            ready(Ok(response(0, json!({"next":"opaque cursor"}))))
        })
        .await
        .unwrap();
        assert!(unknown.partial);
        assert_eq!(unknown.pages, 1);

        let (sections, repeated) = read_outline(endpoint(), "course-id", |url| {
            ready(Ok(response(
                0,
                if url.query().is_none() {
                    link(1)
                } else {
                    json!({})
                },
            )))
        })
        .await
        .unwrap();
        assert!(repeated.partial);
        assert_eq!(repeated.pages, 2);
        assert_eq!(sections.len(), 1);

        let mut requests = 0;
        let (sections, bounded) = read_outline(endpoint(), "course-id", |_| {
            requests += 1;
            ready(Ok(response(requests, link(requests))))
        })
        .await
        .unwrap();
        assert!(bounded.partial);
        assert_eq!(bounded.pages, MAX_OUTLINE_PAGES);
        assert_eq!(sections.len(), usize::from(MAX_OUTLINE_PAGES));
    }
}
