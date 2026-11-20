//! Backward-compatibility tests: files written by older versions of the crate
//! must keep opening and reading correctly.
//!
//! `tests/fixtures/v0.3.0` was written by the published crates.io release 0.3.0;
//! `tests/fixtures/branch-b50cbc1` by this repository at commit b50cbc1. Both were
//! produced by `tests/fixtures/generate.rs.txt`.

use dataspool_rs::{PersistentVectorStore, SpoolReader};
use std::path::{Path, PathBuf};

const FIXTURE_SETS: [&str; 2] = ["v0.3.0", "branch-b50cbc1"];

const CARDS: [&[u8]; 3] = [b"first card", b"", b"third card, a little longer"];

fn fixture(set: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(set)
        .join(name)
}

#[test]
fn old_spools_read() {
    for set in FIXTURE_SETS {
        let mut reader = SpoolReader::open(fixture(set, "basic.spool")).unwrap();
        assert_eq!(reader.card_count(), CARDS.len(), "{set}");
        for (i, card) in CARDS.iter().enumerate() {
            assert_eq!(&reader.read_card(i).unwrap(), card, "{set} card {i}");
        }
        let entry = reader.get_entry(0).unwrap().clone();
        assert_eq!(entry.offset, 17, "{set}");
        assert_eq!(
            reader
                .read_card_at(entry.offset, entry.length as usize)
                .unwrap(),
            CARDS[0]
        );
    }
}

#[test]
fn old_empty_spools_read() {
    for set in FIXTURE_SETS {
        let reader = SpoolReader::open(fixture(set, "empty.spool")).unwrap();
        assert_eq!(reader.card_count(), 0, "{set}");
    }
}

#[test]
fn old_embedded_spools_read() {
    for set in FIXTURE_SETS {
        let mut reader = SpoolReader::open_embedded(fixture(set, "embedded.bin"), 64).unwrap();
        assert_eq!(reader.card_count(), CARDS.len(), "{set}");
        for (i, card) in CARDS.iter().enumerate() {
            assert_eq!(&reader.read_card(i).unwrap(), card, "{set} card {i}");
        }
    }
}

#[test]
fn old_index_databases_read() {
    for set in FIXTURE_SETS {
        // Work on a copy so the committed fixture is never modified.
        let copy = std::env::temp_dir().join(format!(
            "dataspool_compat_{}_{}.db",
            set,
            std::process::id()
        ));
        std::fs::copy(fixture(set, "index.db"), &copy).unwrap();

        let store = PersistentVectorStore::new(&copy, 4).unwrap();
        assert_eq!(store.count().unwrap(), 3, "{set}");

        let hits = store.search(&[1.0, 0.0, 0.0, 0.0], 3).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.doc_ref.id.as_str()).collect();
        assert_eq!(ids, ["card-0", "standalone", "card-2"], "{set}");

        let card0 = &hits[0].doc_ref;
        assert_eq!(card0.file_path, "basic.spool");
        assert_eq!(card0.metadata.as_deref(), Some(r#"{"n":0}"#));
        assert_eq!(card0.spool_offset, Some(17));
        assert_eq!(card0.spool_length, Some(10));

        let standalone = &hits[1].doc_ref;
        assert_eq!(standalone.spool_offset, None);
        assert_eq!(standalone.spool_length, None);

        // The spool offsets in the database point at the right cards.
        let mut reader = SpoolReader::open(fixture(set, "basic.spool")).unwrap();
        let card2 = &hits[2].doc_ref;
        assert_eq!(
            reader
                .read_card_at(
                    card2.spool_offset.unwrap(),
                    card2.spool_length.unwrap() as usize
                )
                .unwrap(),
            CARDS[2]
        );

        drop(store);
        std::fs::remove_file(&copy).unwrap();
    }
}
