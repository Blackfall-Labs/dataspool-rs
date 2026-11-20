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
