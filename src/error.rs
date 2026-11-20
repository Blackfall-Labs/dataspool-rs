//! Error types for DataSpool
//!
//! # Error table
//!
//! Spool operations ([`SpoolBuilder`](crate::SpoolBuilder) and
//! [`SpoolReader`](crate::SpoolReader)) report failures as follows.
//!
//! | Situation                                                        | Error                                  |
//! |------------------------------------------------------------------|----------------------------------------|
//! | First 4 bytes are not `SP01` (SPECIFICATION.md §4 step 1)          | [`DataSpoolError::InvalidFormat`]      |
//! | Version byte is not `1` (§4 step 2, §7)                            | [`DataSpoolError::InvalidFormat`]      |
//! | Header or index cut short by the end of the file or embedded region | [`DataSpoolError::InvalidFormat`]   |
//! | Unfinished spool: `index_offset` still the placeholder 0 (§3)      | [`DataSpoolError::InvalidFormat`]      |
//! | Index does not fit between `index_offset` and the end of the spool | [`DataSpoolError::InvalidFormat`]      |
//! | Index entry pointing outside the card data (§2.1)                  | [`DataSpoolError::InvalidFormat`]      |
//! | Embedded spool whose base plus an offset overflows `u64` (§5)      | [`DataSpoolError::InvalidFormat`]      |
//! | Card index `i >= card_count` (§4 step 5)                           | [`DataSpoolError::Decompression`]      |
//! | Writer given a card over `u32::MAX` bytes or more than `u32::MAX` cards (§2.1) | [`DataSpoolError::Io`] with [`ErrorKind::InvalidInput`](std::io::ErrorKind::InvalidInput) |
//! | Any failure of the underlying file (open, seek, read, write, sync) | [`DataSpoolError::Io`]                 |
//!
//! [`DataSpoolError::Dictionary`] and [`DataSpoolError::Serialization`] are not
//! produced by this crate; they remain for applications that wrap card
//! encoding in the same error type.
//!
//! The index database ([`PersistentVectorStore`](crate::PersistentVectorStore))
//! reports `rusqlite::Error`; its error table is in the
//! [`persistent_store`](crate::persistent_store) module documentation.

use std::io;

/// Errors returned by spool operations. See the [module documentation](self)
/// for which situation produces which variant.
#[derive(Debug, thiserror::Error)]
pub enum DataSpoolError {
    /// The underlying file failed, or a writer was given input the format
    /// cannot represent (`ErrorKind::InvalidInput`).
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// The bytes are not a valid, finished SP01 spool of a supported version.
    #[error("Invalid spool format")]
    InvalidFormat,

    /// Not produced by this crate.
    #[error("Dictionary error: {0}")]
    Dictionary(String),

    /// A requested card does not exist in the spool.
    #[error("Decompression error: {0}")]
    Decompression(String),

    /// Not produced by this crate.
    #[error("Serialization error: {0}")]
    Serialization(String),
}

pub type Result<T> = std::result::Result<T, DataSpoolError>;
