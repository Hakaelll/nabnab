#[path = "../automation_cli/client.rs"]
mod automation_client;
#[path = "../automation_credentials.rs"]
mod automation_credentials;
#[path = "../automation_wire.rs"]
mod automation_wire;
#[path = "../automation_cli/mcp.rs"]
mod mcp;
use automation_wire::{ApiError, Envelope, Response, VERSION};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const USAGE:&str="nabnab-cli connect --name NAME [--json]\nnabnab-cli permissions --client ID [--json]\nnabnab-cli inspect --url URL --client ID [--json]\nnabnab-cli plan|tools --params-json JSON|--params-file FILE --client ID [--json]\nnabnab-cli enqueue --plan ID --client ID [--json]\nnabnab-cli jobs|status [--job ID] --client ID [--json]\nnabnab-cli cancel --job ID --client ID [--json]\nnabnab-cli search [--query TEXT] [--limit N] --client ID [--json]\nnabnab-cli get-item --item ID --client ID [--json]\nnabnab-cli mcp --client ID\nnabnab-cli worker\nKeep the desktop or an explicit worker open. Automation must be enabled. Connect and every plan require local desktop approval. Secrets never appear in command arguments or output.";
fn invalid(message: &str) -> ApiError {
    ApiError::new("invalid_arguments", message, false)
}
#[cfg(windows)]
fn run_worker() -> Result<(), ApiError> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let failure = || {
        ApiError::new(
        "owner_unavailable",
        "Could not run the local worker. Close the desktop owner and enable automation in nabnab first.",
        false,
    )
    };
    let binary = std::env::current_exe()
        .map_err(|_| failure())?
        .with_file_name("nabnab.exe");
    let mut worker = Command::new(binary)
        .arg("--automation-worker")
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|_| failure())?;
    let lease = worker.stdin.take();
    let status = worker.wait().map_err(|_| failure())?;
    drop(lease);
    if status.success() {
        Ok(())
    } else {
        Err(failure())
    }
}
#[cfg(not(windows))]
fn run_worker() -> Result<(), ApiError> {
    Err(ApiError::new(
        "unsupported_platform",
        "Local automation currently requires Windows.",
        false,
    ))
}
fn arguments(args: Vec<String>) -> Result<(String, BTreeMap<String, String>, bool), ApiError> {
    let mut args = args.into_iter();
    let command = args.next().ok_or_else(|| invalid(USAGE))?;
    let mut values = BTreeMap::new();
    let mut json = false;
    while let Some(flag) = args.next() {
        if flag == "--json" {
            if json {
                return Err(invalid("Do not repeat --json."));
            }
            json = true;
            continue;
        }
        if !flag.starts_with("--") {
            return Err(invalid("Use named command options; see --help."));
        }
        let value = args
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| invalid("Each option requires a value."))?;
        if values.insert(flag, value).is_some() {
            return Err(invalid("Do not repeat command options."));
        }
    }
    Ok((command, values, json))
}
fn take(values: &mut BTreeMap<String, String>, name: &str) -> Result<String, ApiError> {
    values
        .remove(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(&format!("Missing required option {name}.")))
}
fn request(
    command: &str,
    values: &mut BTreeMap<String, String>,
) -> Result<(String, Value), ApiError> {
    Ok(match command {
        "connect" => ("connect".into(), json!({"name":take(values,"--name")?})),
        "inspect" => ("media.inspect".into(), json!({"url":take(values,"--url")?})),
        "permissions" => ("client.status".into(), json!({})),
        "plan" | "tools" => {
            let inline = values.remove("--params-json");
            let file = values.remove("--params-file");
            let text = match (inline, file) {
                (Some(text), None) => text,
                (None, Some(file)) => {
                    let metadata = std::fs::metadata(&file)
                        .map_err(|_| invalid("The parameter file is unavailable."))?;
                    if !metadata.is_file() || metadata.len() > automation_wire::MAX_REQUEST as u64 {
                        return Err(invalid("Use a regular UTF-8 parameter file up to256KiB."));
                    }
                    std::fs::read_to_string(file)
                        .map_err(|_| invalid("Use a readable UTF-8 parameter file."))?
                }
                _ => {
                    return Err(invalid(
                        "Choose exactly one of --params-json or --params-file.",
                    ))
                }
            };
            if text.len() > automation_wire::MAX_REQUEST {
                return Err(invalid("Parameters exceed256KiB."));
            }
            let params: Value = serde_json::from_str(&text)
                .map_err(|_| invalid("Use a valid JSON parameter object."))?;
            if !params.is_object() {
                return Err(invalid("Use a JSON object for operation parameters."));
            }
            (
                (if command == "plan" {
                    "jobs.plan"
                } else {
                    "tools.run"
                })
                .into(),
                params,
            )
        }
        "enqueue" => (
            "jobs.enqueue".into(),
            json!({"planId":take(values,"--plan")?}),
        ),
        "jobs" | "status" => {
            let mut params = json!({});
            if let Some(id) = values.remove("--job") {
                params["jobId"] = json!(id);
            }
            ("jobs.status".into(), params)
        }
        "cancel" => ("jobs.cancel".into(), json!({"jobId":take(values,"--job")?})),
        "search" => {
            let mut params = json!({});
            if let Some(query) = values.remove("--query") {
                params["query"] = json!(query);
            }
            if let Some(limit) = values.remove("--limit") {
                let limit = limit
                    .parse::<u16>()
                    .ok()
                    .filter(|limit| (1..=100).contains(limit))
                    .ok_or_else(|| invalid("Use a search limit from 1 to 100."))?;
                params["limit"] = json!(limit);
            }
            ("library.search".into(), params)
        }
        "get-item" => (
            "library.get_item".into(),
            json!({"itemId":take(values,"--item")?}),
        ),
        _ => return Err(invalid("Unknown command. Run nabnab-cli --help.")),
    })
}
fn exit_code(error: &ApiError) -> u8 {
    match error.code.as_str() {
        "invalid_arguments" | "invalid_request" | "unsupported_version" => 2,
        "owner_not_running" | "owner_unavailable" | "disabled" | "unsupported_platform" => 3,
        "unauthorized"
        | "permission_denied"
        | "client_pending"
        | "client_revoked"
        | "credentials_unavailable"
        | "authorization_required" => 4,
        "approval_required" | "plan_not_approved" => 5,
        _ => 1,
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "help" | "-h") {
        println!("{USAGE}");
        return std::process::ExitCode::SUCCESS;
    }
    if args.len() == 1 && args[0] == "--version" {
        println!("nabnab-cli wire1 / rmcp3.9.0");
        return std::process::ExitCode::SUCCESS;
    }
    let is_mcp = args.first().is_some_and(|arg| arg == "mcp");
    let compact = args.iter().any(|arg| arg == "--json");
    let result = async {
        let (command, mut values, _) = arguments(args)?;
        if command == "worker" {
            if !values.is_empty() {
                return Err(invalid("Unknown worker option."));
            }
            run_worker()?;
            return Ok(None);
        }
        let local_root = || {
            automation_credentials::data_root().map_err(|_| {
                ApiError::new(
                    "owner_not_running",
                    "Open the nabnab desktop application and enable local automation.",
                    true,
                )
            })
        };
        if command == "mcp" {
            let id = take(&mut values, "--client")?;
            if !values.is_empty() {
                return Err(invalid("Unknown MCP option."));
            }
            mcp::serve(&local_root()?, &id).await?;
            return Ok(None);
        }
        let client = if command == "connect" {
            None
        } else {
            Some(take(&mut values, "--client")?)
        };
        let (method, params) = request(&command, &mut values)?;
        if !values.is_empty() {
            return Err(invalid("Unknown option for this command."));
        }
        let root = local_root()?;
        let secret = client
            .as_deref()
            .map(|id| {
                automation_credentials::load(&root, id).map_err(|_| {
                    ApiError::new(
                        "credentials_unavailable",
                        "Connect this client and approve it in the desktop first.",
                        false,
                    )
                })
            })
            .transpose()?;
        let response = automation_client::call(
            &root,
            &Envelope {
                version: VERSION,
                client_id: client,
                secret,
                method,
                params,
            },
        )
        .await?;
        Ok::<_, ApiError>(Some(response))
    }
    .await;
    let response = match result {
        Ok(None) => return std::process::ExitCode::SUCCESS,
        Ok(Some(response)) => response,
        Err(error) => Response::failure(error.code, error.message, error.retryable),
    };
    let code = response.error.as_ref().map_or(0, exit_code);
    if is_mcp {
        if let Some(error) = response.error {
            eprintln!("{error}");
        }
    } else {
        println!("{}",if compact{serde_json::to_string(&response)}else{serde_json::to_string_pretty(&response)}.unwrap_or_else(|_|"{\"version\":1,\"ok\":false,\"error\":{\"code\":\"serialization_error\",\"message\":\"Could not encode response.\",\"retryable\":false}}".into()));
        if let Some(error) = response.error {
            eprintln!("{error}");
        }
    }
    std::process::ExitCode::from(code)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_has_closed_options_and_never_accepts_secret_arguments() {
        assert!(arguments(vec!["status".into(), "--json".into(), "--json".into()]).is_err());
        let (_, mut options, _) = arguments(vec![
            "enqueue".into(),
            "--plan".into(),
            "approved".into(),
            "--secret".into(),
            "hidden".into(),
        ])
        .unwrap();
        let (method, params) = request("enqueue", &mut options).unwrap();
        assert_eq!(method, "jobs.enqueue");
        assert_eq!(params, json!({"planId":"approved"}));
        assert!(!options.is_empty());
        assert!(request("shell", &mut BTreeMap::new()).is_err());
    }
    #[test]
    fn literal_json_parameters_remain_data() {
        let mut values=BTreeMap::from([("--params-json".into(),r#"{"operation":{"kind":"frame","libraryId":"opaque","at":1,"width":640},"outputRootId":"out","collection":"Review","budget":{"maxBytes":1000000,"maxSeconds":20}}"#.into())]);
        let (method, params) = request("tools", &mut values).unwrap();
        assert_eq!(method, "tools.run");
        assert_eq!(params["operation"]["libraryId"], "opaque");
        assert!(values.is_empty());
    }
}
