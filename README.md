# DataSpool

[![Crates.io](https://img.shields.io/crates/v/dataspool-rs.svg)](https://crates.io/crates/dataspool-rs)
[![Documentation](https://docs.rs/dataspool-rs/badge.svg)](https://docs.rs/dataspool-rs)

> **Status: prototype, kept public as a record.** This is the original DataSpool
> prototype from Blackfall Labs' first year. The current DataSpool format lives in
> Databank, which is not open source. This repository is published with its full
> history and is not maintained for production use.

DataSpool bundles many small binary items ("cards") into a single indexed `.spool`
file, so any item can be read with one seek instead of opening thousands of small
files. An optional SQLite database stores metadata and vector embeddings that point
into the spool by byte offset.

The full format is in [SPECIFICATION.md](SPECIFICATION.md). You can write a
compatible implementation from it without reading this code.

## Install

```toml
[dependencies]
dataspool-rs = "0.4"
```

## Usage

### Write a spool

```rust,no_run
use dataspool_rs::SpoolBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = SpoolBuilder::new("data.spool")?;
    builder.add_card(b"first card")?;
    let second = builder.add_card(b"second card")?;
    builder.finalize()?;

    println!("card 1 is {} bytes at offset {}", second.length, second.offset);
    Ok(())
}
```

### Read a spool

```rust,no_run
use dataspool_rs::SpoolReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = SpoolReader::open("data.spool")?;
    for i in 0..reader.card_count() {
        let card = reader.read_card(i)?;
        println!("card {i}: {} bytes", card.len());
    }
    Ok(())
}
```

### Read a spool embedded in another file

Spools can sit inside a larger file, such as an Engram archive. Pass the byte
offset where the spool starts:

```rust,no_run
use dataspool_rs::SpoolReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = SpoolReader::open_embedded("archive.eng", 4096)?;
    let card = reader.read_card(0)?;
    println!("card 0: {} bytes", card.len());
    Ok(())
}
```

### Index cards with embeddings

```rust,no_run
use dataspool_rs::PersistentVectorStore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = PersistentVectorStore::new("index.db", 4)?; // 4-dimensional vectors

    store.add_document_with_spool(
        "doc-1",
        "data.spool",
        "example",
        Some(r#"{"title": "First"}"#),
        Some(17), // spool offset
        Some(10), // card length
        &[0.1, 0.2, 0.3, 0.4],
    )?;

    for hit in store.search(&[0.1, 0.2, 0.3, 0.4], 5)? {
        println!("{} scored {:.3}", hit.doc_ref.id, hit.score);
    }
    Ok(())
}
```

## Format at a glance

```text
[magic "SP01"][version u8 = 1][card_count u32][index_offset u64]   17-byte header
[card 0][card 1] ... [card N-1]                                    raw card bytes
[offset u64, length u32] x N                                       index
```

All integers are little-endian. Offsets are relative to the start of the spool.

## Build and test

```bash
cargo build
cargo test
cargo clippy --all-targets
```

## History

DataSpool was extracted from SAM, an earlier Blackfall Labs project, in December
2025. See [CHANGELOG.md](CHANGELOG.md) for the release history.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this work, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
