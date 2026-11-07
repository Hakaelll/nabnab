use crate::{
    automation_credentials as credentials,
    automation_wire::{self as wire, ApiError, Envelope, Response},
};
use std::path::Path;

#[cfg(windows)]
pub async fn call(root: &Path, request: &Envelope) -> Result<Response, ApiError> {
    use tokio::io::AsyncWriteExt;
    request.validate()?;
    let name = credentials::pipe_name(root).map_err(|_| {
        ApiError::new(
            "owner_not_running",
            "Open the nabnab desktop application and enable local automation.",
            true,
        )
    })?;
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut pipe = loop {
        match tokio::net::windows::named_pipe::ClientOptions::new()
            .security_qos_flags(windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION)
            .open(&name)
        {
            Ok(pipe) => break pipe,
            Err(error)
                if error.raw_os_error()
                    == Some(windows_sys::Win32::Foundation::ERROR_PIPE_BUSY as i32) =>
            {
                if tokio::time::Instant::now() >= until {
                    return Err(ApiError::new(
                        "busy",
                        "The local desktop automation service is busy. Retry shortly.",
                        true,
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(_) => {
                return Err(ApiError::new(
                    "owner_not_running",
                    "Open the nabnab desktop application and enable local automation.",
                    true,
                ))
            }
        }
    };
    use std::os::windows::io::AsRawHandle;
    credentials::verify_pipe_server(pipe.as_raw_handle().cast()).map_err(|_| {
        ApiError::new(
            "owner_mismatch",
            "The local automation pipe owner could not be verified. No credentials were sent.",
            false,
        )
    })?;
    let bytes = serde_json::to_vec(request).map_err(|_| {
        ApiError::new(
            "invalid_request",
            "The request could not be encoded.",
            false,
        )
    })?;
    tokio::time::timeout(std::time::Duration::from_secs(125), async {
        wire::write_frame(&mut pipe, &bytes, wire::MAX_REQUEST).await?;
        let bytes = wire::read_frame(&mut pipe, wire::MAX_RESPONSE).await?;
        let response: Response = serde_json::from_slice(&bytes).map_err(|_| {
            ApiError::new(
                "invalid_response",
                "The desktop returned unreadable local automation data.",
                false,
            )
        })?;
        response.validate()?;
        let _ = pipe.write_all(&[6]).await;
        Ok(response)
    })
    .await
    .map_err(|_| {
        ApiError::new(
            "request_timeout",
            "The desktop request timed out. Check job status before retrying a mutation.",
            true,
        )
    })?
}
#[cfg(not(windows))]
pub async fn call(_: &Path, _: &Envelope) -> Result<Response, ApiError> {
    Err(ApiError::new(
        "unsupported_platform",
        "Local automation currently requires Windows.",
        false,
    ))
}
