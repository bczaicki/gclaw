use crate::types::{CompletionRequest, CompletionResponse, ModelInfo, StreamDelta};
use crate::Result;
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamDelta>> + Send>>>;

    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    fn name(&self) -> &str;
}
