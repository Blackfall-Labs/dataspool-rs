# DataSpool - Efficient Data Bundling System

**DataSpool** is a high-performance data bundling library that eliminates filesystem overhead by concatenating multiple items (cards, images, binary blobs) into a single indexed `.spool` file with SQLite-based metadata and vector embeddings.

## Features

- 📦 **Efficient Bundling** - Single file storage with byte-offset index
- 🚀 **Random Access** - Direct seeks to any item without scanning
- 🔍 **Vector Search** - SQLite-backed embeddings for semantic retrieval
- 📊 **Metadata Storage** - Rich metadata with full-text search (FTS5)
- 🔄 **Multiple Variants** - Cards (compressed CML), images, binary blobs
- 💾 **Compact Format** - Minimal overhead, optimal for thousands of items
- 🔐 **Type-Safe** - Rust type safety with serde serialization

## Quick Start

### Writing a Spool

```rust
use dataspool::{SpoolBuilder, SpoolEntry};

// Create spool builder
let mut builder = SpoolBuilder::new();

// Add entries
builder.add_entry(SpoolEntry {
    id: "item1".to_string(),
    data: b"Item 1 data".to_vec(),
});

builder.add_entry(SpoolEntry {
    id: "item2".to_string(),
    data: b"Item 2 data".to_vec(),
});

// Write to file
builder.write_to_file("data.spool")?;
```

### Reading from a Spool

```rust
use dataspool::SpoolReader;

// Open spool
let reader = SpoolReader::open("data.spool")?;

// Read specific entry
let data = reader.read_entry(0)?; // Read first entry
println!("Item 0: {} bytes", data.len());

// Iterate entries
for (index, entry) in reader.iter_entries().enumerate() {
    let data = entry?;
    println!("Item {}: {} bytes", index, data.len());
}
```

### Persistent Vector Store

```rust
use dataspool::{PersistentVectorStore, DocumentRef};

// Create persistent store
let mut store = PersistentVectorStore::new("vectors.db")?;

// Add document with embedding
let doc_ref = DocumentRef {
    id: "doc1".to_string(),
    file_path: "data.spool".to_string(),
    source: "web-scrape".to_string(),
    metadata: Some(r#"{"title": "Example"}"#.to_string()),
    spool_offset: Some(0),
    spool_length: Some(1024),
};

let embedding = vec![0.1, 0.2, 0.3, 0.4]; // Example embedding vector
store.add_document_ref(&doc_ref, &embedding)?;

// Search by vector similarity
let query_vector = vec![0.15, 0.25, 0.35, 0.45];
let results = store.search(&query_vector, 10)?;

for result in results {
    println!("ID: {}, Score: {:.3}", result.id, result.score);
}
```

## Spool Format

### File Structure

```
.spool file:
┌─────────────────────────────┐
│ Magic: "SP01"       (4 bytes)│
│ Version: 1          (1 byte) │
│ Card Count          (4 bytes)│
│ Index Offset        (8 bytes)│
├─────────────────────────────┤
│ Card 0 Data                  │
│ Card 1 Data                  │
│ ...                          │
│ Card N Data                  │
├─────────────────────────────┤
│ Index:                       │
│   [offset0, len0]            │
│   [offset1, len1]            │
│   ...                        │
│   [offsetN, lenN]            │
└─────────────────────────────┘

.db file (SQLite):
┌─────────────────────────────┐
│ documents table:             │
│   - id                       │
│   - file_path                │
│   - source                   │
│   - metadata (JSON)          │
│   - spool_offset             │
│   - spool_length             │
├─────────────────────────────┤
│ embeddings table:            │
│   - doc_id                   │
│   - vector (BLOB)            │
└─────────────────────────────┘
```

### Format Details

- **Magic Number**: `SP01` (4 bytes) - Identifies spool format
- **Version**: `1` (1 byte) - Format version
- **Card Count**: Number of entries in spool (u32)
- **Index Offset**: Byte offset where index starts (u64)
- **Index**: Array of `[offset: u64, length: u64]` pairs

## Architecture

```
┌─────────────┐
│  DataCard   │ (compressed CML)
└──────┬──────┘
       │
       v
┌─────────────┐     ┌──────────────┐
│ SpoolBuilder│────>│  .spool file │
└─────────────┘     └──────────────┘
                           │
                           v
                    ┌──────────────┐
                    │ SpoolReader  │
                    └──────┬───────┘
                           │
       ┌───────────────────┴───────────────────┐
       v                                       v
┌──────────────────┐                  ┌────────────────┐
│ PersistentVector │                  │  .db (SQLite)  │
│      Store       │<─────────────────│  - documents   │
└──────────────────┘                  │  - embeddings  │
                                      └────────────────┘
```

## Use Cases

### 1. Knowledge Base Archival

Bundle thousands of documentation cards into a single file:

```rust
// Build spool from cards
let mut builder = SpoolBuilder::new();
for card in documentation_cards {
    builder.add_entry(SpoolEntry {
        id: card.id,
        data: card.compressed_data,
    });
}
builder.write_to_file("rust-stdlib.spool")?;

// Create vector index
let mut store = PersistentVectorStore::new("rust-stdlib.db")?;
for (i, embedding) in embeddings.iter().enumerate() {
    store.add_document_ref(&DocumentRef {
        id: format!("card_{}", i),
        file_path: "rust-stdlib.spool".to_string(),
        spool_offset: Some(offsets[i]),
        spool_length: Some(lengths[i]),
        ...
    }, embedding)?;
}
```

### 2. Image Dataset Storage

Store image collections with metadata:

```rust
let mut builder = SpoolBuilder::new();

for image_path in image_paths {
    let data = std::fs::read(&image_path)?;
    builder.add_entry(SpoolEntry {
        id: image_path.file_stem().unwrap().to_string(),
        data,
    });
}

builder.write_to_file("images.spool")?;
```

### 3. Binary Blob Archival

Archive arbitrary binary data with fast random access:

```rust
// Write blobs
let mut builder = SpoolBuilder::new();
builder.add_entry(SpoolEntry { id: "blob1".into(), data: blob1 });
builder.add_entry(SpoolEntry { id: "blob2".into(), data: blob2 });
builder.write_to_file("blobs.spool")?;

// Random access read
let reader = SpoolReader::open("blobs.spool")?;
let blob1_data = reader.read_entry(0)?; // Direct access, no scan
```

## Performance

**Benchmark results** (3,309 items, Rust stdlib documentation):

| Operation | Time | Notes |
|-----------|------|-------|
| Build spool | ~200ms | Writing all items + index |
| Read single item | <1ms | Direct byte offset seek |
| Read all items | ~50ms | Sequential read |
| SQLite insert (1 doc) | ~0.5ms | With embedding |
| Vector search (10 results) | ~5ms | Cosine similarity + index |

## Comparison to Alternatives

| Approach | Read Speed | Storage Overhead | Random Access |
|----------|------------|------------------|---------------|
| Individual files | Slow (3,309 inodes) | High (4KB/file) | Yes |
| tar archive | Slow (must scan) | Low | No |
| zip archive | Fast | Medium | Yes |
| **DataSpool** | **Fast** | **Minimal** | **Yes** |

### DataSpool Advantages

- **No compression overhead** - Items pre-compressed by BytePunch
- **Instant random access** - Direct byte offset, no central directory scan
- **Integrated vector DB** - Semantic search without external tools
- **Minimal format** - Simple binary format, easy to parse

## Dependencies

```toml
[dependencies]
dataspool = "0.1.0"
bytepunch = "0.1.0"  # For compressed item decompression
```

### Dependency Graph

```
dataspool
├── bytepunch (compression)
├── rusqlite (SQLite database)
├── serde (serialization)
└── thiserror (error handling)
```

## Features

### Default

Basic spool read/write and persistent vector store.

### Optional: `async`

Async APIs for non-blocking I/O:

```toml
[dependencies]
dataspool = { version = "0.1.0", features = ["async"] }
```

```rust
use dataspool::async_api::AsyncSpoolReader;

let reader = AsyncSpoolReader::open("data.spool").await?;
let data = reader.read_entry(0).await?;
```

## Installation

Add to `Cargo.toml`:

```toml
[dependencies]
dataspool = "0.1.0"
```

Or with async support:

```toml
[dependencies]
dataspool = { version = "0.1.0", features = ["async"] }
```

## Testing

```bash
# Run all tests
cargo test

# Run with logging
RUST_LOG=debug cargo test

# Test specific module
cargo test spool
cargo test persistent_store
```

## Examples

See `examples/` directory:

- `build_spool.rs` - Build a spool from files
- `read_spool.rs` - Read entries from a spool
- `vector_search.rs` - Semantic search with embeddings

Run with:

```bash
cargo run --example build_spool
cargo run --example read_spool
cargo run --example vector_search
```

## Roadmap

- [ ] Image-based spools with EXIF metadata
- [ ] Audio/video spool variants
- [ ] Compression statistics per entry
- [ ] Incremental spool updates (append-only mode)
- [ ] Multi-threaded indexing
- [ ] Memory-mapped I/O for large spools
- [ ] Network streaming protocol

## History

Extracted from the [SAM (Societal Advisory Module)](https://github.com/Blackfall-Labs/sam) project, where it provides the spool bundling system for knowledge base archival.

## License

MIT - See [LICENSE](LICENSE) for details.

## Author

Magnus Trent <magnus@blackfall.dev>

## Links

- **GitHub:** https://github.com/Blackfall-Labs/dataspool-rs
- **Docs:** https://docs.rs/dataspool
- **Crates.io:** https://crates.io/crates/dataspool
- **SAM Project:** https://github.com/Blackfall-Labs/sam
