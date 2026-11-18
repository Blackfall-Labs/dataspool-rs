//! # DataSpool: Efficient Data Bundling System
//!
//! DataSpool eliminates filesystem overhead by concatenating multiple items
//! (cards, images, etc.) into a single indexed file with SQLite-based metadata.
//!
//! ## Features
//!
//! - **Spool Format**: Single file with byte-offset index
//! - **Vector Database**: SQLite-based embeddings and metadata
//! - **Random Access**: Direct byte offset seeks, no scanning
//! - **Multiple Variants**: Cards, Images (future), Binary blobs
//!
//! ## Format
//!
//! ```text
//! .spool file:
//!   [Magic: SP01][Version: 1]
//!   [Card Count: u32][Index Offset: u64]
//!   [Card 0 data...][Card 1 data...][Card N data...]
//!   [Index: offset0, len0, offset1, len1, ...]
//!
//! .db file (SQLite):
//!   - documents table (id, file_path, source, metadata, spool_offset, spool_length)
//!   - embeddings table (doc_id, vector BLOB)
//! ```

pub mod error;
pub mod persistent_store;
pub mod spool;

pub use error::{DataSpoolError, Result};
pub use persistent_store::{DocumentRef, FileSearchResult, PersistentVectorStore};
pub use spool::{SpoolBuilder, SpoolEntry, SpoolReader};

// Compile the README examples as doctests so they stay in sync with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
