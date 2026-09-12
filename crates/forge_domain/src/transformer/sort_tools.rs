use super::Transformer;
use crate::{Context, ToolOrder};

/// Transformer that sorts tools in the context according to a specified
/// ordering strategy
pub struct SortTools {
    order: ToolOrder,
}

impl SortTools {
    pub fn new(order: ToolOrder) -> Self {
        Self { order }
    }
}

impl Default for SortTools {
    fn default() -> Self {
        Self::new(ToolOrder::default())
    }
}

impl Transformer for SortTools {
    type Value = Context;

    fn transform(&mut self, mut context: Self::Value) -> Self::Value {
        self.order.sort(&mut context.tools);
        context
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::ToolDefinition;

    fn fixture_context_with_tools() -> Context {
        Context::default().tools(vec![
            ToolDefinition::new("zebra_tool").description("Z tool"),
            ToolDefinition::new("alpha_tool").description("A tool"),
            ToolDefinition::new("beta_tool").description("B tool"),
        ])
    }

    #[test]
    fn test_sorts_tools_alphabetically() {
        let fixture = fixture_context_with_tools();

        let mut transformer = SortTools::new(ToolOrder::new(vec![])); // Empty = alphabetical
        let actual = transformer.transform(fixture);

        let expected_order = vec!["alpha_tool", "beta_tool", "zebra_tool"];
        let actual_order: Vec<String> = actual
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();

        assert_eq!(actual_order, expected_order);
    }

    #[test]
    fn test_sorts_tools_with_custom_order() {
        use crate::ToolName;

        let fixture = fixture_context_with_tools();

        let custom_order = ToolOrder::new(vec![
            ToolName::new("zebra_tool"),
            ToolName::new("alpha_tool"),
        ]);
        let mut transformer = SortTools::new(custom_order);
        let actual = transformer.transform(fixture);

        // zebra_tool and alpha_tool come first (in that order), rest
        // alphabetically
        let expected_order = vec!["zebra_tool", "alpha_tool", "beta_tool"];
        let actual_order: Vec<String> = actual
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();

        assert_eq!(actual_order, expected_order);
    }
}
