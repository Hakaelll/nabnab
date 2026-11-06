use super::*;
use crate::options::{
    self, Clip, Conditions, Context, PreparedDownload, Rule, SubtitleMode, WorkOptions,
};

fn repository(label: &str) -> Repository {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "nabnab-productivity-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("media")).unwrap();
    Repository::open(&root, &root.join("media")).unwrap()
}
fn prepared(repo: &Repository, values: WorkOptions) -> PreparedDownload {
    let url = "https://www.youtube.com/watch?v=authorized-fixture";
    let context = Context {
        url: url.into(),
        title: "Camera lesson".into(),
        creator: "Local fixture".into(),
        platform: "YouTube".into(),
        content_id: "authorized-fixture".into(),
        content_type: "video".into(),
        ..Default::default()
    };
    PreparedDownload {
        url: url.into(),
        title: context.title.clone(),
        content_id: options::content_identity(url).unwrap(),
        options: options::resolve(repo, &context, None, Some(&values)).unwrap(),
        source_id: None,
        group_id: Some("fixture-group".into()),
    }
}
#[test]
fn related_outputs_are_independent_and_partial_success_does_not_satisfy_a_preset() {
    let mut repo = repository("partial");
    let item = prepared(
        &repo,
        WorkOptions {
            additional_audio: Some(Format::Mp3),
            subtitle_languages: vec!["es".into()],
            subtitle_mode: SubtitleMode::Sidecar,
            ..Default::default()
        },
    );
    let jobs = enqueue_prepared(&mut repo, vec![item.clone()], "fixture-first").unwrap();
    assert_eq!(jobs.len(), 3);
    assert_eq!(
        repo.data
            .jobs
            .iter()
            .filter(|j| j.options.as_ref().unwrap().values.subtitle_mode == SubtitleMode::Only)
            .count(),
        1
    );
    for (index, job) in repo.data.jobs.iter_mut().enumerate() {
        job.state = if index == 1 {
            JobState::Error
        } else {
            JobState::Completed
        };
        if index != 1 {
            let path = job.folder.join(format!("output-{index}.fixture"));
            fs::write(&path, b"verified fixture").unwrap();
            job.outputs = vec![path];
        }
    }
    assert_eq!(prepared_status(&repo, &item), None);
    let accepted = enqueue_prepared(&mut repo, vec![item.clone()], "fixture-retry-output").unwrap();
    assert_eq!(repo.data.jobs.len(), 4);
    assert_eq!(accepted.len(), 3);
    assert_eq!(
        accepted
            .iter()
            .filter(|j| j.state == JobState::Pending)
            .count(),
        1
    );
    assert_eq!(
        accepted
            .iter()
            .find(|j| j.state == JobState::Pending)
            .unwrap()
            .options
            .as_ref()
            .unwrap()
            .values
            .format,
        Format::Mp3
    );
    assert_eq!(prepared_status(&repo, &item), Some("active"));
}
#[test]
fn content_aliases_are_idempotent_but_fragments_and_subtitles_are_different_outputs() {
    let mut repo = repository("identities");
    let full = prepared(&repo, WorkOptions::default());
    let first = enqueue_prepared(&mut repo, vec![full.clone()], "manual-fixture").unwrap();
    let mut followed = full.clone();
    followed.url = "https://youtu.be/authorized-fixture?t=3".into();
    followed.source_id = Some("followed-fixture".into());
    let duplicate = enqueue_prepared(&mut repo, vec![followed], "automatic-fixture").unwrap();
    assert_eq!(duplicate[0].id, first[0].id);
    assert_eq!(repo.data.jobs.len(), 1);
    let clip = prepared(
        &repo,
        WorkOptions {
            clip: Some(Clip {
                start: 3.0,
                end: 8.0,
                precise: false,
            }),
            ..Default::default()
        },
    );
    let subs = prepared(
        &repo,
        WorkOptions {
            subtitle_mode: SubtitleMode::Only,
            subtitle_languages: vec!["es".into()],
            ..Default::default()
        },
    );
    let accepted = enqueue_prepared(&mut repo, vec![clip, subs], "other-outputs").unwrap();
    assert_eq!(accepted.len(), 2);
    assert_eq!(repo.data.jobs.len(), 3);
    assert_ne!(
        operation_key(&repo.data.jobs[0].options.as_ref().unwrap().values),
        operation_key(&repo.data.jobs[1].options.as_ref().unwrap().values)
    );
    assert_eq!(
        enqueue_prepared(&mut repo, vec![full], "manual-fixture").unwrap()[0].id,
        first[0].id
    );
}
#[test]
fn rule_precedence_merges_tags_and_queued_options_do_not_follow_preset_edits() {
    let mut repo = repository("rules");
    repo.data.rules = vec![
        Rule {
            id: "tutorial-rule".into(),
            name: "Tutorials".into(),
            enabled: true,
            conditions: Conditions {
                title: "camera".into(),
                ..Default::default()
            },
            preset_id: Some("reference".into()),
            relative_folder: Some("Tutorials/{creador}".into()),
            collection: Some("Tutorials".into()),
            tags: vec!["learning".into(), "LEARNING".into()],
        },
        Rule {
            id: "later-rule".into(),
            name: "Must not win".into(),
            enabled: true,
            conditions: Default::default(),
            preset_id: None,
            relative_folder: Some("wrong".into()),
            collection: None,
            tags: vec![],
        },
    ];
    let item = prepared(&repo, WorkOptions::default());
    let ruled = options::resolve(&repo, &item.options.context, None, None).unwrap();
    assert_eq!(ruled.values.relative_folder, "Tutorials/{creador}");
    assert_eq!(ruled.values.tags, vec!["learning"]);
    let preset = options::resolve(&repo, &item.options.context, Some("audio"), None).unwrap();
    assert_eq!(preset.values.format, Format::AudioOriginal);
    assert_eq!(preset.values.relative_folder, "{plataforma}");
    assert_eq!(preset.values.tags, vec!["learning"]);
    let frozen = PreparedDownload {
        options: preset,
        ..item
    };
    enqueue_prepared(&mut repo, vec![frozen], "frozen-fixture").unwrap();
    repo.data
        .presets
        .iter_mut()
        .find(|p| p.id == "audio")
        .unwrap()
        .options
        .format = Format::Mp3;
    let stored = repo.data.jobs[0].options.as_ref().unwrap();
    assert_eq!(stored.preset_revision, Some(1));
    assert_eq!(stored.values.format, Format::AudioOriginal);
    assert!(stored.origins["format"].starts_with("Ajuste: Audio"));
    assert_eq!(stored.origins["tag:learning"], "Regla: Tutorials");
}
#[test]
fn invalid_second_item_never_partially_enqueues_the_first() {
    let mut repo = repository("atomic");
    let valid = prepared(&repo, WorkOptions::default());
    let mut invalid = valid.clone();
    invalid.options.values.relative_folder = "../outside".into();
    assert!(enqueue_prepared(&mut repo, vec![valid, invalid], "invalid-fixture").is_err());
    assert!(repo.data.jobs.is_empty());
}

#[test]
fn scheduled_local_tools_keep_their_pending_schedule_on_restart() {
    let mut job = Job::new(
        Source::Transform {
            library_id: "local-fixture".into(),
            options: ToolOptions::Trim {
                start: 0.0,
                end: 1.0,
                precise: false,
            },
        },
        "Local clip".into(),
        std::env::temp_dir(),
        "scheduled-tool-fixture".into(),
        None,
    );
    job.schedule = crate::scheduling::Schedule::Once {
        at: now() + 86_400_000,
        timezone: "UTC".into(),
        missed: crate::scheduling::MissedPolicy::RunWhenPossible,
        grace_minutes: 15,
    };
    job.recover();
    assert_eq!(job.state, JobState::Pending);
    job.schedule = crate::scheduling::Schedule::Immediate;
    job.recover();
    assert_eq!(job.state, JobState::Paused);
}

#[test]
fn protected_operational_urls_do_not_leak_through_prepared_context_or_identity() {
    let mut repo = repository("private-context");
    let url = "https://cdn.example.org/private-path?accessToken=SECRET#PRIVATE";
    let context = Context {
        url: url.into(),
        title: "Private fixture".into(),
        content_id: options::content_identity(url).unwrap(),
        ..Default::default()
    };
    let resolved = options::resolve(&repo, &context, None, None).unwrap();
    let jobs = enqueue_prepared(
        &mut repo,
        vec![PreparedDownload {
            url: url.into(),
            title: context.title,
            content_id: context.content_id,
            options: resolved,
            source_id: None,
            group_id: None,
        }],
        "private-fixture",
    )
    .unwrap();
    let json = serde_json::to_string(&repo.data).unwrap();
    for secret in ["SECRET", "private-path", "PRIVATE"] {
        assert!(!json.contains(secret), "{secret}");
    }
    let Source::Download {
        url_ref: Some(reference),
        ..
    } = &jobs[0].source
    else {
        panic!("protected download expected")
    };
    assert_eq!(
        crate::sessions::resolve_url(repo.root(), reference)
            .unwrap()
            .as_str(),
        url
    );
}

#[test]
fn reused_outputs_gain_source_and_collection_relations_without_mutating_original_options() {
    let mut repo = repository("relations");
    let first = prepared(&repo, WorkOptions::default());
    let jobs = enqueue_prepared(&mut repo, vec![first.clone()], "original-fixture").unwrap();
    let file = repo.data.settings.folder.join("existing.mp4");
    fs::write(&file, b"local fixture").unwrap();
    let mut local = library::index(&file, None, None, None).unwrap();
    local.details.content_id = Some(first.content_id.clone());
    repo.data.library.push(local);
    repo.data.jobs[0].state = JobState::Completed;
    repo.data.jobs[0].outputs = vec![repo.data.library[0].path.clone()];
    let mut followed = first;
    followed.source_id = Some("second-source".into());
    followed.options.values.collection = Some("References".into());
    followed.options.values.tags = vec!["learning".into()];
    let reused = enqueue_prepared(&mut repo, vec![followed], "followed-fixture").unwrap();
    assert_eq!(reused[0].id, jobs[0].id);
    assert_eq!(repo.data.jobs.len(), 1);
    assert_eq!(repo.data.library.len(), 1);
    assert_eq!(
        repo.data.library[0].details.source_ids,
        vec!["second-source"]
    );
    assert_eq!(repo.data.library[0].collections, vec!["References"]);
    assert_eq!(repo.data.library[0].tags, vec!["learning"]);
    assert_eq!(repo.data.collections, vec!["References"]);
    assert_eq!(
        repo.data.jobs[0]
            .options
            .as_ref()
            .unwrap()
            .values
            .collection,
        None
    );
}

#[tokio::test]
#[ignore = "Requires bundled FFmpeg/ffprobe; uses only a synthetic local video and subtitles"]
async fn real_engines_clip_subtitles_are_rebased_before_embedding() {
    let engines = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines");
    let repo = repository("clip-subtitles");
    let staging = repo.data.settings.folder.join("stage");
    fs::create_dir(&staging).unwrap();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../artifacts/fixtures/nabnab prueba.mp4");
    let (_sender, cancel) = watch::channel(false);
    let clip = crate::tools::execute(
        &engines,
        &fixture,
        &staging,
        &ToolOptions::Trim {
            start: 1.0,
            end: 4.0,
            precise: true,
        },
        cancel.clone(),
        1,
        |_| {},
    )
    .await
    .unwrap();
    fs::write(staging.join("track.en.srt"),"1\n00:00:00,500 --> 00:00:01,500\nFirst clipped phrase\n\n2\n00:00:03,500 --> 00:00:04,500\nLast clipped phrase\n").unwrap();
    let options = WorkOptions {
        clip: Some(Clip {
            start: 1.0,
            end: 4.0,
            precise: true,
        }),
        subtitle_mode: SubtitleMode::Embed,
        subtitle_languages: vec!["en".into()],
        ..Default::default()
    };
    let output = embed_clip_subtitles(&engines, &clip, &staging, &options, cancel.clone())
        .await
        .unwrap();
    let info = media::probe(&engines, &output, cancel.clone())
        .await
        .unwrap();
    media::validate_subtitle(&info, "en").unwrap();
    let extracted = staging.join("extracted.srt");
    engine::tool(
        &engines,
        "ffmpeg.exe",
        vec![
            "-nostdin".into(),
            "-v".into(),
            "error".into(),
            "-n".into(),
            "-i".into(),
            output.to_string_lossy().into_owned(),
            "-map".into(),
            "0:s:0".into(),
            extracted.to_string_lossy().into_owned(),
        ],
        cancel,
    )
    .await
    .unwrap();
    let segments = crate::captions::parse(&fs::read_to_string(extracted).unwrap()).unwrap();
    assert_eq!(segments[0].text, "First clipped phrase");
    assert_eq!(segments[0].start, 0.0);
    assert_eq!(segments[0].end, 0.5);
    assert_eq!(segments[1].start, 2.5);
    assert_eq!(segments[1].end, 3.0);
}
