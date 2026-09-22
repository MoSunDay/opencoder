//! Shared finite-planner configuration.
use opencoder_llm::ChatRequest;

pub fn configured_request(
    config: &opencoder_core::Config,
    mut request: ChatRequest,
) -> ChatRequest {
    if request.reasoning_effort.is_none() {
        request.reasoning_effort = config.reasoning_effort.clone();
    }
    request
}
