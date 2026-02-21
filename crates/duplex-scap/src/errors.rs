#[derive(Debug, thiserror::Error)]
pub enum DuplexScapError {
    #[error("capture already running")]
    AlreadyRunning,
    #[error("display not found: {0}")]
    DisplayNotFound(u32),
    #[error("platform not supported")]
    Unsupported,
    #[error("internal error: {0}")]
    Internal(String),
}
