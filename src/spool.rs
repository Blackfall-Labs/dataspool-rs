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
        let length = card_data.len() as u32;

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
        let card_count = self.entries.len() as u32;

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
            return Err(DataSpoolError::Decompression(format!(
                "Card index {} out of bounds (max: {})",
                index,
                self.entries.len() - 1
            )));
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
}
