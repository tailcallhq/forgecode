use forge_domain::Transformer;

use crate::dto::openai::Request;

/// Makes the Request compatible with xAI's API.
/// xAI's /v1/chat/completions is OpenAI-compatible but rejects several
/// parameters that OpenAI accepts.
pub struct MakeXaiCompat;

impl Transformer for MakeXaiCompat {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // xAI rejects stream_options
        request.stream_options = None;

        // xAI uses max_tokens, not max_completion_tokens
        // MakeOpenAiCompat already moved max_tokens → max_completion_tokens,
        // so move it back
        if request.max_completion_tokens.is_some() && request.max_tokens.is_none() {
            request.max_tokens = request.max_completion_tokens.take();
        }
        request.max_completion_tokens = None;

        // xAI does not support parallel_tool_calls
        request.parallel_tool_calls = None;

        // xAI does not support prediction
        request.prediction = None;

        // xAI does not support reasoning_effort (uses model variants instead)
        request.reasoning_effort = None;

        // xAI does not support thinking config
        request.thinking = None;

        // xAI does not support logit_bias
        request.logit_bias = None;

        // xAI does not support top_logprobs
        request.top_logprobs = None;

        // xAI does not support seed
        request.seed = None;

        request
    }
}
