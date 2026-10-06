use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::coding::{decode_bytes, decode_u32_le, encode_bytes, encode_u32_le};
use crate::error::Result;

pub struct Wal {
    file: File,
}

impl Wal {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { file })
    }

    pub fn append(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        // 1. Build payload buffer (key + value)
        let mut payload = Vec::new();
        encode_bytes(key, &mut payload);
        encode_bytes(value, &mut payload);

        // 2. Compute CRC32 checksum of the payload
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&payload);
        let checksum = hasher.finalize();

        // 3. Record buffer: encode CRC32, then append payload
        let mut record = Vec::new();
        encode_u32_le(checksum, &mut record);
        record.extend_from_slice(&payload);

        // 4. Write record to self.file using write_all
        self.file.write_all(&record)?;

        // 5. Return Ok
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

        /// Recovers all valid key-value pairs from the log.
    /// Stops safely at the first corrupted or incomplete (torn) write.
    pub fn recover<P: AsRef<Path>>(path: P) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let data = std::fs::read(path)?;
        let mut cur: &[u8] = &data;
        let mut records = Vec::new();

        while !cur.is_empty() {
            // 1. Read CRC32 (4 bytes). If truncated, stop recovery.
            let (expected_crc, after_crc) = match decode_u32_le(cur) {
                Ok(res) => res,
                Err(_) => break,
            };

            // 2. Read key. If truncated, stop recovery.
            let (key, after_key) = match decode_bytes(after_crc) {
                Ok(res) => res,
                Err(_) => break,
            };

            // 3. Read value. If truncated, stop recovery.
            let (value, after_val) = match decode_bytes(after_key) {
                Ok(res) => res,
                Err(_) => break,
            };

            // 4. Verify CRC32 over the payload: [key_len + key] + [val_len + val]
            let payload_len = after_crc.len() - after_val.len();
            let payload = &after_crc[..payload_len];

            let mut hasher = crc32fast::Hasher::new();
            hasher.update(payload);
            if hasher.finalize() != expected_crc {
                // Checksum mismatch! Corrupted record / torn write, stop cleanly.
                break;
            }

            // 5. Valid record! Convert borrowed slices to owned Vec<u8>
            records.push((key.to_vec(), value.to_vec()));
            cur = after_val;
        }

        Ok(records)
    }



}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wal_recovery_with_corruption() {
        let path = "test_wal.log";
        // Clean up previous run if it exists
        let _ = std::fs::remove_file(path);
        // 1. Write two valid records
        {
            let mut wal = Wal::open(path).unwrap();
            wal.append(b"key1", b"val1").unwrap();
            wal.append(b"key2", b"val2").unwrap();
            wal.sync().unwrap();
        }
        // 2. Simulate a crash: Append torn/garbage bytes to the file!
        {
            let mut file = OpenOptions::new().append(true).open(path).unwrap();
            file.write_all(b"corrupted_garbage_bytes").unwrap();
            file.sync_all().unwrap();
        }
        // 3. Recover! It should cleanly recover key1 and key2, ignoring the garbage!
        let records = Wal::recover(path).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], (b"key1".to_vec(), b"val1".to_vec()));
        assert_eq!(records[1], (b"key2".to_vec(), b"val2".to_vec()));
        // Clean up
        let _ = std::fs::remove_file(path);
    }
}
