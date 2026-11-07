use super::*;

fn fixture() -> (PathBuf, Repository, String, String, String) {
    let root = std::env::temp_dir().join(format!(
        "nabnab-automation-test-{}",
        credentials::random_id().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    let output = root.join("out");
    fs::create_dir(&output).unwrap();
    let output = library::authorized_path(&output).unwrap();
    let mut repo = Repository::open(&root, &output).unwrap();
    repo.data.automation.enabled = true;
    repo.data.collections = vec!["Shared".into(), "Private".into()];
    let path = root.join("sample.wav");
    fs::write(&path, b"unchanged source fixture").unwrap();
    let mut item = library::index(&path, None, None, None).unwrap();
    item.collections = vec!["Shared".into()];
    let item_id = item.id.clone();
    repo.data.library.push(item);
    let client_id = credentials::random_id().unwrap();
    let root_id = credentials::random_id().unwrap();
    repo.data.automation.roots.push(OutputRoot {
        id: root_id.clone(),
        label: "out".into(),
        path: output,
    });
    repo.data.automation.clients.push(Client {
        id: client_id.clone(),
        name: "Test client".into(),
        state: ClientState::Authorized,
        created_at: queue::now(),
        grant: Grant {
            capabilities: CAPABILITIES.iter().map(|v| v.to_string()).collect(),
            collections: vec!["Shared".into()],
            output_root_ids: vec![root_id.clone()],
            ..Grant::default()
        },
    });
    repo.save().unwrap();
    (root, repo, client_id, root_id, item_id)
}
pub(crate) fn state_with_plan() -> (PathBuf, Arc<AppState>, String, String) {
    let (root, repo, client_id, root_id, item_id) = fixture();
    let state = Arc::new(AppState::default());
    *state.repository.lock().unwrap() = Some(repo);
    let client = state
        .repository
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .data
        .automation
        .clients[0]
        .clone();
    let mut plan = prepare(
        &state,
        &client,
        PlanInput {
            operation: Operation::ExtractAudio {
                library_id: item_id,
                format: "mp3".into(),
            },
            output_root_id: root_id,
            collection: "Shared".into(),
            budget: Budget {
                max_bytes: 8 * MIN_BYTES,
                max_seconds: 30,
            },
        },
    )
    .unwrap();
    let path = state
        .repository
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .data
        .library[0]
        .path
        .clone();
    plan.input_hash = Some(file_hash(&path).unwrap());
    plan.digest = plan_digest(&plan).unwrap();
    let id = plan.id.clone();
    state
        .repository
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .data
        .automation
        .plans
        .push(plan);
    (root, state, client_id, id)
}
pub(crate) fn active_job() -> (PathBuf, Arc<AppState>, queue::Job) {
    let (root, state, client, id) = state_with_plan();
    state
        .repository
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .data
        .automation
        .plans[0]
        .state = PlanState::Approved;
    enqueue(&state, &client, &id).unwrap();
    let job = {
        let mut guard = state.repository.lock().unwrap();
        let job = &mut guard.as_mut().unwrap().data.jobs[0];
        job.state = queue::JobState::Processing;
        job.run_id = 1;
        job.clone()
    };
    (root, state, job)
}
fn cleanup(root: PathBuf, state: Arc<AppState>) {
    state.repository.lock().unwrap().take();
    drop(state);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn default_is_off_and_all_capabilities_are_explicit() {
    let data = Data::default();
    assert!(!data.enabled);
    assert!(Grant::default().capabilities.is_empty());
    assert!(!Grant::default().include_documents);
    assert!(validate(&data).is_ok());
}
#[test]
fn schemas_reject_capture_shell_paths_and_overrides() {
    for operation in [
        json!({"kind":"capture","source":"desktop"}),
        json!({"kind":"shell","args":["cmd"]}),
        json!({"kind":"extractAudio","path":"C:/private.wav","format":"mp3"}),
        json!({"kind":"extractAudio","libraryId":"id","format":"mp3","args":["-i","https://remote"]}),
    ] {
        assert!(serde_json::from_value::<PlanInput>(json!({"operation":operation,"outputRootId":"root","collection":"Shared","budget":{"maxBytes":MIN_BYTES,"maxSeconds":5}})).is_err());
    }
}
#[test]
fn collections_are_opt_in_and_documents_are_excluded_by_default() {
    let (root, repo, _, _, _) = fixture();
    let client = &repo.data.automation.clients[0];
    let mut item = repo.data.library[0].clone();
    assert!(allowed_item(client, &item));
    item.collections = vec!["Private".into()];
    assert!(!allowed_item(client, &item));
    item.collections = vec!["Shared".into()];
    item.kind = "document".into();
    assert!(!allowed_item(client, &item));
    let mut grant = client.clone();
    grant.grant.include_documents = true;
    assert!(allowed_item(&grant, &item));
    grant.grant.collections.clear();
    assert!(!allowed_item(&grant, &item));
    drop(repo);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn approval_is_exact_single_use_and_bound_to_client() {
    let (root, state, client, id) = state_with_plan();
    assert!(enqueue(&state, &client, &id)
        .unwrap_err()
        .contains("approve"));
    {
        let mut lock = state.repository.lock().unwrap();
        let repo = lock.as_mut().unwrap();
        repo.data.automation.plans[0].state = PlanState::Approved;
        assert!(validate(&repo.data.automation).is_ok());
        let encoded = serde_json::to_vec(&repo.data.automation).unwrap();
        assert!(serde_json::from_slice::<Data>(&encoded).is_ok());
    }
    assert!(enqueue(&state, &credentials::random_id().unwrap(), &id).is_err());
    let result = enqueue(&state, &client, &id).unwrap();
    assert!(result["jobId"].as_str().is_some());
    assert!(enqueue(&state, &client, &id).is_err());
    {
        let lock = state.repository.lock().unwrap();
        let repo = lock.as_ref().unwrap();
        assert_eq!(repo.data.jobs.len(), 1);
        let job = &repo.data.jobs[0];
        assert!(job_allowed(repo, job).is_ok());
        let mut modified = job.clone();
        modified.folder = root.clone();
        assert!(job_allowed(repo, &modified).is_err());
        modified = job.clone();
        modified.source = queue::Source::Transform {
            library_id: "another".into(),
            options: ToolOptions::Mute,
        };
        assert!(job_allowed(repo, &modified).is_err());
    }
    assert!(queue::action(&state, &[result["jobId"].as_str().unwrap().into()], "retry").is_err());
    cleanup(root, state);
}
#[test]
fn revocation_budget_expiry_and_input_changes_block_execution() {
    let (root, state, client, id) = state_with_plan();
    {
        let mut lock = state.repository.lock().unwrap();
        let repo = lock.as_mut().unwrap();
        let plan = &repo.data.automation.plans[0];
        assert!(grant_plan(repo, plan).is_ok());
        let mut changed = plan.clone();
        changed.budget.max_bytes += 1;
        assert!(grant_plan(repo, &changed).is_err());
        changed = plan.clone();
        changed.expires_at = queue::now() - 1;
        changed.digest = plan_digest(&changed).unwrap();
        assert!(grant_plan(repo, &changed).is_err());
        repo.data.automation.plans[0].state = PlanState::Approved;
    }
    enqueue(&state, &client, &id).unwrap();
    let job = state.repository.lock().unwrap().as_ref().unwrap().data.jobs[0].clone();
    assert!(verify_input(&state, &job).is_ok());
    let path = state
        .repository
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .data
        .library[0]
        .path
        .clone();
    fs::write(path, b"changed input").unwrap();
    assert!(verify_input(&state, &job).unwrap_err().contains("changed"));
    let stage = root.join("stage");
    fs::create_dir(&stage).unwrap();
    assert!(
        runtime_guard(&state, &job, &stage, std::time::Duration::from_secs(31))
            .unwrap_err()
            .contains("runtime budget")
    );
    fs::write(
        stage.join("oversized.bin"),
        vec![0; 8 * MIN_BYTES as usize + 1],
    )
    .unwrap();
    assert!(
        runtime_guard(&state, &job, &stage, std::time::Duration::ZERO)
            .unwrap_err()
            .contains("disk budget")
    );
    {
        let mut lock = state.repository.lock().unwrap();
        let repo = lock.as_mut().unwrap();
        assert!(output_allowed(repo, &job, 8 * MIN_BYTES + 1).is_err());
        repo.data.automation.clients[0].state = ClientState::Revoked;
        assert!(job_allowed(repo, &job).is_err());
    }
    cleanup(root, state);
}
#[test]
fn public_responses_omit_paths_urls_secrets_and_text() {
    let (root, repo, _, _, _) = fixture();
    let mut item = repo.data.library[0].clone();
    item.source = Some("https://example.com/private?token=secret".into());
    let value = item_view(&item);
    assert_eq!(value["name"], "sample.wav");
    let encoded = value.to_string();
    for forbidden in ["source", "path", "token", "text", "collections"] {
        assert!(!encoded.contains(forbidden));
    }
    drop(repo);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejected_and_unsaved_plans_do_not_leave_protected_urls() {
    let (root, state, client_id, _) = state_with_plan();
    let client = state
        .repository
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .data
        .automation
        .clients[0]
        .clone();
    let root_id = client.grant.output_root_ids[0].clone();
    let plan = prepare(
        &state,
        &client,
        PlanInput {
            operation: Operation::Download {
                url: "https://example.org/private/first.mp4?sig=do-not-store".into(),
                format: crate::model::Format::Mp4,
            },
            output_root_id: root_id,
            collection: "Shared".into(),
            budget: Budget {
                max_bytes: 8 * MIN_BYTES,
                max_seconds: 30,
            },
        },
    )
    .unwrap();
    {
        let mut guard = state.repository.lock().unwrap();
        let repo = guard.as_mut().unwrap();
        repo.data.automation.clients[0]
            .grant
            .capabilities
            .retain(|c| c != "jobs.enqueue");
        for _ in 0..5 {
            assert!(persist_plan(repo, plan.clone()).is_err());
        }
        assert!(!root.join("job-urls").exists());
        repo.data.automation.clients[0].grant = client.grant.clone();
        let schema = repo.data.schema_version;
        repo.data.schema_version = 999;
        assert!(persist_plan(repo, plan.clone()).is_err());
        repo.data.schema_version = schema;
        assert_eq!(fs::read_dir(root.join("job-urls")).unwrap().count(), 0);
        for _ in 1..MAX_PLANS {
            let mut filled = plan.clone();
            filled.id = credentials::random_id().unwrap();
            filled.digest = plan_digest(&filled).unwrap();
            repo.data.automation.plans.push(filled);
        }
        for _ in 0..5 {
            assert!(persist_plan(repo, plan.clone()).is_err());
        }
        assert_eq!(fs::read_dir(root.join("job-urls")).unwrap().count(), 0);
        assert_eq!(repo.data.automation.clients[0].id, client_id);
    }
    cleanup(root, state);
}
#[test]
fn source_review_distinguishes_paths_without_persisting_access_urls() {
    let first = source_identity("https://example.org/private/first.mp4?sig=secret").unwrap();
    let second = source_identity("https://example.org/private/second.mp4?sig=secret").unwrap();
    assert_ne!(first, second);
    assert!(first.contains("first.mp4"));
    assert!(second.contains("second.mp4"));
    for identity in [first, second] {
        assert!(!identity.contains("secret"));
        assert!(!identity.contains("sig="));
        assert!(!identity.contains("private"));
    }
    assert_ne!(
        source_identity("https://example.org/a/file.mp4").unwrap(),
        source_identity("https://example.org/b/file.mp4").unwrap()
    );
}
#[test]
fn cancellation_rechecks_revocation_in_the_same_repository_transaction() {
    let (root, state, client, id) = state_with_plan();
    state
        .repository
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .data
        .automation
        .plans[0]
        .state = PlanState::Approved;
    let receipt = enqueue(&state, &client, &id).unwrap();
    let job_id = receipt["jobId"].as_str().unwrap().to_owned();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut guard = state.repository.lock().unwrap();
    let worker_state = state.clone();
    let worker_barrier = barrier.clone();
    let worker_id = job_id.clone();
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        queue::automation_cancel(&worker_state, &worker_id, &client)
    });
    barrier.wait();
    let repo = guard.as_mut().unwrap();
    repo.data.automation.clients[0]
        .grant
        .capabilities
        .retain(|c| c != "jobs.cancel");
    repo.save().unwrap();
    drop(guard);
    assert!(worker.join().unwrap().is_err());
    assert!(matches!(
        state.repository.lock().unwrap().as_ref().unwrap().data.jobs[0].state,
        queue::JobState::Pending
    ));
    cleanup(root, state);
}
