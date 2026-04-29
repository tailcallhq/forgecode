use std::sync::Arc;

use anyhow::{Context, anyhow};
use forge_app::{
    EnvironmentInfra, HttpInfra, WebSearchAnswerBox, WebSearchKnowledgeGraph, WebSearchOrganicResult,
    WebSearchRelatedQuestion, WebSearchResponse, WebSearchService, WebSearchTopStory,
};
use forge_domain::{WebSearch, WebSearchDevice, WebSearchMode, WebSearchSafe};
use reqwest::Url;
use serde::Deserialize;
use thiserror::Error;

const SERP_API_URL: &str = "https://serpapi.com/search";
const SERPAPI_API_KEY_ENV: &str = "SERPAPI_API_KEY";

#[derive(Debug, Error)]
enum WebSearchError {
    #[error("SERPAPI_API_KEY is not set")]
    MissingApiKey,
    #[error("Search query cannot be empty")]
    EmptyQuery,
    #[error("SerpApi returned an error: {0}")]
    Api(String),
}

/// Searches the public web through SerpApi-backed Google search.
pub struct ForgeWebSearch<F>(Arc<F>);

impl<F> ForgeWebSearch<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self(infra)
    }
}

impl<F: EnvironmentInfra<Config = forge_config::ForgeConfig>> ForgeWebSearch<F> {
    fn api_key(&self) -> anyhow::Result<String> {
        self.0
            .get_env_var(SERPAPI_API_KEY_ENV)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!(WebSearchError::MissingApiKey))
    }

    fn build_url(&self, params: &WebSearch, api_key: &str) -> anyhow::Result<Url> {
        if params.query.trim().is_empty() {
            return Err(anyhow!(WebSearchError::EmptyQuery));
        }

        let mut url = Url::parse(SERP_API_URL)?;
        let engine = match params.mode {
            WebSearchMode::Light => "google_light",
            WebSearchMode::Standard => "google",
        };

        {
            let mut query = url.query_pairs_mut();
            query.append_pair("engine", engine);
            query.append_pair("q", params.query.trim());
            query.append_pair("api_key", api_key);

            if let Some(location) = params.location.as_deref() {
                query.append_pair("location", location);
            }
            if let Some(google_domain) = params.google_domain.as_deref() {
                query.append_pair("google_domain", google_domain);
            }
            if let Some(gl) = params.gl.as_deref() {
                query.append_pair("gl", gl);
            }
            if let Some(hl) = params.hl.as_deref() {
                query.append_pair("hl", hl);
            }
            if let Some(start) = params.start {
                query.append_pair("start", &start.to_string());
            }
            if let Some(safe) = params.safe {
                query.append_pair("safe", match safe {
                    WebSearchSafe::Active => "active",
                    WebSearchSafe::Off => "off",
                });
            }
            if let Some(device) = params.device {
                query.append_pair("device", match device {
                    WebSearchDevice::Desktop => "desktop",
                    WebSearchDevice::Tablet => "tablet",
                    WebSearchDevice::Mobile => "mobile",
                });
            }
            if let Some(no_cache) = params.no_cache {
                query.append_pair("no_cache", &no_cache.to_string());
            }
        }

        Ok(url)
    }

    fn parse_response(&self, params: &WebSearch, body: &[u8]) -> anyhow::Result<WebSearchResponse> {
        let response: SerpApiResponse =
            serde_json::from_slice(body).context("Failed to parse SerpApi response")?;

        if let Some(error) = response.error {
            return Err(anyhow!(WebSearchError::Api(error)));
        }

        let engine = response
            .search_parameters
            .and_then(|value| value.engine)
            .unwrap_or_else(|| match params.mode {
                WebSearchMode::Light => "google_light".to_string(),
                WebSearchMode::Standard => "google".to_string(),
            });

        let answer_box = response.answer_box.map(|value| WebSearchAnswerBox {
            title: value.title,
            answer: value.answer,
            snippet: value.snippet,
            link: value.link,
        });

        let knowledge_graph = response.knowledge_graph.map(|value| WebSearchKnowledgeGraph {
            title: value.title,
            entity_type: value.entity_type,
            description: value.description,
            website: value.website,
        });

        let organic_results = response
            .organic_results
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                let title = value.title?;
                let link = value.link?;
                Some(WebSearchOrganicResult {
                    position: value.position,
                    title,
                    link,
                    displayed_link: value.displayed_link,
                    source: value.source,
                    snippet: value.snippet,
                })
            })
            .collect();

        let related_questions = response
            .related_questions
            .unwrap_or_default()
            .into_iter()
            .map(|value| WebSearchRelatedQuestion {
                question: value.question,
                snippet: value.snippet,
            })
            .collect();

        let related_searches = response
            .related_searches
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| match value {
                SerpRelatedSearch::Query { query } => Some(query),
                SerpRelatedSearch::Text(query) => Some(query),
            })
            .collect();

        let top_stories = response
            .top_stories
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| {
                Some(WebSearchTopStory {
                    title: value.title?,
                    link: value.link,
                    source: value.source,
                    date: value.date,
                    snippet: value.snippet,
                })
            })
            .collect();

        Ok(WebSearchResponse {
            query: params.query.clone(),
            engine,
            search_id: response.search_metadata.and_then(|value| value.id),
            answer_box,
            knowledge_graph,
            organic_results,
            related_questions,
            related_searches,
            top_stories,
        })
    }
}

#[async_trait::async_trait]
impl<F: HttpInfra + EnvironmentInfra<Config = forge_config::ForgeConfig> + Send + Sync>
    WebSearchService for ForgeWebSearch<F>
{
    async fn web_search(&self, params: WebSearch) -> anyhow::Result<WebSearchResponse> {
        let api_key = self.api_key()?;
        let url = self.build_url(&params, &api_key)?;
        let response = self
            .0
            .http_get(&url, None)
            .await
            .with_context(|| format!("Failed to execute web search for query '{}'", params.query))?;
        let body = response.bytes().await.context("Failed to read SerpApi response body")?;

        self.parse_response(&params, &body)
    }
}

#[derive(Debug, Deserialize)]
struct SerpApiResponse {
    #[serde(default)]
    search_metadata: Option<SerpSearchMetadata>,
    #[serde(default)]
    search_parameters: Option<SerpSearchParameters>,
    #[serde(default)]
    answer_box: Option<SerpAnswerBox>,
    #[serde(default)]
    knowledge_graph: Option<SerpKnowledgeGraph>,
    #[serde(default)]
    organic_results: Option<Vec<SerpOrganicResult>>,
    #[serde(default)]
    related_questions: Option<Vec<SerpRelatedQuestion>>,
    #[serde(default)]
    related_searches: Option<Vec<SerpRelatedSearch>>,
    #[serde(default)]
    top_stories: Option<Vec<SerpTopStory>>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerpSearchMetadata {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerpSearchParameters {
    #[serde(default)]
    engine: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerpAnswerBox {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    answer: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerpKnowledgeGraph {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "type")]
    entity_type: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    website: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerpOrganicResult {
    #[serde(default)]
    position: Option<u32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    displayed_link: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SerpRelatedQuestion {
    question: String,
    #[serde(default)]
    snippet: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SerpRelatedSearch {
    Query { query: String },
    Text(String),
}

#[derive(Debug, Deserialize)]
struct SerpTopStory {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use forge_app::{HttpInfra, domain::Environment};
    use forge_domain::{ConfigOperation, WebSearch};
    use pretty_assertions::assert_eq;
    use reqwest::Response;
    use reqwest::header::HeaderMap;
    use reqwest_eventsource::EventSource;

    use super::*;

    struct MockInfra {
        env: BTreeMap<String, String>,
    }

    impl EnvironmentInfra for MockInfra {
        type Config = forge_config::ForgeConfig;

        fn get_env_var(&self, key: &str) -> Option<String> {
            self.env.get(key).cloned()
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            self.env.clone()
        }

        fn get_environment(&self) -> Environment {
            use fake::{Fake, Faker};
            Faker.fake()
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            Ok(forge_config::ForgeConfig::default())
        }

        async fn update_environment(&self, _ops: Vec<ConfigOperation>) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl HttpInfra for MockInfra {
        async fn http_get(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
        ) -> anyhow::Result<Response> {
            unimplemented!()
        }

        async fn http_post(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
            _body: bytes::Bytes,
        ) -> anyhow::Result<Response> {
            unimplemented!()
        }

        async fn http_delete(&self, _url: &Url) -> anyhow::Result<Response> {
            unimplemented!()
        }

        async fn http_eventsource(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
            _body: bytes::Bytes,
        ) -> anyhow::Result<EventSource> {
            unimplemented!()
        }
    }

    fn fixture() -> ForgeWebSearch<MockInfra> {
        ForgeWebSearch::new(Arc::new(MockInfra {
            env: BTreeMap::from([(SERPAPI_API_KEY_ENV.to_string(), "secret".to_string())]),
        }))
    }

    #[tokio::test]
    async fn test_web_search_requires_api_key() {
        let setup = ForgeWebSearch::new(Arc::new(MockInfra { env: BTreeMap::new() }));

        let actual = setup.web_search(WebSearch::default().query("rust web frameworks")).await;
        let expected = "SERPAPI_API_KEY is not set";

        assert_eq!(actual.unwrap_err().to_string(), expected);
    }

    #[test]
    fn test_build_url_uses_light_mode_by_default() {
        let setup = fixture();
        let actual = setup
            .build_url(&WebSearch::default().query("rust web frameworks"), "secret")
            .unwrap();
        let expected = Some("google_light");

        assert_eq!(
            actual.query_pairs().find(|(key, _)| key == "engine").map(|(_, value)| value.to_string()).as_deref(),
            expected
        );
    }

    #[test]
    fn test_build_url_uses_standard_mode_when_requested() {
        let setup = fixture();
        let actual = setup
            .build_url(
                &WebSearch::default()
                    .query("rust web frameworks")
                    .mode(WebSearchMode::Standard),
                "secret",
            )
            .unwrap();
        let expected = Some("google");

        assert_eq!(
            actual.query_pairs().find(|(key, _)| key == "engine").map(|(_, value)| value.to_string()).as_deref(),
            expected
        );
    }

    #[test]
    fn test_build_url_forwards_optional_parameters() {
        let setup = fixture();
        let actual = setup
            .build_url(
                &WebSearch::default()
                    .query("best rust books")
                    .location("Austin, Texas, United States")
                    .google_domain("google.com")
                    .gl("us")
                    .hl("en")
                    .start(10_u32)
                    .safe(WebSearchSafe::Off)
                    .device(WebSearchDevice::Mobile)
                    .no_cache(true),
                "secret",
            )
            .unwrap();
        let expected = BTreeMap::from([
            ("device".to_string(), "mobile".to_string()),
            ("engine".to_string(), "google_light".to_string()),
            ("gl".to_string(), "us".to_string()),
            ("google_domain".to_string(), "google.com".to_string()),
            ("hl".to_string(), "en".to_string()),
            ("location".to_string(), "Austin, Texas, United States".to_string()),
            ("no_cache".to_string(), "true".to_string()),
            ("q".to_string(), "best rust books".to_string()),
            ("safe".to_string(), "off".to_string()),
            ("start".to_string(), "10".to_string()),
        ]);

        let actual = actual
            .query_pairs()
            .filter(|(key, _)| key != "api_key")
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_response_normalizes_light_results() {
        let setup = fixture();
        let actual = setup
            .parse_response(
                &WebSearch::default().query("coffee"),
                br#"{
                    "search_metadata": { "id": "abc123" },
                    "search_parameters": { "engine": "google_light" },
                    "organic_results": [
                        {
                            "position": 1,
                            "title": "Coffee - Wikipedia",
                            "link": "https://en.wikipedia.org/wiki/Coffee",
                            "displayed_link": "en.wikipedia.org \u203a wiki \u203a Coffee",
                            "snippet": "Coffee is a brewed drink."
                        }
                    ],
                    "related_searches": [
                        { "query": "coffee beans" },
                        "coffee near me"
                    ]
                }"#,
            )
            .unwrap();
        let expected = WebSearchResponse {
            query: "coffee".to_string(),
            engine: "google_light".to_string(),
            search_id: Some("abc123".to_string()),
            answer_box: None,
            knowledge_graph: None,
            organic_results: vec![WebSearchOrganicResult {
                position: Some(1),
                title: "Coffee - Wikipedia".to_string(),
                link: "https://en.wikipedia.org/wiki/Coffee".to_string(),
                displayed_link: Some("en.wikipedia.org \u{203a} wiki \u{203a} Coffee".to_string()),
                source: None,
                snippet: Some("Coffee is a brewed drink.".to_string()),
            }],
            related_questions: vec![],
            related_searches: vec!["coffee beans".to_string(), "coffee near me".to_string()],
            top_stories: vec![],
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_response_normalizes_standard_rich_results() {
        let setup = fixture();
        let actual = setup
            .parse_response(
                &WebSearch::default()
                    .query("saturn")
                    .mode(WebSearchMode::Standard),
                br#"{
                    "search_metadata": { "id": "xyz789" },
                    "search_parameters": { "engine": "google" },
                    "answer_box": {
                        "title": "Saturn",
                        "answer": "A gas giant planet",
                        "link": "https://example.com/saturn"
                    },
                    "knowledge_graph": {
                        "title": "Saturn",
                        "type": "Planet",
                        "description": "The sixth planet from the Sun.",
                        "website": "https://science.nasa.gov/saturn/"
                    },
                    "organic_results": [
                        {
                            "position": 1,
                            "title": "Saturn Facts",
                            "link": "https://science.nasa.gov/saturn/facts/",
                            "displayed_link": "science.nasa.gov \u203a saturn \u203a facts",
                            "source": "NASA",
                            "snippet": "Saturn facts and figures."
                        }
                    ],
                    "related_questions": [
                        {
                            "question": "What is Saturn made of?",
                            "snippet": "Mostly hydrogen and helium."
                        }
                    ],
                    "top_stories": [
                        {
                            "title": "New Saturn mission announced",
                            "link": "https://example.com/story",
                            "source": "Space News",
                            "date": "1 day ago",
                            "snippet": "A new mission could launch soon."
                        }
                    ]
                }"#,
            )
            .unwrap();
        let expected = WebSearchResponse {
            query: "saturn".to_string(),
            engine: "google".to_string(),
            search_id: Some("xyz789".to_string()),
            answer_box: Some(WebSearchAnswerBox {
                title: Some("Saturn".to_string()),
                answer: Some("A gas giant planet".to_string()),
                snippet: None,
                link: Some("https://example.com/saturn".to_string()),
            }),
            knowledge_graph: Some(WebSearchKnowledgeGraph {
                title: Some("Saturn".to_string()),
                entity_type: Some("Planet".to_string()),
                description: Some("The sixth planet from the Sun.".to_string()),
                website: Some("https://science.nasa.gov/saturn/".to_string()),
            }),
            organic_results: vec![WebSearchOrganicResult {
                position: Some(1),
                title: "Saturn Facts".to_string(),
                link: "https://science.nasa.gov/saturn/facts/".to_string(),
                displayed_link: Some("science.nasa.gov \u{203a} saturn \u{203a} facts".to_string()),
                source: Some("NASA".to_string()),
                snippet: Some("Saturn facts and figures.".to_string()),
            }],
            related_questions: vec![WebSearchRelatedQuestion {
                question: "What is Saturn made of?".to_string(),
                snippet: Some("Mostly hydrogen and helium.".to_string()),
            }],
            related_searches: vec![],
            top_stories: vec![WebSearchTopStory {
                title: "New Saturn mission announced".to_string(),
                link: Some("https://example.com/story".to_string()),
                source: Some("Space News".to_string()),
                date: Some("1 day ago".to_string()),
                snippet: Some("A new mission could launch soon.".to_string()),
            }],
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_response_surfaces_serpapi_errors() {
        let setup = fixture();
        let actual = setup.parse_response(
            &WebSearch::default().query("blocked"),
            br#"{ "error": "Invalid API key." }"#,
        );
        let expected = "SerpApi returned an error: Invalid API key.";

        assert_eq!(actual.unwrap_err().to_string(), expected);
    }
}
