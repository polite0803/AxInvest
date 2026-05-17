use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parse error: {0}")]
    ParseError(String),

    #[error("Rate limit exceeded from {vendor}")]
    RateLimited { vendor: String },

    #[error("Vendor error from {vendor}: {message}")]
    VendorError { vendor: String, message: String },

    #[error("Stock code not found: {0}")]
    NotFound(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
