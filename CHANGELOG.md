# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/).

## [0.4.0] - Unreleased

Documentation, licensing and cleanup ahead of opening the repository with its full
history. The SP01 on-disk format and the index database schema did not change;
spools and databases written by 0.3.0 still open (see `tests/compat_fixtures.rs`).

The version moves to 0.4.0 because three changes are visible to callers (details
under "Changed (behavior)" and "Fixed"):

- A `SpoolReader` opened with `open_embedded` returns spool-relative offsets from
  `entries()`, `get_entry()` and `read_card_at()` (they were host-file positions).
- A spool with the wrong magic bytes is rejected as `DataSpoolError::InvalidFormat`
  (was `DataSpoolError::Decompression`).
- `SpoolReader::read_card_at` checks that the range lies within the card data and
  returns `DataSpoolError::Decompression` otherwise.

### Added
- `SPECIFICATION.md`, a complete description of the SP01 spool format and the index
  database, sufficient to build a compatible implementation.
- `LICENSE-APACHE`: the crate is now dual licensed under MIT OR Apache-2.0.
- Compatibility fixtures in `tests/fixtures/` written by the published 0.3.0 release
  and by this branch before the specification work, with tests that read them.
- `tests/spec_conformance.rs`: pins the signatures of the reference-implementation
  functions SPECIFICATION.md §8 names and checks written bytes and reads against
  §2 to §6. The crate rustdoc maps each specification section to its API.

### Changed
- README rewritten so every example matches the real API.
- Removed the unused `bytepunch-rs` dependency. The crate now builds from crates.io
  alone.

### Fixed
- All Clippy warnings.
- `SpoolReader::read_card` on an empty spool returns an error instead of panicking.
- An unfinished spool (index offset still 0) is rejected as an invalid format instead
  of opening as an empty spool.
- A corrupt card count no longer aborts the process on a huge allocation: the index must
  fit in the file (or embedded region) or the spool is rejected as an invalid format.
- `SpoolReader::open_embedded` no longer overflows when adding the base offset; an
  overflow is rejected as an invalid format.
- Index entries are validated on open: a card must lie between the header and the
  index, otherwise the spool is rejected as an invalid format.
- `SpoolBuilder` returns an error instead of silently truncating a card longer than
  `u32::MAX` bytes or a card count above `u32::MAX`; nothing is written for that card.
- `PersistentVectorStore::search` no longer panics when a score is NaN; NaN scores
  are ranked last.
- A spool whose header or index is cut short (including an embedded spool whose base
  offset is at or past the end of the host file) is rejected as
  `DataSpoolError::InvalidFormat` instead of `DataSpoolError::Io(UnexpectedEof)`.
- A spool with the wrong magic bytes is rejected as `DataSpoolError::InvalidFormat`
  (was `DataSpoolError::Decompression`), the same as a wrong version. The rustdoc of
  the `error` module now has a table of which situation returns which error.

### Added (API)
- `SpoolReader::spool_len()`: the spool's total length (`index_offset + 12 *
  card_count`), so a host file can find where an embedded spool ends.
- `PersistentVectorStore::delete_document()`: deletes a document; its embedding
  goes with it through the schema's `ON DELETE CASCADE`.

### Changed (behavior)
- A `SpoolReader` opened with `open_embedded` keeps the index's spool-relative
  offsets, as SPECIFICATION.md §2.1 and §5 define them: `entries()`, `get_entry()`
  and `read_card_at()` use the same offsets as a stand-alone copy of the spool (they
  were host-file positions before). The new `SpoolReader::base_offset()` returns
  where the spool starts in the host file.
- `SpoolReader::read_card_at` checks that the range lies within the card data and
  returns `DataSpoolError::Decompression` otherwise, instead of reading the header,
  the index, past the end of the file, or allocating an arbitrary length.
- `PersistentVectorStore::add_document_with_spool` rejects a spool offset without a
  length or a length without an offset (`rusqlite::Error::InvalidParameterName`);
  SPECIFICATION.md §6.1 requires both or neither.
- `PersistentVectorStore` turns on SQLite foreign keys for its connection, so the
  `ON DELETE CASCADE` in the schema takes effect and an embedding cannot point at a
  missing document.
- `PersistentVectorStore::search` returns an error
  (`rusqlite::Error::FromSqlConversionFailure`) when a stored vector's length does not
  match the store's dimension, instead of silently comparing vectors of different
  lengths. SPECIFICATION.md §6.2: every vector in one database shares one dimension.
- `PersistentVectorStore::search` ranks deterministically: cosine similarity is
  accumulated in `f64`, and documents with equal scores are ordered by id instead of
  by SQLite row order. The exact ranking rule is in the rustdoc of `search`.
- `PersistentVectorStore::add_document` and `add_document_with_spool` write the
  document row and its embedding in one transaction, so a failure cannot leave a
  document without its embedding.
- Spool offsets are stored in the database as non-negative INTEGERs:
  `add_document_with_spool` rejects an offset above `i64::MAX`
  (`rusqlite::Error::InvalidParameterName`) instead of storing it as a negative
  number. Negative offsets already stored by 0.3.0 still read back as the `u64` that
  was written. A stored `spool_length` outside the `u32` range is an error
  (`rusqlite::Error::FromSqlConversionFailure`) instead of being truncated.

### Documentation
- The `persistent_store` module no longer claims an HNSW index. Search is an exact
  cosine-similarity scan, which is what SPECIFICATION.md §6.3 ranks by.
- The crate docs no longer list card "variants" (images, blobs). Cards are opaque
  byte strings, as SPECIFICATION.md §2.2 says; any content can be a card.
- The rustdoc of the `persistent_store` module has a table of which situation returns
  which error.

### Removed
- `dataspool.md`, an out-of-date copy of the README.

## [0.3.0] - 2026-03-11

### Added
- `SpoolReader::open_embedded()` to read a spool stored inside a larger file at a
  byte offset, without extracting it.

## [0.2.0] - 2026-01-08

### Changed
- Updated to `rusqlite` 0.32.

## [0.1.0] - 2025-12-21

### Added
- Initial release, extracted from SAM.
- `SpoolBuilder` for creating `.spool` files.
- `SpoolReader` for reading cards by index through the byte-offset index.
- `PersistentVectorStore` for SQLite-backed metadata and embeddings.

[0.4.0]: https://crates.io/crates/dataspool-rs/0.4.0
[0.3.0]: https://crates.io/crates/dataspool-rs/0.3.0
[0.2.0]: https://crates.io/crates/dataspool-rs/0.2.0
[0.1.0]: https://crates.io/crates/dataspool-rs/0.1.0
