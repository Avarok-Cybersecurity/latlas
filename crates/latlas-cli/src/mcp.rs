// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The stdio MCP server (rmcp 0.16): `tools/list` serves
//! `latlas_core::tools::tools()` verbatim (the schemas the token gate measured) and
//! `tools/call` dispatches to [`crate::handlers::call`]. A tool error is a tool result with
//! `isError` and the registry code, so the client sees the code, not a transport failure.
//!
//! Owner: latlas-cli (cut from metrale-kg-mcp, 2026-10-23 K0).
//! Invariants: no tool is served that `tools()` does not list.

use crate::app::App;
use crate::handlers;
use crate::host::PackHost;
use latlas_core::Pack;
use latlas_core::tools::tools;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 2026-10-23: The server handler around the shared app.
pub struct Server<P: Pack, H: PackHost<P>> {
    pub app: Arc<Mutex<App<P, H>>>,
    /// 2026-10-23: The name the server reports at `initialize` (the binary's, e.g. `metrale-kg`).
    pub name: &'static str,
    /// 2026-10-23: The version it reports (the binary's).
    pub version: &'static str,
}

impl<P: Pack, H: PackHost<P>> Clone for Server<P, H> {
    fn clone(&self) -> Self {
        Server {
            app: self.app.clone(),
            name: self.name,
            version: self.version,
        }
    }
}

/// 2026-10-23: The rmcp `Tool` list from the one schema definition.
pub fn tool_list<P: Pack>() -> Vec<Tool> {
    tools::<P>()
        .into_iter()
        .filter_map(|t| serde_json::from_value::<Tool>(t).ok())
        .collect()
}

impl<P, H> ServerHandler for Server<P, H>
where
    P: Pack + Send + Sync + 'static,
    H: PackHost<P> + Send + Sync + 'static,
    App<P, H>: Send,
{
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: self.name.into(),
                title: None,
                version: self.version.into(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(P::INSTRUCTIONS.into()),
            ..ServerInfo::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(tool_list::<P>()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        let mut app = self.app.lock().await;
        Ok(match handlers::call(&mut app, &request.name, &args).await {
            Ok(v) => CallToolResult::success(vec![Content::text(v.to_string())]),
            Err(e) => {
                let body = serde_json::json!({"error": e.code().name(), "message": e.to_string()});
                CallToolResult::error(vec![Content::text(body.to_string())])
            }
        })
    }
}
