//! Error types for DataSpool

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum DataSpoolError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Invalid spool format")]
    InvalidFormat,

    #[error("Dictionary error: {0}")]
    Dictionary(String),

    #[error("Decompression error: {0}")]
    Decompression(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

pub type Result<T> = std::result::Result<T, DataSpoolError>;
