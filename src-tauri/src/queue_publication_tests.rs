use super::*;

#[test]
fn selected_files_create_versions_without_replacing_existing_files_or_links() {
    let root=std::env::temp_dir().join(format!("nabnab-selected-versions-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let staging=root.join("stage");let destination=root.join("files");
    fs::create_dir_all(&staging).unwrap();fs::create_dir_all(&destination).unwrap();
    let input=staging.join("report.pdf");fs::write(&input,b"new report").unwrap();
    fs::write(destination.join("report.pdf"),b"original report").unwrap();
    fs::write(destination.join("report (2).pdf"),b"another report").unwrap();
    let (file,target)=available_file_version(&staging,&destination,&input).unwrap();
    assert_eq!(target.file_name().unwrap(),"report (3).pdf");
    assert_eq!(fs::read(&file).unwrap(),b"new report");
    assert_eq!(file.file_name(),target.file_name());
    let published=crate::publish(&staging,&file.to_string_lossy(),&destination).unwrap();
    assert_eq!(fs::read(published).unwrap(),b"new report");
    assert_eq!(fs::read(destination.join("report.pdf")).unwrap(),b"original report");
    assert_eq!(fs::read(destination.join("report (2).pdf")).unwrap(),b"another report");
    assert_eq!(fs::read(input).unwrap(),b"new report");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cancellation_interrupts_a_wait_for_output_publication() {
    let state = Arc::new(AppState::default());
    let (sender, cancel) = watch::channel(false);
    let held = publication_lock(&state, cancel.clone()).await.unwrap();
    let shared = state.clone();
    let waiting = tokio::spawn(async move { publication_lock(&shared, cancel).await });
    sender.send(true).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.unwrap_err(), engine::CANCELLED);
    drop(held);
}

#[tokio::test]
async fn concurrent_identical_results_wait_for_indexing_and_publish_one_file() {
    let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
    let root = parent.join(format!(
        "nabnab-publication-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let destination = root.join("media");
    fs::create_dir_all(&destination).unwrap();
    let mut repo = Repository::open(&root, &destination).unwrap();
    let mut jobs = Vec::new();
    for host in ["example.org", "example.net"] {
        let mut job = Job::new(
            Source::Download {
                url: format!("https://{host}/media.mp4"),
                url_ref: None,
                format: Format::Mp4,
                height: None,
                audio_bitrate: None,
                subtitle_language: None,
            },
            host.into(),
            destination.clone(),
            host.into(),
            None,
        );
        job.protect_source(repo.root()).unwrap();
        job.state = JobState::Verifying;
        job.run_id = 1;
        let staging = stage(&job).unwrap();
        let file = staging.join(format!("{}.mp4", job.id));
        fs::write(&file, b"verified identical output").unwrap();
        let item = library::index(&file, None, None, None).unwrap();
        repo.data.jobs.push(job.clone());
        jobs.push((job, staging, file, item));
    }
    repo.save().unwrap();
    let state = Arc::new(AppState {
        repository: Mutex::new(Some(repo)),
        ..Default::default()
    });
    let (_sender, cancel) = watch::channel(false);
    let (first_job, first_stage, first_file, first_item) = jobs.remove(0);
    let (first_path, first_item, publication) = publish_verified_output(
        &state,
        &first_job,
        &first_stage,
        &first_file,
        first_item,
        cancel.clone(),
    )
    .await
    .unwrap();
    let (second_job, second_stage, second_file, second_item) = jobs.remove(0);
    let shared = state.clone();
    let second_id = second_job.id.clone();
    let mut second = tokio::spawn(async move {
        publish_verified_output(
            &shared,
            &second_job,
            &second_stage,
            &second_file,
            second_item,
            cancel,
        )
        .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut second)
            .await
            .is_err()
    );
    {
        let mut guard = state.repository.lock().unwrap();
        let repo = guard.as_mut().unwrap();
        let first = repo
            .data
            .jobs
            .iter_mut()
            .find(|job| job.id == first_job.id)
            .unwrap();
        first.state = JobState::Completed;
        first.outputs = vec![first_path.clone()];
        repo.data.library.extend(first_item);
        repo.save().unwrap();
    }
    drop(publication);
    let (second_path, second_item, publication) = second.await.unwrap().unwrap();
    assert_eq!(second_path, first_path);
    assert_eq!(second_item.len(), 1);
    assert_eq!(second_item[0].path, first_path);
    assert_eq!(fs::read(&first_path).unwrap(), b"verified identical output");
    let files = fs::read_dir(&destination)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().unwrap().is_file())
        .count();
    assert_eq!(files, 1);
    assert!(state
        .repository
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .data
        .jobs
        .iter()
        .any(|job| job.id == second_id && job.outputs.is_empty()));
    drop(publication);
    drop(state);
    let resolved = fs::canonicalize(&root).unwrap();
    assert_eq!(resolved.parent(), Some(parent.as_path()));
    assert!(resolved
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("nabnab-publication-"));
    fs::remove_dir_all(resolved).unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn a_published_stem_set_recovers_together_and_detects_a_changed_track() {
    let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
    let root = parent.join(format!("nabnab-stem-set-{}-{}", std::process::id(), now()));
    let destination = root.join("media");
    fs::create_dir_all(&destination).unwrap();
    let mut repo = Repository::open(&root, &destination).unwrap();
    let library_id = "a".repeat(64);
    let mut job = Job::new(
        Source::Stems { selection: crate::stems::Selection { library_id: library_id.clone(), input_hash: "b".repeat(64) } },
        "Stems".into(), destination, "stems-test".into(), None,
    );
    job.state = JobState::Verifying;
    job.run_id = 1;
    let staging = stage(&job).unwrap();
    let directory = staging.join("Stems");
    fs::create_dir(&directory).unwrap();
    let items = crate::stems::STEMS.map(|role| {
        let file = directory.join(format!("{role}.flac"));
        fs::write(&file, role.as_bytes()).unwrap();
        let mut item = library::index(&file, None, None, Some(library_id.clone())).unwrap();
        item.details.stems = Some(crate::stems::Reference {
            set_id: job.id.clone(), role: role.into(), model: crate::stems::component::VERSION.into(),
        });
        item
    });
    repo.data.jobs.push(job.clone());
    let state = AppState { repository: Mutex::new(Some(repo)), ..Default::default() };
    let (_sender, cancel) = watch::channel(false);
    let (published, items, lock) = publish_stems(&state, &job, &staging, &directory, items.to_vec(), cancel.clone()).await.unwrap();
    drop(lock);
    assert!(!directory.exists());
    assert_eq!(fs::read_dir(&published).unwrap().count(), 2);
    let (recovered, tracks, lock) = recover_publication(&state, &job, &staging, cancel.clone()).await.unwrap().unwrap();
    assert_eq!(recovered, published);
    assert_eq!(tracks.iter().map(|item| &item.path).collect::<Vec<_>>(), items.iter().map(|item| &item.path).collect::<Vec<_>>());
    drop(lock);
    fs::write(&tracks[0].path, b"Changed").unwrap();
    let recovery = recover_publication(&state, &job, &staging, cancel).await;
    assert!(matches!(recovery, Err(error) if error.contains("modified")));
    assert_eq!(fs::read(&tracks[0].path).unwrap(), b"Changed");
    drop(state);
    fs::remove_dir_all(root).unwrap();
}
