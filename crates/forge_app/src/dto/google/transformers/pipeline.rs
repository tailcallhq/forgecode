use forge_domain::{DefaultTransformation, Provider, Transformer};
use url::Url;

use super::reasoning_effort::ReasoningEffort;
use super::set_thinking::SetThinking;
use crate::dto::google::Request;

/// Pipeline for transforming requests based on the provider type
pub struct ProviderPipeline<'a> {
    #[allow(dead_code)]
    provider: &'a Provider<Url>,
    model_id: &'a str,
}

impl<'a> ProviderPipeline<'a> {
    /// Creates a new provider pipeline for the given provider
    pub fn new(provider: &'a Provider<Url>, model_id: &'a str) -> Self {
        Self { provider, model_id }
    }
}

impl Transformer for ProviderPipeline<'_> {
    type Value = Request;

    fn transform(&mut self, request: Self::Value) -> Self::Value {
        let set_thinking = SetThinking::new(self.model_id);
        let reasoning_effort = ReasoningEffort;

        let mut combined = DefaultTransformation::<Request>::new()
            .pipe(set_thinking)
            .pipe(reasoning_effort);

        combined.transform(request)
    }
}
