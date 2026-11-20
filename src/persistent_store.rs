//! Persistent Vector Store with SQLite
//!
//! Stores documents, embeddings, and HNSW index in SQLite for instant loading

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

// SearchResult moved inline
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
}

/// Document reference (points to compressed file OR spool location)
#[derive(Debug, Clone)]
pub struct DocumentRef {
    pub id: String,
    pub file_path: String,
    pub source: String,
    pub metadata: Option<String>,
    /// If present, this document is in a spool at this byte offset
    pub spool_offset: Option<u64>,
    /// Length of the document in the spool
    pub spool_length: Option<u32>,
}

/// Search result with file reference
#[derive(Debug, Clone)]
pub struct FileSearchResult {
    pub doc_ref: DocumentRef,
    pub score: f32,
}

/// Persistent vector store backed by SQLite
pub struct PersistentVectorStore {
    conn: Arc<Mutex<Connection>>,
    dimension: usize,
}

impl PersistentVectorStore {
    /// Create or open a persistent vector store
    ///
    /// The connection runs with SQLite foreign keys enabled, so the
    /// `ON DELETE CASCADE` on `embeddings.doc_id` (SPECIFICATION.md §6) takes
    /// effect: deleting a document deletes its embedding.
    pub fn new<P: AsRef<Path>>(path: P, dimension: usize) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        // Foreign key enforcement is per connection and off by default in SQLite.
        conn.pragma_update(None, "foreign_keys", true)?;

        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            dimension,
        };

        store.initialize_schema()?;
        Ok(store)
    }

    /// Initialize database schema
    fn initialize_schema(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        // Documents table (stores FILE REFERENCES OR SPOOL OFFSETS, not content)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                file_path TEXT NOT NULL,
                source TEXT NOT NULL,
                metadata TEXT,
                spool_offset INTEGER,
                spool_length INTEGER
            )",
            [],
        )?;

        // Embeddings table (vector stored as blob)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS embeddings (
                doc_id TEXT PRIMARY KEY,
                vector BLOB NOT NULL,
                FOREIGN KEY (doc_id) REFERENCES documents(id) ON DELETE CASCADE
            )",
            [],
        )?;

        // Note: FTS5 removed - content is in compressed files, not in DB
        // Can add FTS later by indexing decompressed content if needed

        Ok(())
    }

    /// Add a document with its embedding and file path
    pub fn add_document(
        &self,
        id: &str,
        file_path: &str,
        source: &str,
        metadata: Option<&str>,
        embedding: &[f32],
    ) -> rusqlite::Result<()> {
        self.add_document_with_spool(id, file_path, source, metadata, None, None, embedding)
    }

    /// Add a document with spool location
    ///
    /// `spool_offset` and `spool_length` are both `Some` (the document is a card
    /// in the spool at `file_path`) or both `None` (`file_path` is a stand-alone
    /// file), as SPECIFICATION.md §6.1 requires. Passing only one of them is an
    /// error and nothing is written.
    #[allow(clippy::too_many_arguments)]
    pub fn add_document_with_spool(
        &self,
        id: &str,
        file_path: &str,
        source: &str,
        metadata: Option<&str>,
        spool_offset: Option<u64>,
        spool_length: Option<u32>,
        embedding: &[f32],
    ) -> rusqlite::Result<()> {
        if embedding.len() != self.dimension {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Expected {} dims, got {}",
                self.dimension,
                embedding.len()
            )));
        }

        if spool_offset.is_some() != spool_length.is_some() {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "spool_offset ({:?}) and spool_length ({:?}) must both be set or both be None",
                spool_offset, spool_length
            )));
        }

        let conn = self.conn.lock().unwrap();

        // Store document file reference with optional spool location
        conn.execute(
            "INSERT OR REPLACE INTO documents (id, file_path, source, metadata, spool_offset, spool_length) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                file_path,
                source,
                metadata,
                spool_offset.map(|o| o as i64),
                spool_length.map(|l| l as i64),
            ],
        )?;

        // Store embedding as blob
        let vector_bytes = embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect::<Vec<u8>>();

        conn.execute(
            "INSERT OR REPLACE INTO embeddings (doc_id, vector) VALUES (?1, ?2)",
            params![id, &vector_bytes],
        )?;

        Ok(())
    }

    /// Search for similar documents using vector similarity
    /// Returns file references, caller must read and decompress files
    ///
    /// Ranking (SPECIFICATION.md §6.3): every stored vector is compared with
    /// the query by cosine similarity, and the top `top_k` documents are
    /// returned, highest similarity first. Precisely:
    ///
    /// - similarity is `dot(q, v) / (|q| * |v|)`, accumulated in `f64` and
    ///   returned as `f32`; it is 0 when either vector has zero magnitude;
    /// - a NaN similarity (from NaN or infinite components) ranks after every
    ///   number;
    /// - documents with equal similarity are ordered by `id`, ascending byte
    ///   order, so the result does not depend on storage order;
    /// - documents without a stored vector are not ranked.
    pub fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> rusqlite::Result<Vec<FileSearchResult>> {
        if query_embedding.len() != self.dimension {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Expected {} dims, got {}",
                self.dimension,
                query_embedding.len()
            )));
        }

        let dimension = self.dimension;
        let conn = self.conn.lock().unwrap();

        // Get all document references with embeddings (including spool offsets)
        let mut stmt = conn.prepare(
            "SELECT d.id, d.file_path, d.source, d.metadata, d.spool_offset, d.spool_length, e.vector
             FROM documents d
             JOIN embeddings e ON d.id = e.doc_id"
        )?;

        let mut results: Vec<(DocumentRef, f32)> = Vec::new();

        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let file_path: String = row.get(1)?;
            let source: String = row.get(2)?;
            let metadata: Option<String> = row.get(3)?;
            let spool_offset: Option<i64> = row.get(4)?;
            let spool_length: Option<i64> = row.get(5)?;
            let vector_bytes: Vec<u8> = row.get(6)?;

            // Every vector in one database shares the store's dimension
            // (SPECIFICATION.md §6.2); comparing vectors of different lengths
            // would silently ignore part of one of them.
            if vector_bytes.len() != dimension * 4 {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Blob,
                    format!(
                        "embedding for document {:?} is {} bytes; dimension {} needs {} bytes",
                        id,
                        vector_bytes.len(),
                        dimension,
                        dimension * 4
                    )
                    .into(),
                ));
            }

            // Deserialize vector
            let embedding: Vec<f32> = vector_bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

            // Calculate cosine similarity
            let similarity = cosine_similarity(query_embedding, &embedding);

            Ok((
                DocumentRef {
                    id,
                    file_path,
                    source,
                    metadata,
                    spool_offset: spool_offset.map(|o| o as u64),
                    spool_length: spool_length.map(|l| l as u32),
                },
                similarity,
            ))
        })?;

        for row in rows {
            let (doc_ref, score) = row?;
            results.push((doc_ref, score));
        }

        // Sort by similarity (descending), NaN scores last, equal scores by
        // document id so the ranking does not depend on SQLite row order.
        results.sort_by(|a, b| score_order(a.1, b.1).then_with(|| a.0.id.cmp(&b.0.id)));

        // Take top_k
        Ok(results
            .into_iter()
            .take(top_k)
            .map(|(doc_ref, score)| FileSearchResult { doc_ref, score })
            .collect())
    }

    /// Delete a document and, through `ON DELETE CASCADE`, its embedding.
    ///
    /// Returns `true` if a document with this id existed.
    pub fn delete_document(&self, id: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute("DELETE FROM documents WHERE id = ?1", params![id])?;
        Ok(deleted > 0)
    }

    /// Get document count
    pub fn count(&self) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

/// Total ordering for search scores: highest first, NaN scores last.
fn score_order(a: f32, b: f32) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => b.total_cmp(&a),
    }
}

/// Calculate cosine similarity between two vectors
///
/// Accumulates in `f64` so that the score, and therefore the ranking, depends
/// as little as possible on summation order and `f32` rounding.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (f64::from(x), f64::from(y));
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        (dot / (norm_a.sqrt() * norm_b.sqrt())) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persistent_store() {
        let store = PersistentVectorStore::new(":memory:", 384).unwrap();

        // Add a document with file reference
        let embedding = vec![0.1f32; 384];
        store
            .add_document(
                "test-1",
                "docs/test-1.cml.bp",
                "test",
                Some("{\"type\": \"test\"}"),
                &embedding,
            )
            .unwrap();

        assert_eq!(store.count().unwrap(), 1);

        // Search - returns file references
        let results = store.search(&embedding, 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].doc_ref.id, "test-1");
        assert_eq!(results[0].doc_ref.file_path, "docs/test-1.cml.bp");
        assert!(results[0].score > 0.99); // Nearly identical vectors
    }

    #[test]
    fn test_persistent_store_with_spool() {
        let store = PersistentVectorStore::new(":memory:", 384).unwrap();

        // Add a document with spool location
        let embedding = vec![0.2f32; 384];
        store
            .add_document_with_spool(
                "test-2",
                "docs/test.spool",
                "test",
                Some("{\"type\": \"spool\"}"),
                Some(1024),
                Some(512),
                &embedding,
            )
            .unwrap();

        assert_eq!(store.count().unwrap(), 1);

        // Search - returns spool offsets
        let results = store.search(&embedding, 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].doc_ref.id, "test-2");
        assert_eq!(results[0].doc_ref.spool_offset, Some(1024));
        assert_eq!(results[0].doc_ref.spool_length, Some(512));
    }

    #[test]
    fn test_search_with_nan_score() {
        let store = PersistentVectorStore::new(":memory:", 4).unwrap();
        let query = [1.0f32, 0.0, 0.0, 0.0];

        store
            .add_document(
                "nan",
                "docs/nan.bp",
                "test",
                None,
                &[f32::NAN, 0.0, 0.0, 0.0],
            )
            .unwrap();
        store
            .add_document("far", "docs/far.bp", "test", None, &[0.0, 1.0, 0.0, 0.0])
            .unwrap();
        store
            .add_document("near", "docs/near.bp", "test", None, &[1.0, 0.0, 0.0, 0.0])
            .unwrap();

        let results = store.search(&query, 3).unwrap();
        let ids: Vec<&str> = results.iter().map(|r| r.doc_ref.id.as_str()).collect();
        assert_eq!(ids, ["near", "far", "nan"]);
        assert!(results[2].score.is_nan());
    }

    #[test]
    fn test_spool_offset_and_length_are_both_or_neither() {
        let store = PersistentVectorStore::new(":memory:", 2).unwrap();
        let v = [1.0f32, 0.0];

        for (offset, length) in [(Some(17), None), (None, Some(8))] {
            assert!(matches!(
                store.add_document_with_spool("half", "a.spool", "t", None, offset, length, &v),
                Err(rusqlite::Error::InvalidParameterName(_))
            ));
        }
        assert_eq!(store.count().unwrap(), 0);

        store
            .add_document_with_spool("both", "a.spool", "t", None, Some(17), Some(8), &v)
            .unwrap();
        store
            .add_document_with_spool("neither", "a.bin", "t", None, None, None, &v)
            .unwrap();
        assert_eq!(store.count().unwrap(), 2);
    }

    fn embedding_count(store: &PersistentVectorStore) -> i64 {
        store
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn test_delete_document_cascades_to_embedding() {
        let store = PersistentVectorStore::new(":memory:", 2).unwrap();
        store
            .add_document("keep", "k.bin", "t", None, &[1.0, 0.0])
            .unwrap();
        store
            .add_document("drop", "d.bin", "t", None, &[0.0, 1.0])
            .unwrap();
        assert_eq!(embedding_count(&store), 2);

        assert!(store.delete_document("drop").unwrap());
        assert!(!store.delete_document("drop").unwrap());
        assert!(!store.delete_document("never-added").unwrap());

        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(embedding_count(&store), 1);
        let hits = store.search(&[0.0, 1.0], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc_ref.id, "keep");
    }

    #[test]
    fn test_foreign_keys_are_enforced() {
        let store = PersistentVectorStore::new(":memory:", 2).unwrap();
        let conn = store.conn.lock().unwrap();
        let on: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(on, 1);
        // An embedding cannot point at a document that does not exist.
        assert!(conn
            .execute(
                "INSERT INTO embeddings (doc_id, vector) VALUES ('ghost', x'00000000')",
                [],
            )
            .is_err());
    }

    #[test]
    fn test_replacing_a_document_keeps_one_embedding() {
        let store = PersistentVectorStore::new(":memory:", 2).unwrap();
        store
            .add_document("doc", "old.bin", "t", None, &[1.0, 0.0])
            .unwrap();
        store
            .add_document("doc", "new.bin", "t", None, &[0.0, 1.0])
            .unwrap();
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(embedding_count(&store), 1);
        let hits = store.search(&[0.0, 1.0], 1).unwrap();
        assert_eq!(hits[0].doc_ref.file_path, "new.bin");
        assert!(hits[0].score > 0.99);
    }

    #[test]
    fn test_stored_vector_of_another_dimension_is_error() {
        let store = PersistentVectorStore::new(":memory:", 2).unwrap();
        store
            .add_document("ok", "ok.bin", "t", None, &[1.0, 0.0])
            .unwrap();
        // A vector of a different dimension written by some other program.
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO documents (id, file_path, source) VALUES ('wide', 'w.bin', 't')",
                [],
            )
            .unwrap();
            let wide: Vec<u8> = [1.0f32, 0.0, 0.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect();
            conn.execute(
                "INSERT INTO embeddings (doc_id, vector) VALUES ('wide', ?1)",
                params![wide],
            )
            .unwrap();
        }

        assert!(matches!(
            store.search(&[1.0, 0.0], 5),
            Err(rusqlite::Error::FromSqlConversionFailure(6, _, _))
        ));

        // Once the odd vector is gone, search works again.
        assert!(store.delete_document("wide").unwrap());
        assert_eq!(store.search(&[1.0, 0.0], 5).unwrap().len(), 1);
    }

    #[test]
    fn test_equal_scores_rank_by_id() {
        let store = PersistentVectorStore::new(":memory:", 2).unwrap();
        // Same direction, different magnitudes: all have similarity 1.
        for (id, v) in [("c", [3.0, 0.0]), ("a", [1.0, 0.0]), ("b", [2.0, 0.0])] {
            store.add_document(id, "f.bin", "t", None, &v).unwrap();
        }
        store
            .add_document("orthogonal", "f.bin", "t", None, &[0.0, 1.0])
            .unwrap();

        let hits = store.search(&[1.0, 0.0], 4).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.doc_ref.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c", "orthogonal"]);
        assert!(hits[..3].iter().all(|h| h.score == 1.0));
        assert_eq!(hits[3].score, 0.0);

        let top2: Vec<String> = store
            .search(&[1.0, 0.0], 2)
            .unwrap()
            .into_iter()
            .map(|h| h.doc_ref.id)
            .collect();
        assert_eq!(top2, ["a", "b"]);
        assert!(store.search(&[1.0, 0.0], 0).unwrap().is_empty());
    }

    #[test]
    fn test_zero_magnitude_has_similarity_zero() {
        let store = PersistentVectorStore::new(":memory:", 2).unwrap();
        store
            .add_document("zero", "z.bin", "t", None, &[0.0, 0.0])
            .unwrap();
        store
            .add_document("opposite", "o.bin", "t", None, &[-1.0, 0.0])
            .unwrap();

        let hits = store.search(&[1.0, 0.0], 2).unwrap();
        assert_eq!(hits[0].doc_ref.id, "zero");
        assert_eq!(hits[0].score, 0.0);
        assert_eq!(hits[1].score, -1.0);

        // A zero query scores every document 0, so the order is by id.
        let hits = store.search(&[0.0, 0.0], 2).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.doc_ref.id.as_str()).collect();
        assert_eq!(ids, ["opposite", "zero"]);
    }
}
