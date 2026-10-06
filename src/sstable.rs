use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::block::{Block, BlockBuilder};
use crate::bloom::BloomFilter;
use crate::coding::{decode_bytes, encode_bytes};
use crate::error::{EngineError, Result};

const MAGIC_NUMBER: u32 = 0x53535442; // "SSTB"
const FOOTER_SIZE: u64 = 40;

#[derive(Debug, PartialEq)]
pub struct TableFooter {
    pub index_offset: u64,
    pub index_len: u64,
    pub filter_offset: u64,
    pub filter_len: u64,
    pub filter_k: u32,
    pub magic: u32,
}

impl TableFooter {
    pub fn encode(&self) -> [u8; 40] {
        let mut buf = [0u8; 40];
        buf[0..8].copy_from_slice(&self.index_offset.to_le_bytes());
        buf[8..16].copy_from_slice(&self.index_len.to_le_bytes());
        buf[16..24].copy_from_slice(&self.filter_offset.to_le_bytes());
        buf[24..32].copy_from_slice(&self.filter_len.to_le_bytes());
        buf[32..36].copy_from_slice(&self.filter_k.to_le_bytes());
        buf[36..40].copy_from_slice(&self.magic.to_le_bytes());
        buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 40 {
            return Err(EngineError::UnexpectedEof);
        }
        let index_offset = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let index_len = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let filter_offset = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let filter_len = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let filter_k = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
        let magic = u32::from_le_bytes(bytes[36..40].try_into().unwrap());

        if magic != MAGIC_NUMBER {
            return Err(EngineError::InvalidChecksum);
        }

        Ok(Self {
            index_offset,
            index_len,
            filter_offset,
            filter_len,
            filter_k,
            magic,
        })
    }
}

pub struct SsTableBuilder {
    file: File,
    current_block: BlockBuilder,
    first_key_of_block: Option<Vec<u8>>,
    index: Vec<(Vec<u8>, u64, u64)>, // (first_key, offset, length)
    keys_for_filter: Vec<Vec<u8>>,
    current_offset: u64,
    block_size: usize,
}

impl SsTableBuilder {
    pub fn new<P: AsRef<Path>>(path: P, block_size: usize) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;

        Ok(Self {
            file,
            current_block: BlockBuilder::new(block_size),
            first_key_of_block: None,
            index: Vec::new(),
            keys_for_filter: Vec::new(),
            current_offset: 0,
            block_size,
        })
    }

    pub fn add(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        if self.first_key_of_block.is_none() {
            self.first_key_of_block = Some(key.to_vec());
        }

        // If block reaches target size, flush it
        if self.current_block.current_size() >= self.block_size && !self.current_block.is_empty() {
            self.flush_current_block()?;
            self.first_key_of_block = Some(key.to_vec());
        }

        self.current_block.add(key, value);
        self.keys_for_filter.push(key.to_vec());
        Ok(())
    }

    fn flush_current_block(&mut self) -> Result<()> {
        let block_builder = std::mem::replace(&mut self.current_block, BlockBuilder::new(self.block_size));
        let block_bytes = block_builder.build();
        let block_len = block_bytes.len() as u64;

        self.file.write_all(&block_bytes)?;

        if let Some(first_key) = self.first_key_of_block.take() {
            self.index.push((first_key, self.current_offset, block_len));
        }

        self.current_offset += block_len;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        // 1. Flush any remaining entries in the final block
        if !self.current_block.is_empty() {
            self.flush_current_block()?;
        }

        // 2. Write Index Block: [key_len][key][offset: 8B][len: 8B]
        let index_offset = self.current_offset;
        let mut index_buf = Vec::new();
        for (first_key, offset, len) in &self.index {
            encode_bytes(first_key, &mut index_buf);
            index_buf.extend_from_slice(&offset.to_le_bytes());
            index_buf.extend_from_slice(&len.to_le_bytes());
        }
        self.file.write_all(&index_buf)?;
        let index_len = index_buf.len() as u64;
        self.current_offset += index_len;

        // 3. Write Filter Block (Bloom Filter)
        let filter_offset = self.current_offset;
        let key_refs: Vec<&[u8]> = self.keys_for_filter.iter().map(|k| k.as_slice()).collect();
        let filter = BloomFilter::build_from_keys(&key_refs, 10);
        self.file.write_all(filter.bitmap())?;
        let filter_len = filter.bitmap().len() as u64;
        let filter_k = filter.k() as u32;
        self.current_offset += filter_len;

        // 4. Write Fixed 40-Byte Footer
        let footer = TableFooter {
            index_offset,
            index_len,
            filter_offset,
            filter_len,
            filter_k,
            magic: MAGIC_NUMBER,
        };
        self.file.write_all(&footer.encode())?;
        self.file.sync_all()?;
        Ok(())
    }
}

pub struct SsTable {
    file: File,
    index: Vec<(Vec<u8>, u64, u64)>,
    filter: BloomFilter,
}

impl SsTable {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = OpenOptions::new().read(true).open(path)?;

        // 1. Read Footer from the last 40 bytes
        file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
        let mut footer_bytes = [0u8; 40];
        file.read_exact(&mut footer_bytes)?;
        let footer = TableFooter::decode(&footer_bytes)?;

        // 2. Read Filter Block
        file.seek(SeekFrom::Start(footer.filter_offset))?;
        let mut filter_bytes = vec![0u8; footer.filter_len as usize];
        file.read_exact(&mut filter_bytes)?;
        let filter = BloomFilter::build_from_keys(&[], 0); // we will wrap filter_bytes below
        // Construct filter from raw bytes
        let filter = BloomFilterWrapper::from_raw(filter_bytes, footer.filter_k as u8);

        // 3. Read Index Block
        file.seek(SeekFrom::Start(footer.index_offset))?;
        let mut index_buf = vec![0u8; footer.index_len as usize];
        file.read_exact(&mut index_buf)?;

        let mut index = Vec::new();
        let mut cur: &[u8] = &index_buf;
        while !cur.is_empty() {
            let (key, rest) = decode_bytes(cur)?;
            let offset = u64::from_le_bytes(rest[0..8].try_into().unwrap());
            let len = u64::from_le_bytes(rest[8..16].try_into().unwrap());
            index.push((key.to_vec(), offset, len));
            cur = &rest[16..];
        }

        

        Ok(Self { file, index, filter: filter.into_inner() })
    }

    /// Point lookup: checks Bloom filter -> binary searches sparse index -> reads 1 block from disk!
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // Step 1: Bloom Filter check (0 disk reads!)
        if !self.filter.may_contain(key) {
            return Ok(None);
        }

        // Step 2: Binary Search Sparse Index
        if self.index.is_empty() {
            return Ok(None);
        }

        let idx = match self.index.binary_search_by(|(first_key, _, _)| first_key.as_slice().cmp(key)) {
            Ok(i) => i,
            Err(0) => return Ok(None),
            Err(i) => i - 1,
        };

        // Step 3: Seek & Read ONLY the target 4 KB Block from disk
        let (_, offset, len) = &self.index[idx];
        self.file.seek(SeekFrom::Start(*offset))?;
        let mut block_buf = vec![0u8; *len as usize];
        self.file.read_exact(&mut block_buf)?;

        let block = Block::decode(block_buf)?;
        Ok(block.get(key).map(|v| v.to_vec()))
    }

    pub fn iter_all(&mut self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut results = Vec::new();
        for (_, offset, len) in &self.index {
            self.file.seek(SeekFrom::Start(*offset))?;
            let mut block_buf = vec![0u8; *len as usize];
            self.file.read_exact(&mut block_buf)?;
            let block = Block::decode(block_buf)?;
            // Decode all entries using the block's offsets
            for i in 0..block.len() {
                if let Some((k, v)) = block.get_by_index(i) {
                    results.push((k.to_vec(), v.to_vec()));
                }
            }
        }
        Ok(results)
    }
}

// Small helper to construct a BloomFilter from raw bytes
struct BloomFilterWrapper(BloomFilter);
impl BloomFilterWrapper {
    fn from_raw(bitmap: Vec<u8>, k: u8) -> Self {
        Self(BloomFilter::from_raw(bitmap, k))
    }
    fn into_inner(self) -> BloomFilter {
        self.0
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sstable_build_and_read() {
        let path = "test_sstable.sst";
        let _ = std::fs::remove_file(path);

        // Build an SSTable with small 64-byte blocks to force multiple blocks
        {
            let mut builder = SsTableBuilder::new(path, 64).unwrap();
            builder.add(b"apple", b"val_apple").unwrap();
            builder.add(b"banana", b"val_banana").unwrap();
            builder.add(b"cherry", b"val_cherry").unwrap();
            builder.add(b"date", b"val_date").unwrap();
            builder.add(b"elderberry", b"val_elderberry").unwrap();
            builder.finish().unwrap();
        }

        // Open and read from the SSTable!
        {
            let mut sstable = SsTable::open(path).unwrap();

            // Hits
            assert_eq!(sstable.get(b"apple").unwrap(), Some(b"val_apple".to_vec()));
            assert_eq!(sstable.get(b"cherry").unwrap(), Some(b"val_cherry".to_vec()));
            assert_eq!(sstable.get(b"elderberry").unwrap(), Some(b"val_elderberry".to_vec()));

            // Misses (Bloom filter intercepts these!)
            assert_eq!(sstable.get(b"avocado").unwrap(), None);
            assert_eq!(sstable.get(b"watermelon").unwrap(), None);
        }

        let _ = std::fs::remove_file(path);
    }
}
