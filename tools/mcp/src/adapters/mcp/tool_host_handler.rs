use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, GetPromptRequestParams, GetPromptResponse,
    Implementation, ListPromptsResult, ListToolsResult, PaginatedRequestParams, PromptsCapability,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;

use super::mappers::{OutcomeMapper, PromptMapper, ToolMapper};
use crate::ports::ToolHost;

/// The `rmcp` handler that answers the protocol from a [`ToolHost`].
#[derive(Clone)]
pub(crate) struct ToolHostHandler {
    host: Arc<dyn ToolHost>,
}

impl ToolHostHandler {
    pub(crate) fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }

    fn capabilities(&self) -> ServerCapabilities {
        let mut capabilities = ServerCapabilities::builder().enable_tools().build();
        if !self.host.prompts().is_empty() {
            capabilities.prompts = Some(PromptsCapability::default());
        }
        capabilities
    }
}

impl ServerHandler for ToolHostHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(self.capabilities())
            .with_server_info(Implementation::new(self.host.name(), self.host.version()))
            .with_instructions(self.host.instructions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self
            .host
            .tools()
            .into_iter()
            .map(ToolMapper::to_tool)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let outcome = self.host.call(&request.name, arguments).await;
        Ok(OutcomeMapper::to_result(outcome, self.host.output_format()).into())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let prompts = self
            .host
            .prompts()
            .into_iter()
            .map(PromptMapper::to_prompt)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListPromptsResult::with_all_items(prompts))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let rendered = self.host.prompt(&request.name, &arguments).ok_or_else(|| {
            ErrorData::invalid_params(format!("unknown prompt: {}", request.name), None)
        })?;
        Ok(PromptMapper::to_result(rendered)?.into())
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::Value;

    use super::*;
    use crate::domain::ToolOutcome;
    use crate::ports::FakeToolHost;

    struct ToolsOnlyHost;

    #[async_trait]
    impl ToolHost for ToolsOnlyHost {
        fn name(&self) -> &'static str {
            "tools-only"
        }

        fn version(&self) -> &'static str {
            "0.0.1"
        }

        fn instructions(&self) -> &'static str {
            "only tools"
        }

        fn tools(&self) -> Vec<Value> {
            Vec::new()
        }

        async fn call(&self, _name: &str, _arguments: Value) -> ToolOutcome {
            ToolOutcome::failed("no tools", None)
        }
    }

    #[test]
    fn prompts_are_announced_only_when_the_host_has_them() {
        let tools_only = ToolHostHandler::new(Arc::new(ToolsOnlyHost)).get_info();
        assert!(tools_only.capabilities.prompts.is_none());
        assert!(tools_only.capabilities.tools.is_some());
        let with_prompts = ToolHostHandler::new(Arc::new(FakeToolHost)).get_info();
        assert!(with_prompts.capabilities.prompts.is_some());
    }

    #[test]
    fn a_host_without_prompts_renders_none() {
        assert!(ToolsOnlyHost.prompt("anything", &Value::Null).is_none());
    }
}
