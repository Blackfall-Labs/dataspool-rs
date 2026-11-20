# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

Documentation, licensing and cleanup ahead of opening the repository with its full
history. The crate version and the on-disk format are unchanged from 0.3.0.

### Added
- `SPECIFICATION.md`, a complete description of the SP01 spool format and the index
  database, sufficient to build a compatible implementation.
- `LICENSE-APACHE`: the crate is now dual licensed under MIT OR Apache-2.0.
- Compatibility fixtures in `tests/fixtures/` written by the published 0.3.0 release
  and by this branch before the specification work, with tests that read them.

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

[Unreleased]: https://github.com/blackfall-labs/dataspool-rs/commits/main
[0.3.0]: https://crates.io/crates/dataspool-rs/0.3.0
[0.2.0]: https://crates.io/crates/dataspool-rs/0.2.0
[0.1.0]: https://crates.io/crates/dataspool-rs/0.1.0
