use crate::{
    automation_credentials as credentials,
    automation_wire::{self as wire, ApiError, Envelope, Response},
    AppState,
};
use std::{path::Path, sync::Arc};
use tauri::AppHandle;
use tokio::sync::{watch, Semaphore};

pub struct Server {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    pub fn stop(self) {
        drop(self);
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.task.abort();
    }
}

#[cfg(windows)]
fn listener(
    name: &str,
    first: bool,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer, String> {
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    let descriptor = credentials::SecurityDescriptor::owner_only(false)?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.as_ptr(),
        bInheritHandle: 0,
    };
    let mut options = tokio::net::windows::named_pipe::ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true)
        .max_instances(8)
        .in_buffer_size(wire::MAX_REQUEST as u32)
        .out_buffer_size(65536);
    unsafe {
        options.create_with_security_attributes_raw(
            name,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
        )
    }
    .map_err(|_| {
        "Could not bind the private automation pipe. Another desktop owner may already be running."
            .into()
    })
}
#[cfg(windows)]
pub fn start(app: &AppHandle, state: &Arc<AppState>, root: &Path) -> Result<Server, String> {
    let name = credentials::pipe_name(root)?;
    let mut pipe = listener(&name, true)?;
    let app = app.clone();
    let state = Arc::downgrade(state);
    let (stop, mut stopping) = watch::channel(false);
    let permits = Arc::new(Semaphore::new(4));
    let task = tokio::spawn(async move {
        loop {
            let permit = tokio::select! {_ = stopping.changed()=>break,permit=permits.clone().acquire_owned()=>match permit{Ok(permit)=>permit,Err(_)=>break}};
            tokio::select! {_ = stopping.changed()=>break,result=pipe.connect()=>{if result.is_err(){break;}}}
            let next = match listener(&name, false) {
                Ok(next) => next,
                Err(_) => break,
            };
            let mut client = std::mem::replace(&mut pipe, next);
            let app = app.clone();
            let state = state.clone();
            let mut cancelled = stopping.clone();
            tokio::spawn(async move {
                let operation = async {
                    let _permit = permit;
                    let parsed = tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        wire::read_frame(&mut client, wire::MAX_REQUEST),
                    )
                    .await;
                    let request = match parsed {
                        Ok(Ok(bytes)) => serde_json::from_slice::<Envelope>(&bytes).map_err(|_| {
                            ApiError::new(
                                "invalid_request",
                                "Use a valid versioned local automation envelope.",
                                false,
                            )
                        }),
                        Ok(Err(error)) => Err(error),
                        Err(_) => Err(ApiError::new(
                            "request_timeout",
                            "The local request did not complete in time.",
                            true,
                        )),
                    };
                    let response=match request{
                    Ok(request)=>match request.validate(){Err(error)=>Response::failure(error.code,error.message,error.retryable),Ok(())=>match state.upgrade(){Some(state)=>match tokio::time::timeout(std::time::Duration::from_secs(120),crate::automation::request(app,state,request)).await{Ok(response)=>response,Err(_)=>Response::failure("request_timeout","The desktop operation timed out. Check its status before retrying a mutation.",true)},None=>Response::failure("owner_not_running","Open the nabnab desktop application to use local automation.",true)}},
                    Err(error)=>Response::failure(error.code,error.message,error.retryable)
                };
                    let bytes=serde_json::to_vec(&response).unwrap_or_else(|_|b"{\"version\":1,\"ok\":false,\"error\":{\"code\":\"serialization_error\",\"message\":\"The desktop response could not be encoded.\",\"retryable\":false}}".to_vec());
                    let bytes = if bytes.len() > wire::MAX_RESPONSE {
                        serde_json::to_vec(&Response::failure(
                            "response_too_large",
                            "Narrow the query; the desktop response exceeds 4 MiB.",
                            false,
                        ))
                        .unwrap_or_default()
                    } else {
                        bytes
                    };
                    if wire::write_response(&mut client, &bytes).await.is_ok() {
                        use tokio::io::AsyncReadExt;
                        let mut ack = [0u8; 1];
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            client.read_exact(&mut ack),
                        )
                        .await;
                    }
                };
                tokio::select! {_ = cancelled.changed()=>{},_ = operation=>{}}
            });
        }
    });
    Ok(Server { stop, task })
}
#[cfg(not(windows))]
pub fn start(_: &AppHandle, _: &Arc<AppState>, _: &Path) -> Result<Server, String> {
    Err("Local automation currently requires Windows named pipes.".into())
}
