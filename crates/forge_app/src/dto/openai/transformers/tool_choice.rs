use forge_domain::Transformer;

use crate::dto::openai::{Request, ToolChoice};

pub struct SetToolChoice {
    pub choice: ToolChoice,
}

impl SetToolChoice {
    pub fn new(choice: ToolChoice) -> Self {
        Self { choice }
    }
}

impl Transformer for SetToolChoice {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // Only set tool_choice if there are tools defined
        // This prevents "Function calling config is set without
        // function_declarations" error
        if request
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.is_empty())
        {
            request.tool_choice = Some(self.choice.clone());
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ModelId, ToolDefinition};

    use super::*;

    #[test]
    fn test_gemini_transformer_tool_strategy_with_tools() {
        let context = Context::default()
            .add_tool(ToolDefinition::new("test_tool").description("A test tool"));
        let request = Request::from(context).model(ModelId::new("google/gemini-pro"));

        let transformer = SetToolChoice::new(ToolChoice::Auto);
        let mut transformer = transformer;
        let transformed = transformer.transform(request);

        assert_eq!(transformed.tool_choice, Some(ToolChoice::Auto));
    }

    #[test]
    fn test_gemini_transformer_tool_strategy_without_tools() {
        let context = Context::default();
        let request = Request::from(context).model(ModelId::new("google/gemini-pro"));

        let transformer = SetToolChoice::new(ToolChoice::Auto);
        let mut transformer = transformer;
        let transformed = transformer.transform(request);

        // Should not set tool_choice when no tools are present
        assert_eq!(transformed.tool_choice, None);
    }
}
