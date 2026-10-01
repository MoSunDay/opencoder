use opencoder_core::{
    config::{Endpoint, HttpHeader, ProviderConfig},
    Config, ProviderProtocol,
};

pub(crate) fn guest_config(source: &Config, endpoint: Option<Endpoint>) -> Config {
    let mut config = source.clone();
    config.dag = Default::default();
    config.opencoder_server = Default::default();
    config.providers.clear();
    config.agent.agents_dir = source
        .agent
        .agents_dir
        .as_ref()
        .map(|_| super::io::AGENTS_MOUNT.into());
    if let Some(endpoint) = endpoint {
        config.provider = ProviderConfig {
            protocol: match endpoint.protocol {
                ProviderProtocol::ChatCompletions => "chat_completions",
                ProviderProtocol::Responses => "responses",
            }
            .into(),
            base_url: endpoint.base_url,
            api_key: Some(endpoint.api_key),
            model: Some(source.model_id().into()),
            headers: endpoint
                .headers
                .into_iter()
                .map(|(name, value)| HttpHeader { name, value })
                .collect(),
        };
    } else {
        config.provider = Default::default();
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_preserves_protocol_headers_and_selected_model_without_host_paths() {
        let mut source = Config {
            model: "fixture/selected".into(),
            ..Default::default()
        };
        source.agent.agents_dir = Some("/host/agents".into());
        source.dag.workspace_dir = Some("/host/source".into());
        let config = guest_config(
            &source,
            Some(Endpoint {
                protocol: ProviderProtocol::Responses,
                provider: "fixture".into(),
                base_url: "http://fixture/v1".into(),
                api_key: "fixture-key".into(),
                headers: vec![("x-fixture".into(), "resolved".into())],
            }),
        );
        let endpoint = config.resolve_endpoint().unwrap();
        assert_eq!(config.model_id(), "selected");
        assert_eq!(endpoint.protocol, ProviderProtocol::Responses);
        assert_eq!(
            endpoint.headers,
            vec![("x-fixture".into(), "resolved".into())]
        );
        assert!(config.dag.workspace_dir.is_none());
        assert_eq!(
            config.agent.agents_dir.unwrap().to_str(),
            Some(super::super::io::AGENTS_MOUNT)
        );
    }
}
