# Changelog

## [0.1.0] - 2025-12-21

### Added
- Initial release extracted from SAM project
- SpoolBuilder for creating .spool files
- SpoolReader for reading from .spool files with byte-offset indexing
- PersistentVectorStore for SQLite-based metadata + embeddings
- Support for bundling compressed cards into single file
- Random access via byte offsets (no scanning required)

### Notes
- Extracted from Blackfall Labs SAM project
- Designed for extensibility (future: image spools, binary spools)
- Used to bundle 3,309 Rust stdlib cards into 1.2MB .spool + 7.3MB .db
