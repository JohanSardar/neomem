mod tools;

use std::sync::Arc;
use tools::{ToolEntry, table};

use neomem_core::{EpochMs, IntakeRecord, Span};
use neomem_proto::{IncomingEnvelope, PROTOCOL_VERSION, Request, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};

use crate::dispatch::Dispatcher;

#[derive(Debug, Clone)]
pub struct McpServer(pub Arc<Dispatcher>);

impl McpServer {
    fn request(&self, name: &str, mut arguments: Value) -> anyhow::Result<Request> {
        let entry = table()
            .into_iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;
        let object =
            arguments.as_object_mut().ok_or_else(|| anyhow::anyhow!("arguments must be an object"))?;
        anyhow::ensure!(object.keys().all(|key| entry.properties.get(key).is_some()), "unknown argument");
        if name == "add" {
            let start: Option<EpochMs> = object.remove("start").map(serde_json::from_value).transpose()?;
            let end: Option<EpochMs> = object.remove("end").map(serde_json::from_value).transpose()?;
            let occurred = match (start, end) {
                (Some(start), Some(end)) => Span::new(start, end)?,
                (Some(only), None) | (None, Some(only)) => Span::instant(only),
                (None, None) => Span::instant(self.0.now()),
            };
            object.insert("occurred".into(), serde_json::to_value(occurred)?);
            let record: IntakeRecord = serde_json::from_value(arguments)?;
            record.validate()?;
            return Ok(Request::Add(record));
        }
        if name == "raw" {
            object.entry("text").or_insert_with(|| json!(""));
        }
        Ok(serde_json::from_value(json!({"command":name,"arguments":arguments}))?)
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        table().into_iter().find(|entry| entry.name == name).map(ToolEntry::tool)
    }

    fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> {
        std::future::ready(Ok(ListToolsResult {
            tools: table().into_iter().map(ToolEntry::tool).collect(),
            ..Default::default()
        }))
    }

    async fn call_tool(
        &self,
        params: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, ErrorData> {
        let request = self.request(&params.name, Value::Object(params.arguments.unwrap_or_default()));
        let result = match request {
            Ok(request) => {
                let incoming = IncomingEnvelope {
                    protocol: PROTOCOL_VERSION,
                    request: serde_json::to_value(request)
                        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?,
                };
                let dispatcher = Arc::clone(&self.0);
                match tokio::task::spawn_blocking(move || dispatcher.dispatch(incoming)).await {
                    Ok(response) => response,
                    Err(error) => {
                        return Ok(CallToolResult::error(vec![ContentBlock::text(error.to_string())]).into());
                    }
                }
            }
            Err(error) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(error.to_string())]).into());
            }
        };
        let response: Response = result;
        let text = serde_json::to_string(&response)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let content = vec![ContentBlock::text(text)];
        Ok(if matches!(response, Response::Failed(_)) {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        }
        .into())
    }
}
