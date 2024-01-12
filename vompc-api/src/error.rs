use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("reqwest error: {0}")]
    Request(#[from] reqwest::Error),

    #[error("url error: {0}")]
    Url(#[from] url::ParseError),
}
