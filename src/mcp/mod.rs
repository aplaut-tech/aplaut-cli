//! MCP-сервер `aplaut mcp` (спека mcp-server): stdio через `rmcp`. Каждый вызов инструмента —
//! дочерний процесс того же бинаря с `--json` (M3). Async и `rmcp` — только в этом модуле (M4).

pub mod invoke;
pub mod tools;

use std::ffi::OsString;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};

use crate::error::CliError;

/// Всё, что сервер знает с запуска (§1): режим, флаги для дочерних вызовов, `instructions`.
pub struct ServerSetup {
    pub allow_writes: bool,
    pub forwarded: Vec<OsString>,
    pub instructions: String,
}

/// Обслуживает клиента, пока тот не закроет stdin.
pub fn serve(setup: ServerSetup) -> Result<(), CliError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::io("запуск рантайма MCP", &e))?;
    runtime.block_on(async move {
        let service = Server::new(setup)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| {
                CliError::general(
                    "mcp_handshake_failed",
                    format!("MCP: соединение не установлено: {e}"),
                )
                .with_hint("aplaut mcp запускает MCP-клиент: claude mcp add aplaut -- aplaut mcp")
            })?;
        service
            .waiting()
            .await
            .map_err(|e| CliError::general("internal", format!("MCP: {e}")))?;
        Ok(())
    })
}

struct Server {
    tools: Vec<tools::ToolDef>,
    runner: invoke::Runner,
    instructions: String,
}

impl Server {
    fn new(setup: ServerSetup) -> Self {
        Server {
            tools: tools::enabled(setup.allow_writes),
            runner: invoke::Runner::new(setup.forwarded),
            instructions: setup.instructions,
        }
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("aplaut", env!("CARGO_PKG_VERSION")))
            .with_instructions(self.instructions.clone())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(
            self.tools.iter().map(rmcp_tool).collect(),
        ))
    }

    /// Неизвестный инструмент (и запись без `--allow-writes`) — ошибка протокола; всё остальное —
    /// ответ инструмента с конвертом, чтобы модель прочла `code` и `hint` (§4).
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(tool) = self.tools.iter().find(|t| t.name == request.name) else {
            return Err(McpError::invalid_params(
                format!("unknown tool: {}", request.name),
                None,
            ));
        };
        let arguments = request.arguments.unwrap_or_default();
        let reply = invoke::call(&self.runner, tool, &arguments).await;
        let mut content = vec![ContentBlock::text(reply.envelope)];
        content.extend(reply.data.map(ContentBlock::text));
        let result = if reply.is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        };
        Ok(CallToolResponse::Complete(result))
    }
}

/// Аннотации — по §2.3: у read-only `destructiveHint` и `idempotentHint` смысла не имеют.
fn rmcp_tool(def: &tools::ToolDef) -> Tool {
    let mut annotations = ToolAnnotations::new()
        .read_only(def.hints.read_only)
        .open_world(true);
    if !def.hints.read_only {
        annotations = annotations
            .destructive(def.hints.destructive)
            .idempotent(def.hints.idempotent);
    }
    Tool::new(
        def.name.clone(),
        def.description.clone(),
        Arc::new(def.schema.clone()),
    )
    .with_title(def.title.clone())
    .with_annotations(annotations)
}
