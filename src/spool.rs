//! Data Spooling: Stitch Cards into a Single File
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

/// Entry in the spool index (SPECIFICATION.md §2.1)
#[derive(Debug, Clone)]
pub struct SpoolEntry {
    /// Byte offset where the card starts, relative to the first byte of the
    /// spool. For an embedded spool this is not a position in the host file:
    /// the card is at `base_offset + offset` there.
    pub offset: u64,
    /// Length of card in bytes
    pub length: u32,
}

/// Builds a spool file from individual cards (opaque byte strings)
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
///
/// Every offset the reader takes or returns ([`SpoolEntry::offset`],
/// [`read_card_at`](Self::read_card_at)) is relative to the start of the
/// spool, as the index stores it, whether the spool is a file of its own or
/// embedded in a host file. Only [`base_offset`](Self::base_offset) is a
/// host-file position.
pub struct SpoolReader {
    /// Input file
    file: File,
    /// Position of the spool's first byte in `file` (0 unless embedded)
    base: u64,
    /// Offset of the index, relative to the spool start
    index_offset: u64,
    /// Index entries loaded from spool, offsets relative to the spool start
    entries: Vec<SpoolEntry>,
}

/// Map an unexpected end of file to [`DataSpoolError::InvalidFormat`]: a spool
/// whose header or index is cut short is a malformed spool, not an I/O failure.
fn truncated_is_invalid(err: std::io::Error) -> DataSpoolError {
    if err.kind() == std::io::ErrorKind::UnexpectedEof {
        DataSpoolError::InvalidFormat
    } else {
        DataSpoolError::Io(err)
    }
}

impl SpoolReader {
    /// Open a spool file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_at(File::open(path)?, 0)
    }

    /// Open a spool embedded within a larger file at a given byte offset.
    ///
    /// Reads the SP01 header starting at `base_offset` (SPECIFICATION.md §5).
    /// The index keeps its spool-relative offsets: [`entries`](Self::entries),
    /// [`get_entry`](Self::get_entry) and [`read_card_at`](Self::read_card_at)
    /// use the same offsets as a stand-alone copy of the spool, and the reader
    /// adds `base_offset` when it seeks in the host file. This enables direct
    /// access to spool data stitched into an Engram archive without temp
    /// extraction.
    pub fn open_embedded<P: AsRef<Path>>(path: P, base_offset: u64) -> Result<Self> {
        Self::open_at(File::open(path)?, base_offset)
    }

    /// Read the header and index of the spool that starts at `base` in `file`.
    fn open_at(mut file: File, base: u64) -> Result<Self> {
        // Everything from `base` to the end of the file is available to the
        // spool. A region shorter than the header is a truncated spool.
        let region_len = file.metadata()?.len().saturating_sub(base);
        if region_len < HEADER_LEN {
            return Err(DataSpoolError::InvalidFormat);
        }

        file.seek(SeekFrom::Start(base))?;
        let mut header = [0u8; HEADER_LEN as usize];
        file.read_exact(&mut header).map_err(truncated_is_invalid)?;

        // Read and verify header
        if &header[0..4] != MAGIC {
            return Err(DataSpoolError::InvalidFormat);
        }
        if header[4] != VERSION {
            return Err(DataSpoolError::InvalidFormat);
        }
        let card_count = u32::from_le_bytes(header[5..9].try_into().unwrap());
        let index_offset = u64::from_le_bytes(header[9..17].try_into().unwrap());

        // An unfinished spool still holds the placeholder offset 0; the index
        // can never start inside the header.
        if index_offset < HEADER_LEN {
            return Err(DataSpoolError::InvalidFormat);
        }

        // The index must lie within the spool region before anything is
        // allocated for it.
        if !index_fits(index_offset, card_count, region_len) {
            return Err(DataSpoolError::InvalidFormat);
        }

        // index_offset is relative to spool start — adjust to host file position.
        let index_start = base
            .checked_add(index_offset)
            .ok_or(DataSpoolError::InvalidFormat)?;
        file.seek(SeekFrom::Start(index_start))?;
        let index_len = usize::try_from(u64::from(card_count) * INDEX_ENTRY_LEN)
            .map_err(|_| DataSpoolError::InvalidFormat)?;
        let mut index = vec![0u8; index_len];
        file.read_exact(&mut index).map_err(truncated_is_invalid)?;

        let mut entries = Vec::with_capacity(card_count as usize);
        for raw in index.chunks_exact(INDEX_ENTRY_LEN as usize) {
            let offset = u64::from_le_bytes(raw[0..8].try_into().unwrap());
            let length = u32::from_le_bytes(raw[8..12].try_into().unwrap());

            if !entry_fits(offset, length, index_offset) {
                return Err(DataSpoolError::InvalidFormat);
            }

            entries.push(SpoolEntry { offset, length });
        }

        Ok(Self {
            file,
            base,
            index_offset,
            entries,
        })
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

    /// Read `length` bytes at `offset`, relative to the start of the spool
    /// (for example a [`SpoolEntry`] or a `spool_offset`/`spool_length` pair
    /// from the index database).
    ///
    /// The range must lie within the card data, between the end of the header
    /// and the start of the index; otherwise this returns
    /// [`DataSpoolError::Decompression`] without reading or allocating.
    pub fn read_card_at(&mut self, offset: u64, length: usize) -> Result<Vec<u8>> {
        let in_card_data =
            u32::try_from(length).is_ok_and(|length| entry_fits(offset, length, self.index_offset));
        if !in_card_data {
            return Err(DataSpoolError::Decompression(format!(
                "{} bytes at offset {} are outside the card data (bytes {}..{})",
                length, offset, HEADER_LEN, self.index_offset
            )));
        }

        // The base plus an in-range offset was checked on open not to overflow
        // (the index, which follows every card, fits in the file).
        self.file.seek(SeekFrom::Start(self.base + offset))?;

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

    /// Total length of the spool in bytes: the header, the card data and the
    /// index, which is the last part of a spool (SPECIFICATION.md §2). This is
    /// `index_offset + 12 * card_count`.
    ///
    /// For an embedded spool the spool ends at `base_offset() + spool_len()` in
    /// the host file; anything after that belongs to the host. The reader never
    /// reads past that point.
    pub fn spool_len(&self) -> u64 {
        // Checked on open: the index fits within the file.
        self.index_offset + self.entries.len() as u64 * INDEX_ENTRY_LEN
    }

    /// Position of the spool's first byte in the file it was opened from: the
    /// `base_offset` given to [`open_embedded`](Self::open_embedded), or 0 for
    /// [`open`](Self::open).
    pub fn base_offset(&self) -> u64 {
        self.base
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

    #[test]
    fn test_truncated_spools_are_invalid_format() {
        let path = scratch_path("truncate_src");
        {
            let mut builder = SpoolBuilder::new(&path).unwrap();
            builder.add_card(b"one card").unwrap();
            builder.finalize().unwrap();
        }
        let bytes = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();

        // Every proper prefix of a valid spool is a truncated spool: cut in the
        // magic, the version, the header fields, the card data or the index.
        for cut in 0..bytes.len() {
            let path = scratch_path(&format!("truncate_{cut}"));
            fs::write(&path, &bytes[..cut]).unwrap();
            assert!(
                matches!(SpoolReader::open(&path), Err(DataSpoolError::InvalidFormat)),
                "open accepted a spool cut at {cut} bytes"
            );
            fs::remove_file(&path).unwrap();

            let host_path = scratch_path(&format!("truncate_host_{cut}"));
            let mut host = vec![0xAA; 64];
            host.extend_from_slice(&bytes[..cut]);
            fs::write(&host_path, &host).unwrap();
            assert!(
                matches!(
                    SpoolReader::open_embedded(&host_path, 64),
                    Err(DataSpoolError::InvalidFormat)
                ),
                "open_embedded accepted a spool cut at {cut} bytes"
            );
            fs::remove_file(&host_path).unwrap();
        }

        // A base offset at or past the end of the host file.
        let host_path = scratch_path("truncate_base_past_end");
        fs::write(&host_path, [0xAA; 64]).unwrap();
        for base in [64, 65, u64::MAX] {
            assert!(
                matches!(
                    SpoolReader::open_embedded(&host_path, base),
                    Err(DataSpoolError::InvalidFormat)
                ),
                "open_embedded accepted base {base}"
            );
        }
        fs::remove_file(&host_path).unwrap();
    }

    #[test]
    fn test_bad_magic_and_version_are_invalid_format() {
        let path = scratch_path("magic_src");
        SpoolBuilder::new(&path).unwrap().finalize().unwrap();
        let good = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();

        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(b"SP02");
        let mut bad_version = good.clone();
        bad_version[4] = 2;

        for (name, bytes) in [("bad_magic", bad_magic), ("bad_version", bad_version)] {
            let path = scratch_path(name);
            fs::write(&path, &bytes).unwrap();
            assert!(
                matches!(SpoolReader::open(&path), Err(DataSpoolError::InvalidFormat)),
                "open: {name}"
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
                "open_embedded: {name}"
            );
            fs::remove_file(&host_path).unwrap();
        }
    }

    #[test]
    fn test_embedded_entries_are_spool_relative() {
        let spool_path = scratch_path("relative_src");
        let card1 = b"card one";
        let card2 = b"card two, longer";
        {
            let mut builder = SpoolBuilder::new(&spool_path).unwrap();
            builder.add_card(card1).unwrap();
            builder.add_card(card2).unwrap();
            builder.finalize().unwrap();
        }
        let mut host = vec![0xAA; 64];
        host.extend_from_slice(&fs::read(&spool_path).unwrap());
        host.extend_from_slice(&[0xBB; 32]);
        let host_path = scratch_path("relative_host");
        fs::write(&host_path, &host).unwrap();

        let standalone = SpoolReader::open(&spool_path).unwrap();
        let mut embedded = SpoolReader::open_embedded(&host_path, 64).unwrap();
        assert_eq!(standalone.base_offset(), 0);
        assert_eq!(embedded.base_offset(), 64);

        // Same offsets as the stand-alone spool, as stored in its index.
        for (a, b) in standalone.entries().iter().zip(embedded.entries()) {
            assert_eq!((a.offset, a.length), (b.offset, b.length));
        }
        assert_eq!(embedded.get_entry(0).unwrap().offset, 17);

        // read_card_at takes spool-relative offsets.
        let e = embedded.get_entry(1).unwrap().clone();
        assert_eq!(
            embedded.read_card_at(e.offset, e.length as usize).unwrap(),
            card2
        );

        fs::remove_file(&spool_path).unwrap();
        fs::remove_file(&host_path).unwrap();
    }

    #[test]
    fn test_read_card_at_outside_card_data_is_error() {
        let path = scratch_path("read_at_range");
        {
            let mut builder = SpoolBuilder::new(&path).unwrap();
            builder.add_card(b"one card").unwrap(); // bytes 17..25, index at 25
            builder.finalize().unwrap();
        }
        let mut reader = SpoolReader::open(&path).unwrap();
        assert_eq!(reader.read_card_at(17, 8).unwrap(), b"one card");
        assert_eq!(reader.read_card_at(20, 5).unwrap(), b" card");

        for (offset, length) in [
            (0, 4),           // header
            (16, 2),          // straddles header and card data
            (20, 8),          // runs into the index
            (25, 12),         // the index itself
            (17, usize::MAX), // huge length must not allocate
            (u64::MAX, 1),    // overflow
        ] {
            assert!(
                matches!(
                    reader.read_card_at(offset, length),
                    Err(DataSpoolError::Decompression(_))
                ),
                "read_card_at({offset}, {length}) succeeded"
            );
        }
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_spool_len_marks_the_end_of_an_embedded_spool() {
        let spool_path = scratch_path("end_src");
        {
            let mut builder = SpoolBuilder::new(&spool_path).unwrap();
            builder.add_card(b"card one").unwrap();
            builder.add_card(b"card two").unwrap();
            builder.finalize().unwrap();
        }
        let spool = fs::read(&spool_path).unwrap();

        let reader = SpoolReader::open(&spool_path).unwrap();
        assert_eq!(reader.spool_len(), spool.len() as u64);

        // Host: [64 bytes][spool][trailer]. The spool ends where the trailer starts.
        let mut host = vec![0xAA; 64];
        host.extend_from_slice(&spool);
        host.extend_from_slice(b"TRAILER");
        let host_path = scratch_path("end_host");
        fs::write(&host_path, &host).unwrap();

        let reader = SpoolReader::open_embedded(&host_path, 64).unwrap();
        let end = (reader.base_offset() + reader.spool_len()) as usize;
        assert_eq!(&host[end..], b"TRAILER");

        // An empty spool is just the header and an empty index.
        let empty_path = scratch_path("end_empty");
        SpoolBuilder::new(&empty_path).unwrap().finalize().unwrap();
        assert_eq!(SpoolReader::open(&empty_path).unwrap().spool_len(), 17);

        fs::remove_file(&spool_path).unwrap();
        fs::remove_file(&host_path).unwrap();
        fs::remove_file(&empty_path).unwrap();
    }
}
