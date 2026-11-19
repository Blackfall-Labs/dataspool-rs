# DataSpool Format Specification

**Format:** SP01 (spool file) plus an optional SQLite index database
**Format version:** 1
**Library version:** dataspool-rs 0.3.x
**Status:** Stable prototype specification

This document describes the DataSpool on-disk format completely enough to write a
compatible reader or writer without reading the Rust source. All integers are
little-endian. There is no padding or alignment anywhere in the file.

---

## 1. Overview

A DataSpool bundles many small binary items ("cards") into one file so that a
program can read any item with a single seek instead of opening thousands of small
files. It has two parts:

1. **The spool file** (`.spool`, magic `SP01`): a header, the raw card bytes laid
   end to end, and an index of `(offset, length)` pairs.
2. **The index database** (`.db`, SQLite, optional): document metadata and vector
   embeddings that point into the spool by byte offset.

The spool format is independent of the database. A reader that only needs card
bytes by position can ignore the database entirely.

## 2. Spool file layout

```text
offset  size  field
------  ----  ---------------------------------------------
0       4     magic          ASCII "SP01" (0x53 0x50 0x30 0x31)
4       1     version        u8, must be 1
5       4     card_count     u32, number of cards
9       8     index_offset   u64, byte offset of the index, relative to spool start
17      ...   card data      card 0 bytes, card 1 bytes, ... card N-1 bytes
index   12*N  index          N entries of { offset: u64, length: u32 }
```

The header is exactly **17 bytes**. Card data begins immediately after it, at
offset 17.

### 2.1 Index entries

Each index entry is 12 bytes:

| Field    | Type | Meaning                                                    |
|----------|------|------------------------------------------------------------|
| `offset` | u64  | Byte offset of the card, relative to the start of the spool |
| `length` | u32  | Length of the card in bytes                                 |

Entries appear in card order: entry `i` describes card `i`. A single card is
therefore limited to 4 GiB (`u32::MAX` bytes).

### 2.2 Card contents

The spool does not interpret card bytes. Cards are opaque blobs; the format puts no
constraint on their encoding, compression or size beyond the `u32` length limit.
Applications decide what a card is (a compressed document, an image, a record).

## 3. Writing a spool

A conforming writer:

1. Writes the 17-byte header with `card_count = 0` and `index_offset = 0` as
   placeholders.
2. Appends each card's bytes in order, recording `(offset, length)` for each, where
   `offset` is the position at which that card's first byte was written.
3. Records `index_offset` as the current position (the byte after the last card).
4. Writes all index entries.
5. Seeks back to offset 5 and overwrites `card_count` and `index_offset` with their
   real values.
6. Flushes the file to stable storage.

A spool with zero cards is valid: the header is followed directly by an empty index,
and `index_offset` is 17.

A file whose header still holds the placeholder values (`card_count = 0`,
`index_offset = 0`) was not finalized and must not be treated as a valid spool with
data in it.

## 4. Reading a spool

A conforming reader:

1. Reads 4 bytes and rejects the file unless they equal `SP01`.
2. Reads 1 byte and rejects the file unless it equals `1`.
3. Reads `card_count` (u32) and `index_offset` (u64).
4. Seeks to `index_offset` and reads `card_count` index entries.
5. To read card `i`, seeks to `entries[i].offset` and reads `entries[i].length`
   bytes. An index `i >= card_count` is an error.

## 5. Embedded spools

A spool may be stored inside a larger host file, for example inline in an Engram
archive. Because every offset in the spool is relative to the spool's own first
byte, the spool can be placed at any position `base` in the host file without
rewriting it.

To read an embedded spool, a reader is given the host file and `base`:

1. Seeks to `base` and reads the header as in section 4.
2. Seeks to `base + index_offset` and reads the index.
3. Reads card `i` from `base + entries[i].offset`.

`dataspool-rs` exposes this as `SpoolReader::open_embedded(path, base)`.

## 6. Index database (optional)

The index database is a SQLite 3 file with two tables. It stores references to
content, never the content itself.

```sql
CREATE TABLE IF NOT EXISTS documents (
    id           TEXT PRIMARY KEY,
    file_path    TEXT NOT NULL,
    source       TEXT NOT NULL,
    metadata     TEXT,
    spool_offset INTEGER,
    spool_length INTEGER
);

CREATE TABLE IF NOT EXISTS embeddings (
    doc_id TEXT PRIMARY KEY,
    vector BLOB NOT NULL,
    FOREIGN KEY (doc_id) REFERENCES documents(id) ON DELETE CASCADE
);
```

### 6.1 `documents`

| Column         | Meaning                                                                  |
|----------------|--------------------------------------------------------------------------|
| `id`           | Application-chosen unique document ID                                    |
| `file_path`    | Path of the file holding the document (a spool or a standalone file)     |
| `source`       | Free-form label for where the document came from                         |
| `metadata`     | Optional application metadata; JSON by convention                        |
| `spool_offset` | If set, the document is a card in a spool at this byte offset            |
| `spool_length` | If set, the card's length in bytes                                       |

`spool_offset` and `spool_length` are both set or both null. When they are null,
`file_path` names a standalone file.

### 6.2 `embeddings`

`vector` is the embedding packed as consecutive little-endian IEEE-754 `f32`
values, so its length in bytes is `4 * dimension`. The dimension is not stored in
the database; every vector in one database must share the dimension the
application chose when creating it.

### 6.3 Similarity search

The reference implementation ranks documents by cosine similarity between the query
vector and each stored vector, highest first, and returns the top `k`. A vector with
zero magnitude has similarity 0. Implementations may use any index structure as long
as they return the same ranking.

## 7. Versioning

The `version` byte identifies the spool layout. This document defines version `1`.
A reader must reject versions it does not understand. Any change to the header or
index layout requires a new version number.

## 8. Reference implementation

`dataspool-rs` 0.3.x implements this specification:

- `SpoolBuilder::new`, `add_card`, `finalize` (section 3)
- `SpoolReader::open`, `read_card`, `read_card_at` (section 4)
- `SpoolReader::open_embedded` (section 5)
- `PersistentVectorStore` (section 6)
