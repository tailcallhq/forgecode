use derive_more::{Debug, From};

#[derive(From, Debug)]
pub enum Error {
    #[debug("Serde JSON Error: {}", _0)]
    SerdeJson(serde_json::Error),

    #[debug("PostHog Error: {}", _0)]
    PostHog(posthog_rs::Error),

    #[debug("Tokio Join Error: {}", _0)]
    TokioJoin(tokio::task::JoinError),

    #[debug("IO Error: {}", _0)]
    IO(std::io::Error),
}

pub type Result<A> = std::result::Result<A, Error>;
