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
//!
//! ## Specification
//!
//! `SPECIFICATION.md` is the normative description of the format. Its
//! reference implementation (§8) maps onto this crate as follows:
//!
//! | Section | API |
//! |---------|-----|
//! | §2.1 Index entries | [`SpoolEntry`] `{ offset: u64, length: u32 }`, offsets relative to the spool start |
//! | §3 Writing a spool | [`SpoolBuilder::new`], [`SpoolBuilder::add_card`], [`SpoolBuilder::finalize`] |
//! | §4 Reading a spool | [`SpoolReader::open`], [`SpoolReader::card_count`], [`SpoolReader::read_card`], [`SpoolReader::read_card_at`], [`SpoolReader::get_entry`], [`SpoolReader::entries`] |
//! | §5 Embedded spools | [`SpoolReader::open_embedded`], [`SpoolReader::base_offset`], [`SpoolReader::spool_len`] |
//! | §6 Index database | [`PersistentVectorStore::new`], [`PersistentVectorStore::add_document`], [`PersistentVectorStore::add_document_with_spool`], [`PersistentVectorStore::delete_document`], [`PersistentVectorStore::count`] |
//! | §6.3 Similarity search | [`PersistentVectorStore::search`] returning [`FileSearchResult`] |
//!
//! Errors are listed in the [`error`] module.

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
