use super::*;

pub async fn request(app: AppHandle, state: Arc<AppState>, request: Envelope) -> Response {
    match handle(app, state, request).await {
        Ok(value) => Response::success(value),
        Err(error) => Response {
            version: 1,
            ok: false,
            result: None,
            error: Some(error),
        },
    }
}
async fn handle(
    app: AppHandle,
    state: Arc<AppState>,
    request: Envelope,
) -> Result<Value, ApiError> {
    if request.version != 1 {
        return Err(invalid("Unsupported API version."));
    }
    if request.method == "connect" {
        if state.headless {
            return Err(api(
                "approval_required",
                "Open nabnab Settings to approve a new client.",
            ));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Connect {
            name: String,
        }
        let input: Connect = params(request.params)?;
        let name = input.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(invalid("Choose a client name of 1–80 characters."));
        }
        let result = change(&state, |repo| {
            if !repo.data.automation.enabled {
                return Err("Enable automation in Settings first.".into());
            }
            repo.data
                .automation
                .clients
                .retain(|c| c.state != ClientState::Revoked);
            if repo.data.automation.clients.len() >= MAX_CLIENTS {
                return Err("The local client limit was reached.".into());
            }
            let id = credentials::random_id()?;
            credentials::create(repo.root(), &id)?;
            repo.data.automation.clients.push(Client {
                id,
                name,
                state: ClientState::Pending,
                created_at: queue::now(),
                grant: Grant::default(),
            });
            Ok(())
        })
        .map_err(|e| api("authorization_required", &e))?;
        let client = result
            .clients
            .last()
            .ok_or_else(|| api("internal_error", "The client could not be registered."))?;
        updated(&app);
        return Ok(
            json!({"clientId":client.id,"state":"pending","message":"Authorize this client in nabnab Settings → Automation before using it."}),
        );
    }
    if !CAPABILITIES.contains(&request.method.as_str()) && request.method != "client.status" {
        return Err(api("operation_not_allowed","This operation is not exposed. Capture, shell, credentials and arbitrary paths are unavailable."));
    }
    let client = {
        let guard = state.repository.lock().unwrap();
        let repo = guard
            .as_ref()
            .ok_or_else(|| api("owner_unavailable", "Open nabnab first."))?;
        if !repo.data.automation.enabled {
            return Err(api("disabled", "Automation is disabled."));
        }
        let id = request.client_id.as_deref().ok_or_else(|| {
            api(
                "authorization_required",
                "Connect and authorize a local client first.",
            )
        })?;
        let secret = request
            .secret
            .as_deref()
            .ok_or_else(|| api("authorization_required", "The local credential is missing."))?;
        let expected = credentials::load(repo.root(), id).map_err(|_| {
            api(
                "authorization_required",
                "The local client credential is unavailable.",
            )
        })?;
        if !credentials::constant_time_eq(secret, &expected) {
            return Err(api(
                "authorization_required",
                "The local client credential is invalid.",
            ));
        }
        let client = repo
            .data
            .automation
            .clients
            .iter()
            .find(|c| c.id == id && c.state == ClientState::Authorized)
            .cloned()
            .ok_or_else(|| {
                api(
                    "authorization_required",
                    "Approve this client in nabnab Settings → Automation.",
                )
            })?;
        if request.method != "client.status"
            && !client
                .grant
                .capabilities
                .iter()
                .any(|c| c == &request.method)
        {
            return Err(api(
                "permission_denied",
                "This capability has not been granted to this client.",
            ));
        }
        client
    };
    match request.method.as_str() {
        "client.status" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Empty {}
            let _: Empty = params(request.params)?;
            let guard = state.repository.lock().unwrap();
            let repo = guard
                .as_ref()
                .ok_or_else(|| api("owner_unavailable", "Open nabnab first."))?;
            let current = repo
                .data
                .automation
                .clients
                .iter()
                .find(|c| {
                    c.id == client.id
                        && c.state == ClientState::Authorized
                        && repo.data.automation.enabled
                })
                .ok_or_else(|| {
                    api(
                        "authorization_required",
                        "The local client authorization changed.",
                    )
                })?;
            Ok(
                json!({"clientId":current.id,"grants":{"capabilities":current.grant.capabilities,"collections":current.grant.collections,"includeDocuments":current.grant.include_documents,"outputRoots":repo.data.automation.roots.iter().filter(|r|current.grant.output_root_ids.contains(&r.id)).map(|r|json!({"id":r.id,"label":r.label})).collect::<Vec<_>>(),"maxBytes":current.grant.max_bytes,"maxSeconds":current.grant.max_seconds}}),
            )
        }
        "media.inspect" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Inspect {
                url: String,
            }
            let input: Inspect = params(request.params)?;
            let url = crate::model::validate_url(&input.url)
                .map_err(|_| invalid("Choose a public HTTP(S) URL."))?;
            let (_sender, cancel) = watch::channel(false);
            let operation = async {
                let _permit = crate::batch::metadata_permit(&url).await?;
                let url = crate::providers::validate_remote(&url).await?;
                crate::direct::inspect(&url, 0, cancel.clone()).await
            };
            let media = tokio::time::timeout(std::time::Duration::from_secs(60), operation)
                .await
                .map_err(|_| api("timeout", "Metadata inspection timed out."))?
                .map_err(|_| {
                    api(
                        "inspection_failed",
                        "Choose a direct public media file. Web pages, HLS/DASH playlists and extractor fallbacks are unavailable to automation.",
                    )
                })?;
            // Recheck revocation after the network operation, before returning private results.
            authorize_current(&state, &client.id, "media.inspect")?;
            Ok(
                json!({"title":media.title,"duration":media.duration,"hasVideo":media.has_video,"hasAudio":media.has_audio,"heights":media.heights}),
            )
        }
        "library.search" | "library.get_item" => {
            let guard = state.repository.lock().unwrap();
            let repo = guard
                .as_ref()
                .ok_or_else(|| api("owner_unavailable", "Open nabnab first."))?;
            let client = current_client(repo, &client.id, &request.method)?;
            if request.method == "library.get_item" {
                let input: ItemId = params(request.params)?;
                let item = repo
                    .data
                    .library
                    .iter()
                    .find(|i| i.id == input.item_id && allowed_item(&client, i))
                    .ok_or_else(|| api("not_found", "No authorized library item has this ID."))?;
                Ok(item_view(item))
            } else {
                let input: Search = params(request.params)?;
                if input.query.chars().count() > 200
                    || input.limit.is_some_and(|n| n == 0 || n > 100)
                {
                    return Err(invalid(
                        "Search permits up to 200 characters and 1–100 results.",
                    ));
                }
                let query = input.query.to_lowercase();
                let limit = input.limit.unwrap_or(20) as usize;
                let items: Vec<_> = repo
                    .data
                    .library
                    .iter()
                    .rev()
                    .filter(|i| allowed_item(&client, i) && i.name.to_lowercase().contains(&query))
                    .take(limit + 1)
                    .collect();
                Ok(
                    json!({"items":items.iter().take(limit).map(|i|item_view(i)).collect::<Vec<_>>(),"hasMore":items.len()>limit}),
                )
            }
        }
        "jobs.status" => {
            let input: JobQuery = params(request.params)?;
            let guard = state.repository.lock().unwrap();
            let repo = guard
                .as_ref()
                .ok_or_else(|| api("owner_unavailable", "Open nabnab first."))?;
            let client = current_client(repo, &client.id, &request.method)?;
            let jobs: Vec<_> = repo
                .data
                .jobs
                .iter()
                .rev()
                .filter(|j| {
                    j.automation
                        .as_ref()
                        .is_some_and(|a| a.client_id == client.id)
                        && input.job_id.as_ref().is_none_or(|id| &j.id == id)
                })
                .take(100)
                .map(job_view)
                .collect();
            if input.job_id.is_some() && jobs.is_empty() {
                return Err(api(
                    "not_found",
                    "No job belonging to this client has this ID.",
                ));
            }
            Ok(
                json!({"jobs":jobs,"grants":{"capabilities":client.grant.capabilities,"collections":client.grant.collections,"includeDocuments":client.grant.include_documents,"outputRoots":repo.data.automation.roots.iter().filter(|r|client.grant.output_root_ids.contains(&r.id)).map(|r|json!({"id":r.id,"label":r.label})).collect::<Vec<_>>(),"maxBytes":client.grant.max_bytes,"maxSeconds":client.grant.max_seconds}}),
            )
        }
        "jobs.cancel" => {
            let input: JobId = params(request.params)?;
            queue::automation_cancel(&state, &input.job_id, &client.id).map_err(|_| {
                api(
                    "permission_denied",
                    "This client cannot cancel this job with its current permissions.",
                )
            })?;
            state.runtime.notify.notify_one();
            Ok(json!({"jobId":input.job_id,"cancelRequested":true}))
        }
        "jobs.plan" | "tools.run" => {
            if state.headless {
                return Err(api(
                    "approval_required",
                    "Prepare and approve the exact plan in nabnab before using the worker.",
                ));
            }
            let input: PlanInput = params(request.params)?;
            if request.method == "tools.run"
                && matches!(input.operation, Operation::Download { .. })
            {
                return Err(api(
                    "operation_not_allowed",
                    "tools.run prepares only extractAudio or frame.",
                ));
            }
            let mut plan = prepare(&state, &client, input).map_err(|e| api("plan_rejected", &e))?;
            if let queue::Source::Download { url, .. } = &plan.source {
                crate::providers::validate_remote(url)
                    .await
                    .map_err(|_| api("invalid_source", "Choose an accessible public source."))?;
            }
            if let queue::Source::Transform { library_id, .. } = &plan.source {
                let path = {
                    let guard = state.repository.lock().unwrap();
                    guard
                        .as_ref()
                        .and_then(|r| r.data.library.iter().find(|i| &i.id == library_id))
                        .map(|i| i.path.clone())
                        .ok_or_else(|| api("not_found", "The selected input is unavailable."))?
                };
                plan.input_hash = Some(
                    tokio::task::spawn_blocking(move || file_hash(&path))
                        .await
                        .map_err(|_| api("plan_rejected", "Input verification failed."))?
                        .map_err(|e| api("plan_rejected", &e))?,
                );
            }
            let mut guard = state.repository.lock().unwrap();
            let repo = guard
                .as_mut()
                .ok_or_else(|| api("owner_unavailable", "Open nabnab first."))?;
            let plan = persist_plan(repo, plan).map_err(|e| api("plan_rejected", &e))?;
            drop(guard);
            updated(&app);
            Ok(
                json!({"planId":plan.id,"state":"pending","approvalRequired":true,"expiresAt":plan.expires_at,"digest":plan.digest,"operation":plan.operation,"budget":plan.budget,"collection":plan.collection,"outputRootId":plan.output_root_id}),
            )
        }
        "jobs.enqueue" => {
            let input: Id = params(request.params)?;
            let value = enqueue(&state, &client.id, &input.plan_id)
                .map_err(|e| api("approval_required", &e))?;
            updated(&app);
            state.runtime.notify.notify_one();
            Ok(value)
        }
        _ => Err(api(
            "operation_not_allowed",
            "This operation is unavailable.",
        )),
    }
}
