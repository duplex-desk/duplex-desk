use thiserror::Error;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("platform not supported")]
    Unsupported,

    #[error("init failed: {0}")]
    InitFailed(String),

    #[error("inject failed: {0}")]
    InjectFailed(String),

    #[error("failed to create CGEvent")]
    EventCreateFailed,
}
