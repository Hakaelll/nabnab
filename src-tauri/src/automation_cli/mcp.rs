use crate::{
    automation_client, automation_credentials as credentials,
    automation_wire::{self as wire, ApiError, Envelope, Response},
};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
        ToolAnnotations,
    },
    service::RequestContext,
    ErrorData, RoleServer, ServerHandler, ServiceExt,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

struct Desktop {
    root: PathBuf,
    client_id: String,
    secret: String,
}
fn definition(spec: wire::ToolSpec) -> Tool {
    let mut tool = Tool::new(
        spec.name,
        spec.description,
        spec.schema.as_object().cloned().unwrap_or_default(),
    );
    tool.annotations = Some(
        ToolAnnotations::new()
            .read_only(spec.read_only)
            .destructive(spec.method == "jobs.cancel")
            .open_world(spec.method == "media.inspect" || spec.method == "jobs.plan"),
    );
    tool
}
impl ServerHandler for Desktop {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("nabnab-local",env!("CARGO_PKG_VERSION")))
            .with_instructions("Keep the desktop or an explicitly launched local worker open. A new client and every operation plan require approval in its local Settings. Never request capture, microphones, arbitrary paths, shell commands or credentials. tools_run prepares a plan only. After local approval use jobs_enqueue with its one-use planId.")
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut result = ListToolsResult::default();
        result.tools = wire::tool_specs().into_iter().map(definition).collect();
        Ok(result)
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        wire::tool_specs()
            .into_iter()
            .find(|spec| spec.name == name)
            .map(definition)
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let spec = wire::tool_specs()
            .into_iter()
            .find(|spec| spec.name == request.name)
            .ok_or_else(|| ErrorData::invalid_params("Unknown local automation tool.", None))?;
        let envelope = Envelope {
            version: wire::VERSION,
            client_id: Some(self.client_id.clone()),
            secret: Some(self.secret.clone()),
            method: spec.method.into(),
            params: Value::Object(request.arguments.unwrap_or_default()),
        };
        let response = automation_client::call(&self.root, &envelope)
            .await
            .unwrap_or_else(|error| Response::failure(error.code, error.message, error.retryable));
        let value = serde_json::to_value(&response).map_err(|_| {
            ErrorData::internal_error("The local response could not be encoded.", None)
        })?;
        let content = vec![ContentBlock::text(value.to_string())];
        let mut result = if response.ok {
            CallToolResult::success(content)
        } else {
            CallToolResult::error(content)
        };
        result.structured_content = Some(value);
        Ok(result.into())
    }
}
pub async fn serve(root: &Path, id: &str) -> Result<(), ApiError> {
    let secret = credentials::load(root, id).map_err(|_| {
        ApiError::new(
            "credentials_unavailable",
            "Connect this client and approve it in the desktop before starting MCP.",
            false,
        )
    })?;
    let status = automation_client::call(
        root,
        &Envelope {
            version: wire::VERSION,
            client_id: Some(id.into()),
            secret: Some(secret.clone()),
            method: "client.status".into(),
            params: serde_json::json!({}),
        },
    )
    .await?;
    if !status.ok {
        let error = status.error.unwrap_or_else(|| {
            ApiError::new(
                "unauthorized",
                "Approve this client in the local desktop first.",
                false,
            )
        });
        return Err(error);
    }
    let service = Desktop {
        root: root.to_owned(),
        client_id: id.into(),
        secret,
    }
    .serve(rmcp::transport::stdio())
    .await
    .map_err(|_| {
        ApiError::new(
            "mcp_transport",
            "The MCP stdio session could not initialize.",
            false,
        )
    })?;
    service.waiting().await.map_err(|_| {
        ApiError::new(
            "mcp_transport",
            "The MCP stdio session ended unexpectedly.",
            false,
        )
    })?;
    Ok(())
}
