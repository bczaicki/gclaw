use gclaw_core::traits::LlmProvider;
use gclaw_core::types::CompletionRequest;
use std::sync::Arc;
use tracing::{debug, warn};

/// Routes completion requests to different providers/models based on config.
pub struct ModelRouter {
    default_provider: Arc<dyn LlmProvider>,
    default_model: String,
    providers: Vec<(String, Arc<dyn LlmProvider>)>,
    routes: Vec<Route>,
    fallback_chain: Vec<(Arc<dyn LlmProvider>, String)>,
}

/// A routing rule that maps a pattern to a specific provider and model.
#[derive(Clone, Debug)]
pub struct Route {
    pub name: String,
    pub provider_name: String,
    pub model: String,
}

impl ModelRouter {
    pub fn new(provider: Arc<dyn LlmProvider>, model: String) -> Self {
        Self {
            default_provider: provider,
            default_model: model,
            providers: Vec::new(),
            routes: Vec::new(),
            fallback_chain: Vec::new(),
        }
    }

    /// Register a named provider for routing.
    pub fn register_provider(&mut self, name: String, provider: Arc<dyn LlmProvider>) {
        self.providers.push((name, provider));
    }

    /// Add a routing rule.
    pub fn add_route(&mut self, route: Route) {
        self.routes.push(route);
    }

    /// Set fallback chain — tried in order if primary provider fails.
    pub fn set_fallback_chain(&mut self, chain: Vec<(Arc<dyn LlmProvider>, String)>) {
        self.fallback_chain = chain;
    }

    /// Resolve which provider and model to use for a request.
    /// Returns (provider, model) — the request's model field may override.
    pub fn resolve(&self, request: &CompletionRequest) -> (Arc<dyn LlmProvider>, String) {
        // If the request specifies a model override, honor it
        if !request.model.is_empty() {
            // Check if the model name contains a provider prefix like "anthropic/claude-sonnet-4-6"
            if let Some((provider_prefix, model_name)) = request.model.split_once('/') {
                if let Some((_, provider)) = self
                    .providers
                    .iter()
                    .find(|(name, _)| name == provider_prefix)
                {
                    debug!("Routing to {provider_prefix}/{model_name}");
                    return (provider.clone(), model_name.to_string());
                }
            }
        }

        // Check tool-aware routing: if request has tools, look for a "coding" route
        if !request.tools.is_empty() {
            if let Some(route) = self.routes.iter().find(|r| r.name == "coding") {
                if let Some((_, provider)) = self
                    .providers
                    .iter()
                    .find(|(name, _)| name == &route.provider_name)
                {
                    debug!(
                        "Tool-aware routing to {}/{}",
                        route.provider_name, route.model
                    );
                    return (provider.clone(), route.model.clone());
                }
            }
        }

        // Check for a "simple" route when no tools are present
        if request.tools.is_empty() {
            if let Some(route) = self.routes.iter().find(|r| r.name == "simple") {
                if let Some((_, provider)) = self
                    .providers
                    .iter()
                    .find(|(name, _)| name == &route.provider_name)
                {
                    debug!("Simple routing to {}/{}", route.provider_name, route.model);
                    return (provider.clone(), route.model.clone());
                }
            }
        }

        (self.default_provider.clone(), self.default_model.clone())
    }

    /// Get the fallback chain for retry after primary failure.
    pub fn fallbacks(&self) -> &[(Arc<dyn LlmProvider>, String)] {
        &self.fallback_chain
    }

    /// Get the default provider.
    pub fn default_provider(&self) -> &Arc<dyn LlmProvider> {
        &self.default_provider
    }

    /// Get the default model.
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Override the default model (used for /model command).
    pub fn set_default_model(&mut self, model: String) {
        // If model contains a provider prefix, update provider too
        if let Some((provider_prefix, model_name)) = model.split_once('/') {
            if let Some((_, provider)) = self
                .providers
                .iter()
                .find(|(name, _)| name == provider_prefix)
            {
                self.default_provider = provider.clone();
                self.default_model = model_name.to_string();
                debug!("Switched default to {provider_prefix}/{model_name}");
                return;
            }
            warn!("Unknown provider prefix '{provider_prefix}', using model name as-is");
        }
        self.default_model = model;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::Stream;
    use gclaw_core::types::*;
    use gclaw_core::Result;
    use std::pin::Pin;

    struct MockProvider {
        name: String,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse> {
            Ok(CompletionResponse {
                message: Message {
                    role: Role::Assistant,
                    content: format!("from {}", self.name),
                    tool_calls: vec![],
                    tool_call_id: None,
                },
                model: self.name.clone(),
                done: true,
            })
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamDelta>> + Send>>> {
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![])
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    fn make_request(model: &str, with_tools: bool) -> CompletionRequest {
        CompletionRequest {
            model: model.to_string(),
            messages: vec![],
            tools: if with_tools {
                vec![ToolDefinition {
                    name: "test".to_string(),
                    description: "test tool".to_string(),
                    parameters: serde_json::json!({}),
                }]
            } else {
                vec![]
            },
            temperature: None,
        }
    }

    #[test]
    fn default_routing() {
        let default = Arc::new(MockProvider {
            name: "ollama".to_string(),
        });
        let router = ModelRouter::new(default, "qwen:9b".to_string());
        let (_, model) = router.resolve(&make_request("", false));
        assert_eq!(model, "qwen:9b");
    }

    #[test]
    fn explicit_provider_prefix() {
        let default = Arc::new(MockProvider {
            name: "ollama".to_string(),
        });
        let anthropic = Arc::new(MockProvider {
            name: "anthropic".to_string(),
        });
        let mut router = ModelRouter::new(default, "qwen:9b".to_string());
        router.register_provider("anthropic".to_string(), anthropic);

        let (provider, model) = router.resolve(&make_request("anthropic/claude-sonnet-4-6", false));
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(model, "claude-sonnet-4-6");
    }

    #[test]
    fn tool_aware_routing() {
        let default = Arc::new(MockProvider {
            name: "ollama".to_string(),
        });
        let anthropic = Arc::new(MockProvider {
            name: "anthropic".to_string(),
        });
        let mut router = ModelRouter::new(default, "qwen:9b".to_string());
        router.register_provider("anthropic".to_string(), anthropic);
        router.add_route(Route {
            name: "coding".to_string(),
            provider_name: "anthropic".to_string(),
            model: "claude-sonnet-4-6".to_string(),
        });

        // With tools → coding route
        let (provider, model) = router.resolve(&make_request("", true));
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(model, "claude-sonnet-4-6");

        // Without tools → default
        let (provider, model) = router.resolve(&make_request("", false));
        assert_eq!(provider.name(), "ollama");
        assert_eq!(model, "qwen:9b");
    }

    #[test]
    fn set_default_model_with_prefix() {
        let default = Arc::new(MockProvider {
            name: "ollama".to_string(),
        });
        let anthropic = Arc::new(MockProvider {
            name: "anthropic".to_string(),
        });
        let mut router = ModelRouter::new(default, "qwen:9b".to_string());
        router.register_provider("anthropic".to_string(), anthropic);

        router.set_default_model("anthropic/claude-sonnet-4-6".to_string());
        assert_eq!(router.default_model(), "claude-sonnet-4-6");
        assert_eq!(router.default_provider().name(), "anthropic");
    }
}
