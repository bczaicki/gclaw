use thiserror::Error;

#[derive(Error, Debug)]
pub enum GclawError {
    #[error("LLM provider error: {0}")]
    Provider(String),

    #[error("Channel error: {0}")]
    Channel(String),

    #[error("Tool execution error: {0}")]
    ToolExecution(String),

    #[error("Max iterations reached ({0})")]
    MaxIterationsReached(usize),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Memory error: {0}")]
    Memory(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, GclawError>;
