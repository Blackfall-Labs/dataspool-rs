//! Data Spooling: Stitch BP Cards into Single File
//!
//! Eliminates filesystem overhead by concatenating all cards into one file.
//! Provides byte offset index for direct random access.

use crate::error::{DataSpoolError, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Magic bytes for spool format
const MAGIC: &[u8; 4] = b"SP01";

/// Spool file format version
const VERSION: u8 = 1;

/// Size of the spool header: magic (4) + version (1) + card count (4) + index offset (8)
const HEADER_LEN: u64 = 17;

/// Size of one index entry: offset (8) + length (4)
const INDEX_ENTRY_LEN: u64 = 12;

/// Whether an index of `card_count` entries starting at `index_offset` lies
/// within a spool of `spool_len` bytes.
fn index_fits(index_offset: u64, card_count: u32, spool_len: u64) -> bool {
    u64::from(card_count)
        .checked_mul(INDEX_ENTRY_LEN)
        .and_then(|index_len| index_offset.checked_add(index_len))
        .is_some_and(|index_end| index_end <= spool_len)
}

/// Whether a card at `offset` with `length` bytes (both relative to the spool
/// start) lies between the end of the header and the start of the index.
fn entry_fits(offset: u64, length: u32, index_offset: u64) -> bool {
    offset >= HEADER_LEN
        && offset
            .checked_add(u64::from(length))
            .is_some_and(|card_end| card_end <= index_offset)
}

/// Convert a card length to its on-disk u32, or fail if it does not fit.
fn checked_card_length(len: usize) -> Result<u32> {
    u32::try_from(len).map_err(|_| {
        DataSpoolError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "card of {} bytes exceeds the spool limit of {} bytes",
                len,
                u32::MAX
            ),
        ))
    })
}

/// Convert a card count to its on-disk u32, or fail if it does not fit.
fn checked_card_count(count: usize) -> Result<u32> {
    u32::try_from(count).map_err(|_| {
        DataSpoolError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "{} cards exceed the spool limit of {} cards",
                count,
                u32::MAX
            ),
        ))
    })
}

/// Entry in the spool index
#[derive(Debug, Clone)]
pub struct SpoolEntry {
    /// Byte offset in spool file where card starts
    pub offset: u64,
    /// Length of card in bytes
    pub length: u32,
}

/// Builds a spool file from individual BP cards
pub struct SpoolBuilder {
    /// Output file
    output: File,
    /// Current write position (after header)
    current_offset: u64,
    /// Index entries
    entries: Vec<SpoolEntry>,
}

impl SpoolBuilder {
    /// Create a new spool builder
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut output = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        // Write header (will update later with index offset)
        output.write_all(MAGIC)?;
        output.write_all(&[VERSION])?;
        output.write_all(&0u32.to_le_bytes())?; // card_count (placeholder)
        output.write_all(&0u64.to_le_bytes())?; // index_offset (placeholder)

        let current_offset = output.stream_position()?;

        Ok(Self {
            output,
            current_offset,
            entries: Vec::new(),
        })
    }

    /// Add a card to the spool
    ///
    /// Returns the entry with offset and length
    pub fn add_card(&mut self, card_data: &[u8]) -> Result<SpoolEntry> {
        let offset = self.current_offset;
        // Check both limits before writing anything for this card.
        let length = checked_card_length(card_data.len())?;
        checked_card_count(self.entries.len() + 1)?;

        // Write card data
        self.output.write_all(card_data)?;
        self.current_offset += card_data.len() as u64;

        let entry = SpoolEntry { offset, length };
        self.entries.push(entry.clone());

        Ok(entry)
    }

    /// Finalize the spool and write index
    pub fn finalize(mut self) -> Result<()> {
        let index_offset = self.current_offset;
        let card_count = checked_card_count(self.entries.len())?;

        // Write index at end of file
        for entry in &self.entries {
            self.output.write_all(&entry.offset.to_le_bytes())?;
            self.output.write_all(&entry.length.to_le_bytes())?;
        }

        // Update header with card count and index offset
        self.output.seek(SeekFrom::Start(5))?; // Skip magic + version
        self.output.write_all(&card_count.to_le_bytes())?;
        self.output.write_all(&index_offset.to_le_bytes())?;

        self.output.sync_all()?;

        Ok(())
    }
}

/// Reads cards from a spool file
pub struct SpoolReader {
    /// Input file
    file: File,
    /// Index entries loaded from spool
    entries: Vec<SpoolEntry>,
}

impl SpoolReader {
    /// Open a spool file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path)?;

        // Read and verify header
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(DataSpoolError::Decompression(
                "Invalid spool magic bytes".into(),
            ));
        }

        let mut version = [0u8; 1];
        file.read_exact(&mut version)?;
        if version[0] != VERSION {
            return Err(DataSpoolError::InvalidFormat);
        }

        let mut card_count_bytes = [0u8; 4];
        file.read_exact(&mut card_count_bytes)?;
        let card_count = u32::from_le_bytes(card_count_bytes);

        let mut index_offset_bytes = [0u8; 8];
        file.read_exact(&mut index_offset_bytes)?;
        let index_offset = u64::from_le_bytes(index_offset_bytes);

        // An unfinished spool still holds the placeholder offset 0; the index
        // can never start inside the header.
        if index_offset < HEADER_LEN {
            return Err(DataSpoolError::InvalidFormat);
        }

        // The index must lie within the file before anything is allocated for it.
        if !index_fits(index_offset, card_count, file.metadata()?.len()) {
            return Err(DataSpoolError::InvalidFormat);
        }

        // Read index
        file.seek(SeekFrom::Start(index_offset))?;
        let mut entries = Vec::with_capacity(card_count as usize);

        for _ in 0..card_count {
            let mut offset_bytes = [0u8; 8];
            file.read_exact(&mut offset_bytes)?;
            let offset = u64::from_le_bytes(offset_bytes);

            let mut length_bytes = [0u8; 4];
            file.read_exact(&mut length_bytes)?;
            let length = u32::from_le_bytes(length_bytes);

            if !entry_fits(offset, length, index_offset) {
                return Err(DataSpoolError::InvalidFormat);
            }

            entries.push(SpoolEntry { offset, length });
        }

        Ok(Self { file, entries })
    }

    /// Open a spool embedded within a larger file at a given byte offset.
    ///
    /// Reads the SP01 header starting at `base_offset`, then adjusts all
    /// internal offsets so that `read_card()` seeks to the correct position
    /// within the host file. This enables direct access to spool data
    /// stitched into an Engram archive without temp extraction.
    pub fn open_embedded<P: AsRef<Path>>(path: P, base_offset: u64) -> Result<Self> {
        let mut file = File::open(path)?;

        // Seek to spool start within the host file.
        file.seek(SeekFrom::Start(base_offset))?;

        // Read and verify header.
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(DataSpoolError::Decompression(
                "Invalid spool magic bytes".into(),
            ));
        }

        let mut version = [0u8; 1];
        file.read_exact(&mut version)?;
        if version[0] != VERSION {
            return Err(DataSpoolError::InvalidFormat);
        }

        let mut card_count_bytes = [0u8; 4];
        file.read_exact(&mut card_count_bytes)?;
        let card_count = u32::from_le_bytes(card_count_bytes);

        let mut index_offset_bytes = [0u8; 8];
        file.read_exact(&mut index_offset_bytes)?;
        let index_offset = u64::from_le_bytes(index_offset_bytes);

        // An unfinished spool still holds the placeholder offset 0; the index
        // can never start inside the header.
        if index_offset < HEADER_LEN {
            return Err(DataSpoolError::InvalidFormat);
        }

        // The index must lie within the embedded region (base_offset to end of
        // file) before anything is allocated for it.
        let region_len = file.metadata()?.len().saturating_sub(base_offset);
        if !index_fits(index_offset, card_count, region_len) {
            return Err(DataSpoolError::InvalidFormat);
        }

        // index_offset is relative to spool start — adjust to host file position.
        let index_start = base_offset
            .checked_add(index_offset)
            .ok_or(DataSpoolError::InvalidFormat)?;
        file.seek(SeekFrom::Start(index_start))?;
        let mut entries = Vec::with_capacity(card_count as usize);

        for _ in 0..card_count {
            let mut offset_bytes = [0u8; 8];
            file.read_exact(&mut offset_bytes)?;
            let offset = u64::from_le_bytes(offset_bytes);

            let mut length_bytes = [0u8; 4];
            file.read_exact(&mut length_bytes)?;
            let length = u32::from_le_bytes(length_bytes);

            if !entry_fits(offset, length, index_offset) {
                return Err(DataSpoolError::InvalidFormat);
            }

            // Adjust card offset to host file position.
            let offset = base_offset
                .checked_add(offset)
                .ok_or(DataSpoolError::InvalidFormat)?;
            entries.push(SpoolEntry { offset, length });
        }

        Ok(Self { file, entries })
    }

    /// Get number of cards in spool
    pub fn card_count(&self) -> usize {
        self.entries.len()
    }

    /// Read a card by index
    pub fn read_card(&mut self, index: usize) -> Result<Vec<u8>> {
        if index >= self.entries.len() {
            return Err(match self.entries.len().checked_sub(1) {
                Some(max) => DataSpoolError::Decompression(format!(
                    "Card index {} out of bounds (max: {})",
                    index, max
                )),
                None => DataSpoolError::Decompression(format!(
                    "Card index {} out of bounds (spool is empty)",
                    index
                )),
            });
        }

        let entry = &self.entries[index];
        self.read_card_at(entry.offset, entry.length as usize)
    }

    /// Read a card at specific offset and length
    pub fn read_card_at(&mut self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(offset))?;

        let mut buffer = vec![0u8; length];
        self.file.read_exact(&mut buffer)?;

        Ok(buffer)
    }

    /// Get entry for a card index
    pub fn get_entry(&self, index: usize) -> Option<&SpoolEntry> {
        self.entries.get(index)
    }

    /// Get all entries
    pub fn entries(&self) -> &[SpoolEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_spool_roundtrip() {
        let temp_path = "test_spool.spool";

        // Create test cards
        let card1 = b"BP01\x01\x08code:apisome data 1";
        let card2 = b"BP01\x01\x08code:apisome data 2 longer";
        let card3 = b"BP01\x01\x08code:apidata 3";

        // Build spool
        {
            let mut builder = SpoolBuilder::new(temp_path).unwrap();
            let entry1 = builder.add_card(card1).unwrap();
            let entry2 = builder.add_card(card2).unwrap();
            let entry3 = builder.add_card(card3).unwrap();

            assert_eq!(entry1.length, card1.len() as u32);
            assert_eq!(entry2.length, card2.len() as u32);
            assert_eq!(entry3.length, card3.len() as u32);

            builder.finalize().unwrap();
        }

        // Read spool
        {
            let mut reader = SpoolReader::open(temp_path).unwrap();
            assert_eq!(reader.card_count(), 3);

            let read1 = reader.read_card(0).unwrap();
            let read2 = reader.read_card(1).unwrap();
            let read3 = reader.read_card(2).unwrap();

            assert_eq!(&read1, card1);
            assert_eq!(&read2, card2);
            assert_eq!(&read3, card3);
        }

        // Cleanup
        fs::remove_file(temp_path).unwrap();
    }

    #[test]
    fn test_spool_direct_access() {
        let temp_path = "test_spool_direct.spool";

        let card = b"BP01\x01\x08code:apitest data";

        {
            let mut builder = SpoolBuilder::new(temp_path).unwrap();
            let entry = builder.add_card(card).unwrap();

            // Store offset and length for later
            let offset = entry.offset;
            let length = entry.length;

            builder.finalize().unwrap();

            // Reopen and read by offset
            let mut reader = SpoolReader::open(temp_path).unwrap();
            let read = reader.read_card_at(offset, length as usize).unwrap();
            assert_eq!(&read, card);
        }

        fs::remove_file(temp_path).unwrap();
    }

    #[test]
    fn test_spool_open_embedded() {
        // Simulate a spool embedded within a larger file by prepending junk bytes.
        let temp_spool = "test_embedded_source.spool";
        let temp_host = "test_embedded_host.bin";

        let card1 = b"BP01\x01\x08code:apicard one data";
        let card2 = b"BP01\x01\x08code:apicard two longer data here";

        // Build a normal spool.
        {
            let mut builder = SpoolBuilder::new(temp_spool).unwrap();
            builder.add_card(card1).unwrap();
            builder.add_card(card2).unwrap();
            builder.finalize().unwrap();
        }

        // Create a host file: [64 bytes junk][spool data][32 bytes junk]
        let spool_bytes = fs::read(temp_spool).unwrap();
        let prefix = vec![0xAA; 64];
        let suffix = vec![0xBB; 32];
        let mut host = Vec::new();
        host.extend_from_slice(&prefix);
        host.extend_from_slice(&spool_bytes);
        host.extend_from_slice(&suffix);
        fs::write(temp_host, &host).unwrap();

        // Open embedded at offset 64.
        {
            let mut reader = SpoolReader::open_embedded(temp_host, 64).unwrap();
            assert_eq!(reader.card_count(), 2);

            let read1 = reader.read_card(0).unwrap();
            let read2 = reader.read_card(1).unwrap();
            assert_eq!(&read1, card1);
            assert_eq!(&read2, card2);
        }

        fs::remove_file(temp_spool).unwrap();
        fs::remove_file(temp_host).unwrap();
    }

    /// A unique scratch path for one test, so tests can run in parallel.
    fn scratch_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "dataspool_test_{}_{}.spool",
            name,
            std::process::id()
        ))
    }

    #[test]
    fn test_read_card_from_empty_spool_is_error() {
        let path = scratch_path("empty_read");
        SpoolBuilder::new(&path).unwrap().finalize().unwrap();

        let mut reader = SpoolReader::open(&path).unwrap();
        assert_eq!(reader.card_count(), 0);
        assert!(matches!(
            reader.read_card(0),
            Err(DataSpoolError::Decompression(_))
        ));

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_unfinished_spool_is_rejected() {
        let path = scratch_path("unfinished");
        {
            let mut builder = SpoolBuilder::new(&path).unwrap();
            builder.add_card(b"written but never indexed").unwrap();
            // Dropped without finalize(): the header keeps index_offset = 0.
        }

        assert!(matches!(
            SpoolReader::open(&path),
            Err(DataSpoolError::InvalidFormat)
        ));

        let host_path = scratch_path("unfinished_host");
        let mut host = vec![0xAA; 64];
        host.extend_from_slice(&fs::read(&path).unwrap());
        fs::write(&host_path, &host).unwrap();
        assert!(matches!(
            SpoolReader::open_embedded(&host_path, 64),
            Err(DataSpoolError::InvalidFormat)
        ));

        fs::remove_file(&path).unwrap();
        fs::remove_file(&host_path).unwrap();
    }

    #[test]
    fn test_corrupt_card_count_is_rejected() {
        let path = scratch_path("huge_count");
        {
            let mut builder = SpoolBuilder::new(&path).unwrap();
            builder.add_card(b"one card").unwrap();
            builder.finalize().unwrap();
        }
        // Overwrite card_count (bytes 5..9) with u32::MAX.
        let mut bytes = fs::read(&path).unwrap();
        bytes[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&path, &bytes).unwrap();

        assert!(matches!(
            SpoolReader::open(&path),
            Err(DataSpoolError::InvalidFormat)
        ));

        let host_path = scratch_path("huge_count_host");
        let mut host = vec![0xAA; 64];
        host.extend_from_slice(&bytes);
        fs::write(&host_path, &host).unwrap();
        assert!(matches!(
            SpoolReader::open_embedded(&host_path, 64),
            Err(DataSpoolError::InvalidFormat)
        ));

        fs::remove_file(&path).unwrap();
        fs::remove_file(&host_path).unwrap();
    }

    /// Build a one-card spool and return its bytes with the first index
    /// entry's offset and length replaced.
    fn spool_with_patched_entry(name: &str, offset: u64, length: u32) -> Vec<u8> {
        let path = scratch_path(name);
        {
            let mut builder = SpoolBuilder::new(&path).unwrap();
            builder.add_card(b"one card").unwrap();
            builder.finalize().unwrap();
        }
        let mut bytes = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();

        let index_offset = u64::from_le_bytes(bytes[9..17].try_into().unwrap()) as usize;
        bytes[index_offset..index_offset + 8].copy_from_slice(&offset.to_le_bytes());
        bytes[index_offset + 8..index_offset + 12].copy_from_slice(&length.to_le_bytes());
        bytes
    }

    #[test]
    fn test_embedded_offset_overflow_is_rejected() {
        let bytes = spool_with_patched_entry("overflow_src", u64::MAX, 8);

        let host_path = scratch_path("overflow_host");
        let mut host = vec![0xAA; 64];
        host.extend_from_slice(&bytes);
        fs::write(&host_path, &host).unwrap();

        assert!(matches!(
            SpoolReader::open_embedded(&host_path, 64),
            Err(DataSpoolError::InvalidFormat)
        ));

        fs::remove_file(&host_path).unwrap();
    }

    #[test]
    fn test_out_of_range_entries_are_rejected() {
        // The one card is 8 bytes at offset 17, so the index starts at 25.
        let cases: [(&str, u64, u32); 4] = [
            ("in_header", 0, 8),
            ("overlaps_index", 20, 8),
            ("past_index", 25, 8),
            ("huge_length", 17, u32::MAX),
        ];

        for (name, offset, length) in cases {
            let bytes = spool_with_patched_entry(name, offset, length);

            let path = scratch_path(name);
            fs::write(&path, &bytes).unwrap();
            assert!(
                matches!(SpoolReader::open(&path), Err(DataSpoolError::InvalidFormat)),
                "open accepted entry {name}"
            );
            fs::remove_file(&path).unwrap();

            let host_path = scratch_path(&format!("{name}_host"));
            let mut host = vec![0xAA; 64];
            host.extend_from_slice(&bytes);
            fs::write(&host_path, &host).unwrap();
            assert!(
                matches!(
                    SpoolReader::open_embedded(&host_path, 64),
                    Err(DataSpoolError::InvalidFormat)
                ),
                "open_embedded accepted entry {name}"
            );
            fs::remove_file(&host_path).unwrap();
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_writer_limits_are_checked() {
        let max = u32::MAX as usize;

        assert_eq!(checked_card_length(0).unwrap(), 0);
        assert_eq!(checked_card_length(max).unwrap(), u32::MAX);
        assert!(matches!(
            checked_card_length(max + 1),
            Err(DataSpoolError::Io(_))
        ));

        assert_eq!(checked_card_count(max).unwrap(), u32::MAX);
        assert!(matches!(
            checked_card_count(max + 1),
            Err(DataSpoolError::Io(_))
        ));
    }
}
