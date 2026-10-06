use crate::coding::{decode_bytes, decode_u32_le, encode_bytes, encode_u32_le};
use crate::error::{EngineError, Result};


pub struct BlockBuilder {
    data: Vec<u8>,
    offsets: Vec<u32>,
    target_size: usize,
}

impl BlockBuilder {
    pub fn new(target_size: usize) -> Self {
        Self {
            data: Vec::new(),
            offsets: Vec::new(),
            target_size,
        }
    }

    /// Appends a key-value pair to the block.
    pub fn add(&mut self, key: &[u8], value: &[u8]) {
        // 1. Record the current byte offset where this entry starts:
        self.offsets.push(self.data.len() as u32);

        // 2. Append the key and value:
        encode_bytes(key, &mut self.data);
        encode_bytes(value, &mut self.data);
    }

    /// Calculates the current estimated size of the block:
    /// (data bytes) + (offsets * 4 bytes) + (4 bytes for num_offsets)
    pub fn current_size(&self) -> usize {
        self.data.len() + (self.offsets.len() * 4) + 4
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Finalizes the block by writing the offset array and num_offsets at the end.
    pub fn build(mut self) -> Vec<u8> {
        // 1. Write every offset in self.offsets using encode_u32_le:
        for offset in &self.offsets {
            encode_u32_le(*offset, &mut self.data);
        }

        // 2. Write the total number of offsets at the very end:
        encode_u32_le(self.offsets.len() as u32, &mut self.data);

        // 3. Return the finished byte buffer!
        self.data
    }
}

pub struct Block {
    data: Vec<u8>,
    offsets: Vec<u32>,
}

impl Block {
    /// Decodes a raw block buffer into a Block with its parsed offset array.
    pub fn decode(data: Vec<u8>) -> Result<Self> {
        if data.len() < 4 {
            return Err(EngineError::UnexpectedEof);
        }

        // 1. Read num_offsets from the last 4 bytes:
        let (num_offsets, _) = decode_u32_le(&data[data.len() - 4..])?;
        let num_offsets = num_offsets as usize;

        // 2. Validate that data has enough bytes for the offset array + num_offsets:
        let offset_array_len = num_offsets * 4;
        if data.len() < offset_array_len + 4 {
            return Err(EngineError::UnexpectedEof);
        }

        // 3. Parse each 4-byte offset from the offset section:
        let offset_array_start = data.len() - 4 - offset_array_len;
        let mut offsets = Vec::with_capacity(num_offsets);
        let mut cur = &data[offset_array_start..data.len() - 4];

        for _ in 0..num_offsets {
            let (offset, rest) = decode_u32_le(cur)?;
            offsets.push(offset);
            cur = rest;
        }

        Ok(Self { data, offsets })
    }

    /// Binary searches for a key in this block in O(log K) time.
    /// Returns a zero-copy slice of the value if found, or None.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        // 1. Binary search the offset array!
        let idx = self.offsets.binary_search_by(|&offset| {
            let entry_slice = &self.data[offset as usize..];
            let (entry_key, _) = decode_bytes(entry_slice).expect("corrupt block entry");
            entry_key.cmp(key)
        }).ok()?;

        // 2. We found the key at index `idx`! Extract its value:
        let entry_slice = &self.data[self.offsets[idx] as usize..];
        let (_, after_key) = decode_bytes(entry_slice).ok()?;
        let (val, _) = decode_bytes(after_key).ok()?;
        Some(val)
    }

    /// Returns the first key in the block (used to build the sparse index!).
    pub fn first_key(&self) -> Option<&[u8]> {
        if self.offsets.is_empty() {
            return None;
        }
        let first_offset = self.offsets[0] as usize;
        let (key, _) = decode_bytes(&self.data[first_offset..]).ok()?;
        Some(key)
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }
    
    pub fn get_by_index(&self, index: usize) -> Option<(&[u8], &[u8])> {
        if index >= self.offsets.len() {
            return None;
        }
        let entry_slice = &self.data[self.offsets[index] as usize..];
        let (key, after_key) = decode_bytes(entry_slice).ok()?;
        let (val, _) = decode_bytes(after_key).ok()?;
        Some((key, val))
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_roundtrip_and_binary_search() {
        let mut builder = BlockBuilder::new(4096);
        builder.add(b"apple", b"100");
        builder.add(b"banana", b"200");
        builder.add(b"cherry", b"300");
        builder.add(b"date", b"400");

        let block_bytes = builder.build();

        // 1. Decode block
        let block = Block::decode(block_bytes).unwrap();
        assert_eq!(block.first_key(), Some(b"apple".as_slice()));

        // 2. Binary search hits
        assert_eq!(block.get(b"apple"), Some(b"100".as_slice()));
        assert_eq!(block.get(b"banana"), Some(b"200".as_slice()));
        assert_eq!(block.get(b"cherry"), Some(b"300".as_slice()));
        assert_eq!(block.get(b"date"), Some(b"400".as_slice()));

        // 3. Binary search misses
        assert_eq!(block.get(b"avocado"), None);
        assert_eq!(block.get(b"blueberry"), None);
        assert_eq!(block.get(b"zebra"), None);
    }
}
