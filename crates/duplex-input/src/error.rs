use thiserror::Error;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("platform not supported")]
    Unsupported,

    #[error("init failed: {0}")]
    InitFailed(String),

    #[error("failed to create CGEvent")]
    EventCreateFailed,
}
