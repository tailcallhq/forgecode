use std::marker::PhantomData;

pub trait Transformer: Sized {
    type Value;

    fn transform(&mut self, value: Self::Value) -> Self::Value;

    fn pipe<B>(self, other: B) -> Pipe<Self, B> {
        Pipe(self, other)
    }

    fn when<F: Fn(&Self::Value) -> bool>(self, cond: F) -> Cond<Self, F>
    where
        Self: Sized,
    {
        Cond(self, cond)
    }
}

pub struct DefaultTransformation<T>(PhantomData<T>);

impl<T> DefaultTransformation<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for DefaultTransformation<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Transformer for DefaultTransformation<T> {
    type Value = T;

    fn transform(&mut self, value: Self::Value) -> Self::Value {
        value
    }
}

pub struct Cond<A, F>(A, F);

impl<A, F> Transformer for Cond<A, F>
where
    A: Transformer,
    F: Fn(&A::Value) -> bool,
{
    type Value = A::Value;

    fn transform(&mut self, value: Self::Value) -> Self::Value {
        let f = &self.1;
        if f(&value) {
            self.0.transform(value)
        } else {
            value
        }
    }
}

pub struct Pipe<A, B>(A, B);

impl<A, B, V> Transformer for Pipe<A, B>
where
    A: Transformer<Value = V>,
    B: Transformer<Value = V>,
{
    type Value = V;

    fn transform(&mut self, value: Self::Value) -> Self::Value {
        self.1.transform(self.0.transform(value))
    }
}

// Re-export specific transformers
mod drop_reasoning_details;
mod image_handling;
mod normalize_tool_args;
mod reasoning_normalizer;
mod set_model;
mod sort_tools;
mod transform_tool_calls;

pub use drop_reasoning_details::DropReasoningDetails;
pub use image_handling::ImageHandling;
pub use normalize_tool_args::NormalizeToolCallArguments;
pub use reasoning_normalizer::ReasoningNormalizer;
pub use set_model::SetModel;
pub use sort_tools::SortTools;
pub use transform_tool_calls::TransformToolCalls;

#[cfg(test)]
mod tests {
    use insta::assert_yaml_snapshot;
    use pretty_assertions::assert_eq;
    use serde::Serialize;

    use super::*;
    use crate::{
        Context, ContextMessage, ToolCallFull, ToolCallId, ToolName, ToolOutput, ToolResult,
    };

    #[derive(Serialize)]
    struct TransformationSnapshot {
        transformation: String,
        before: Context,
        after: Context,
    }

    impl TransformationSnapshot {
        fn new(transformation: &str, before: Context, after: Context) -> Self {
            Self { transformation: transformation.to_string(), before, after }
        }
    }

    fn create_context_with_tool_calls() -> Context {
        let tool_call = ToolCallFull {
            name: ToolName::new("test_tool"),
            call_id: Some(ToolCallId::new("call_123")),
            arguments: serde_json::json!({"param": "value"}).into(),
            thought_signature: None,
        };

        Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::assistant(
                "I'll help you",
                None,
                None,
                Some(vec![tool_call]),
            ))
            .add_tool_results(vec![ToolResult {
                name: ToolName::new("test_tool"),
                call_id: Some(ToolCallId::new("call_123")),
                output: ToolOutput::text("Tool result text".to_string()),
            }])
    }

    #[test]
    fn test_default_transformation() {
        let fixture = Context::default().add_message(ContextMessage::user("Test message", None));

        let mut transformer = DefaultTransformation::<Context>::new();
        let actual = transformer.transform(fixture.clone());
        let expected = fixture;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transformer_pipe() {
        let fixture = create_context_with_tool_calls();
        let transform_tool_calls = TransformToolCalls::new();
        let image_handling = ImageHandling::new();

        let mut combined = transform_tool_calls.pipe(image_handling);
        let actual = combined.transform(fixture.clone());

        let snapshot =
            TransformationSnapshot::new("TransformToolCalls.pipe(ImageHandling)", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }
}
