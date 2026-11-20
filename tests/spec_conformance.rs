//! Conformance tests against SPECIFICATION.md.
//!
//! `reference_api_signatures` pins the functions that §8 names as the reference
//! implementation, so a signature change breaks the build here. The other
//! tests check the bytes and behavior that §2 to §6 describe.

use dataspool_rs::{
    DataSpoolError, FileSearchResult, PersistentVectorStore, SpoolBuilder, SpoolEntry, SpoolReader,
};
use std::path::PathBuf;

type SpoolResult<T> = dataspool_rs::Result<T>;
type DbResult<T> = rusqlite::Result<T>;

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "dataspool_conformance_{}_{}",
        name,
        std::process::id()
    ))
}

#[test]
#[allow(clippy::type_complexity)]
fn reference_api_signatures() {
    // §3 Writing a spool
    let _: fn(PathBuf) -> SpoolResult<SpoolBuilder> = SpoolBuilder::new::<PathBuf>;
    let _: fn(&mut SpoolBuilder, &[u8]) -> SpoolResult<SpoolEntry> = SpoolBuilder::add_card;
    let _: fn(SpoolBuilder) -> SpoolResult<()> = SpoolBuilder::finalize;

    // §4 Reading a spool
    let _: fn(PathBuf) -> SpoolResult<SpoolReader> = SpoolReader::open::<PathBuf>;
    let _: fn(&SpoolReader) -> usize = SpoolReader::card_count;
    let _: fn(&mut SpoolReader, usize) -> SpoolResult<Vec<u8>> = SpoolReader::read_card;
    let _: fn(&mut SpoolReader, u64, usize) -> SpoolResult<Vec<u8>> = SpoolReader::read_card_at;
    let _: fn(&SpoolReader, usize) -> Option<&SpoolEntry> = SpoolReader::get_entry;
    let _: fn(&SpoolReader) -> &[SpoolEntry] = SpoolReader::entries;

    // §5 Embedded spools
    let _: fn(PathBuf, u64) -> SpoolResult<SpoolReader> = SpoolReader::open_embedded::<PathBuf>;
    let _: fn(&SpoolReader) -> u64 = SpoolReader::base_offset;
    let _: fn(&SpoolReader) -> u64 = SpoolReader::spool_len;

    // §2.1 Index entries
    let entry = SpoolEntry {
        offset: 17u64,
        length: 0u32,
    };
    let _ = (entry.offset, entry.length);

    // §6 Index database
    let _: fn(PathBuf, usize) -> DbResult<PersistentVectorStore> =
        PersistentVectorStore::new::<PathBuf>;
    let _: fn(&PersistentVectorStore, &str, &str, &str, Option<&str>, &[f32]) -> DbResult<()> =
        PersistentVectorStore::add_document;
    let _: fn(
        &PersistentVectorStore,
        &str,
        &str,
        &str,
        Option<&str>,
        Option<u64>,
        Option<u32>,
        &[f32],
    ) -> DbResult<()> = PersistentVectorStore::add_document_with_spool;
    let _: fn(&PersistentVectorStore, &str) -> DbResult<bool> =
        PersistentVectorStore::delete_document;
    let _: fn(&PersistentVectorStore) -> DbResult<usize> = PersistentVectorStore::count;

    // §6.3 Similarity search
    let _: fn(&PersistentVectorStore, &[f32], usize) -> DbResult<Vec<FileSearchResult>> =
        PersistentVectorStore::search;
}

#[test]
fn written_bytes_follow_section_2_and_3() {
    let path = scratch("layout.spool");
    let mut builder = SpoolBuilder::new(&path).unwrap();
    let a = builder.add_card(b"abc").unwrap();
    let b = builder.add_card(b"de").unwrap();
    builder.finalize().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    // Header: magic, version, card_count, index_offset; 17 bytes.
    assert_eq!(&bytes[0..4], b"SP01");
    assert_eq!(bytes[4], 1);
    assert_eq!(u32::from_le_bytes(bytes[5..9].try_into().unwrap()), 2);
    let index_offset = u64::from_le_bytes(bytes[9..17].try_into().unwrap());
    assert_eq!(index_offset, 17 + 3 + 2);

    // Card data immediately after the header, no padding.
    assert_eq!(&bytes[17..22], b"abcde");
    assert_eq!((a.offset, a.length), (17, 3));
    assert_eq!((b.offset, b.length), (20, 2));

    // Index: 12-byte entries {u64 offset, u32 length}, ending the file.
    let index = &bytes[index_offset as usize..];
    assert_eq!(index.len(), 2 * 12);
    assert_eq!(u64::from_le_bytes(index[0..8].try_into().unwrap()), 17);
    assert_eq!(u32::from_le_bytes(index[8..12].try_into().unwrap()), 3);
    assert_eq!(u64::from_le_bytes(index[12..20].try_into().unwrap()), 20);
    assert_eq!(u32::from_le_bytes(index[20..24].try_into().unwrap()), 2);
}

#[test]
fn zero_card_spool_is_17_bytes() {
    let path = scratch("zero.spool");
    SpoolBuilder::new(&path).unwrap().finalize().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), 17);
    assert_eq!(u64::from_le_bytes(bytes[9..17].try_into().unwrap()), 17);
    let reader = SpoolReader::open(&path).unwrap();
    assert_eq!(reader.card_count(), 0);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn reader_follows_section_4_and_5() {
    let path = scratch("read.spool");
    let mut builder = SpoolBuilder::new(&path).unwrap();
    builder.add_card(b"zero").unwrap();
    builder.add_card(b"one").unwrap();
    builder.finalize().unwrap();
    let spool = std::fs::read(&path).unwrap();

    let mut reader = SpoolReader::open(&path).unwrap();
    assert_eq!(reader.read_card(1).unwrap(), b"one");
    // §4 step 5: an index i >= card_count is an error.
    assert!(matches!(
        reader.read_card(2),
        Err(DataSpoolError::Decompression(_))
    ));

    // §5: the same bytes at any base in a host file read the same way, with
    // the same spool-relative offsets.
    for base in [0usize, 1, 4096] {
        let host_path = scratch(&format!("host_{base}.bin"));
        let mut host = vec![0x5Au8; base];
        host.extend_from_slice(&spool);
        host.extend_from_slice(b"host data after the spool");
        std::fs::write(&host_path, &host).unwrap();

        let mut embedded = SpoolReader::open_embedded(&host_path, base as u64).unwrap();
        assert_eq!(embedded.card_count(), 2);
        assert_eq!(embedded.read_card(0).unwrap(), b"zero");
        assert_eq!(embedded.read_card(1).unwrap(), b"one");
        assert_eq!(embedded.entries()[0].offset, reader.entries()[0].offset);
        assert_eq!(embedded.spool_len(), spool.len() as u64);
        std::fs::remove_file(&host_path).unwrap();
    }
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn database_follows_section_6() {
    let db_path = scratch("index.db");
    let _ = std::fs::remove_file(&db_path);
    {
        let store = PersistentVectorStore::new(&db_path, 3).unwrap();
        store
            .add_document_with_spool(
                "card",
                "data.spool",
                "test",
                Some("{}"),
                Some(17),
                Some(4),
                &[1.0, 2.0, 3.0],
            )
            .unwrap();
    }

    // §6, §6.2: schema as specified, vectors as little-endian f32.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let vector: Vec<u8> = conn
        .query_row(
            "SELECT vector FROM embeddings WHERE doc_id = 'card'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let expected: Vec<u8> = [1.0f32, 2.0, 3.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    assert_eq!(vector, expected);
    let (offset, length): (i64, i64) = conn
        .query_row(
            "SELECT spool_offset, spool_length FROM documents WHERE id = 'card'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((offset, length), (17, 4));
    drop(conn);
    std::fs::remove_file(&db_path).unwrap();
}
