use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const VERSION: u8 = 1;
pub const MAX_REQUEST: usize = 256 * 1024;
pub const MAX_RESPONSE: usize = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Envelope {
    pub version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    pub method: String,
    pub params: Value,
}
impl Envelope {
    pub fn validate(&self) -> Result<(), ApiError> {
        if self.version != VERSION {
            return Err(ApiError::new(
                "unsupported_version",
                "Use local automation wire version 1.",
                false,
            ));
        }
        if self.method.is_empty()
            || self.method.len() > 80
            || !self.params.is_object()
            || self.client_id.as_ref().is_some_and(|id| {
                id.len() != 32
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            || self.secret.as_ref().is_some_and(|secret| {
                secret.len() != 64
                    || !secret
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err(ApiError::new(
                "invalid_request",
                "Use a supported method, object parameters and valid local client credentials.",
                false,
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}
impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ApiError {}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Response {
    pub version: u8,
    pub ok: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_result"
    )]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}
fn present_result<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}
impl Response {
    pub fn success(result: Value) -> Self {
        Self {
            version: VERSION,
            ok: true,
            result: Some(result),
            error: None,
        }
    }
    pub fn failure(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            version: VERSION,
            ok: false,
            result: None,
            error: Some(ApiError::new(code, message, retryable)),
        }
    }
    pub fn validate(&self) -> Result<(), ApiError> {
        if self.version != VERSION
            || self.ok != self.result.is_some()
            || self.ok == self.error.is_some()
        {
            Err(ApiError::new(
                "invalid_response",
                "The desktop returned an invalid automation response.",
                false,
            ))
        } else {
            Ok(())
        }
    }
}
pub async fn read_frame(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    max: usize,
) -> Result<Vec<u8>, ApiError> {
    use tokio::io::AsyncReadExt;
    let size = reader.read_u32_le().await.map_err(|_| {
        ApiError::new(
            "transport_closed",
            "The local automation connection closed.",
            true,
        )
    })? as usize;
    if size == 0 || size > max {
        return Err(ApiError::new(
            "frame_too_large",
            "The local automation message exceeds its limit.",
            false,
        ));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).await.map_err(|_| {
        ApiError::new(
            "transport_closed",
            "The local automation connection closed before its message completed.",
            true,
        )
    })?;
    Ok(bytes)
}
pub async fn write_frame(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    bytes: &[u8],
    max: usize,
) -> Result<(), ApiError> {
    use tokio::io::AsyncWriteExt;
    if bytes.is_empty() || bytes.len() > max {
        return Err(ApiError::new(
            "frame_too_large",
            "The local automation message exceeds its limit.",
            false,
        ));
    }
    writer.write_u32_le(bytes.len() as u32).await.map_err(|_| {
        ApiError::new(
            "transport_closed",
            "The local automation connection closed.",
            true,
        )
    })?;
    writer.write_all(bytes).await.map_err(|_| {
        ApiError::new(
            "transport_closed",
            "The local automation connection closed.",
            true,
        )
    })?;
    writer.flush().await.map_err(|_| {
        ApiError::new(
            "transport_closed",
            "The local automation connection closed.",
            true,
        )
    })
}
pub async fn write_response(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    bytes: &[u8],
) -> Result<(), ApiError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        write_frame(writer, bytes, MAX_RESPONSE),
    )
    .await
    .map_err(|_| {
        ApiError::new(
            "response_timeout",
            "The local client did not read its response in time.",
            true,
        )
    })?
}
pub fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub struct ToolSpec {
    pub name: &'static str,
    pub method: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub read_only: bool,
}
pub fn tool_specs() -> Vec<ToolSpec> {
    let id = json!({"type":"string","minLength":1,"maxLength":192});
    let url = json!({"type":"string","minLength":1,"maxLength":8192});
    let plan = object(
        json!({
            "operation":{"oneOf":[
                object(json!({"kind":{"const":"download"},"url":url,"format":{"enum":["mp4","mp3","m4a","opus","source"]}}), &["kind","url","format"]),
                object(json!({"kind":{"const":"extractAudio"},"libraryId":id,"format":{"enum":["mp3","m4a"]}}), &["kind","libraryId","format"]),
                object(json!({"kind":{"const":"frame"},"libraryId":id,"at":{"type":"number","minimum":0},"width":{"type":"integer","minimum":64,"maximum":1920}}), &["kind","libraryId","at","width"])
            ]},"outputRootId":id,"collection":{"type":"string","maxLength":160},
            "budget":object(json!({"maxBytes":{"type":"integer","minimum":1048576,"maximum":536870912},"maxSeconds":{"type":"integer","minimum":5,"maximum":600}}), &["maxBytes","maxSeconds"])
        }),
        &["operation", "outputRootId", "collection", "budget"],
    );
    let mut local_plan = plan.clone();
    local_plan["properties"]["operation"]["oneOf"]
        .as_array_mut()
        .expect("static operation schema")
        .remove(0);
    vec![
        ToolSpec{name:"media_inspect",method:"media.inspect",description:"Inspect a public direct HTTP(S) media file without cookies, webpage extractors or HLS.",schema:object(json!({"url":url}), &["url"]),read_only:true},
        ToolSpec{name:"jobs_plan",method:"jobs.plan",description:"Prepare a bounded operation for mandatory approval in the desktop. Downloads accept public direct HTTP(S) media files only, without cookies, webpage extractors or HLS. This never starts a job.",schema:plan.clone(),read_only:false},
        ToolSpec{name:"jobs_enqueue",method:"jobs.enqueue",description:"Enqueue a one-use plan already approved in the desktop for this client.",schema:object(json!({"planId":id}), &["planId"]),read_only:false},
        ToolSpec{name:"jobs_status",method:"jobs.status",description:"Read permitted job status and granted output-root identifiers, collections and budgets.",schema:object(json!({"jobId":id}), &[]),read_only:true},
        ToolSpec{name:"jobs_cancel",method:"jobs.cancel",description:"Cancel a job permitted for this client; existing outputs are preserved.",schema:object(json!({"jobId":id}), &["jobId"]),read_only:false},
        ToolSpec{name:"library_search",method:"library.search",description:"Search permitted local library metadata.",schema:object(json!({"query":{"type":"string","maxLength":200},"limit":{"type":"integer","minimum":1,"maximum":100}}), &[]),read_only:true},
        ToolSpec{name:"library_get_item",method:"library.get_item",description:"Read one permitted library item by its opaque identifier.",schema:object(json!({"itemId":id}), &["itemId"]),read_only:true},
        ToolSpec{name:"tools_run",method:"tools.run",description:"Prepare extractAudio or frame only for mandatory desktop approval; no tool executes before approval and enqueue.",schema:local_plan,read_only:false},
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_rejects_unknown_fields_and_inconsistent_responses() {
        assert!(serde_json::from_value::<Envelope>(
            json!({"version":1,"method":"jobs.status","params":{},"path":"C:/arbitrary"})
        )
        .is_err());
        assert!(Response::success(json!({"jobs":[]})).validate().is_ok());
        let null: Response =
            serde_json::from_value(serde_json::to_value(Response::success(Value::Null)).unwrap())
                .unwrap();
        assert!(null.validate().is_ok());
        assert!(Response {
            version: 1,
            ok: true,
            result: None,
            error: None
        }
        .validate()
        .is_err());
        assert_eq!(tool_specs().len(), 8);
        assert!(tool_specs()
            .iter()
            .all(|tool| tool.schema["additionalProperties"] == false));
    }
    #[tokio::test]
    async fn framing_bounds_before_allocation_and_rejects_truncation() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        write_frame(&mut writer, b"{\"version\":1}", 64)
            .await
            .unwrap();
        assert_eq!(
            read_frame(&mut reader, 64).await.unwrap(),
            b"{\"version\":1}"
        );
        use tokio::io::AsyncWriteExt;
        writer.write_u32_le(1000).await.unwrap();
        assert_eq!(
            read_frame(&mut reader, 64).await.unwrap_err().code,
            "frame_too_large"
        );
    }
    #[tokio::test]
    async fn client_that_never_reads_cannot_hold_a_response_writer_indefinitely() {
        let (mut writer, _reader) = tokio::io::duplex(1);
        let result = write_response(&mut writer, b"a response larger than the pipe capacity").await;
        assert_eq!(result.unwrap_err().code, "response_timeout");
    }
}
