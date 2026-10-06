use std::path::Path;

use crate::error::Result;
use crate::sstable::{SsTable, SsTableBuilder};

/// Merges an older SSTable and a newer SSTable into a single consolidated SSTable.
/// If a key exists in both, the value from `newer` is retained and `older` is purged.
pub fn compact_two_tables<P: AsRef<Path>>(
    older: &mut SsTable,
    newer: &mut SsTable,
    output_path: P,
    block_size: usize,
) -> Result<()> {
    let older_entries = older.iter_all()?;
    let newer_entries = newer.iter_all()?;

    let mut builder = SsTableBuilder::new(output_path, block_size)?;

    let mut i = 0;
    let mut j = 0;

    // Two-way merge sort
    while i < older_entries.len() && j < newer_entries.len() {
        let (k_old, v_old) = &older_entries[i];
        let (k_new, v_new) = &newer_entries[j];

        if k_old < k_new {
            builder.add(k_old, v_old)?;
            i += 1;
        } else if k_new < k_old {
            builder.add(k_new, v_new)?;
            j += 1;
        } else {
            // Keys match! Newer wins, older is discarded:
            builder.add(k_new, v_new)?;
            i += 1;
            j += 1;
        }
    }

    // Flush any remaining entries
    while i < older_entries.len() {
        let (k, v) = &older_entries[i];
        builder.add(k, v)?;
        i += 1;
    }

    while j < newer_entries.len() {
        let (k, v) = &newer_entries[j];
        builder.add(k, v)?;
        j += 1;
    }

    builder.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_deduplication() {
        let older_path = "older.sst";
        let newer_path = "newer.sst";
        let compacted_path = "compacted.sst";

        let _ = std::fs::remove_file(older_path);
        let _ = std::fs::remove_file(newer_path);
        let _ = std::fs::remove_file(compacted_path);

        // 1. Build older SSTable: ["a": "1", "b": "old_val", "c": "1"]
        {
            let mut b1 = SsTableBuilder::new(older_path, 64).unwrap();
            b1.add(b"a", b"1").unwrap();
            b1.add(b"b", b"old_val").unwrap();
            b1.add(b"c", b"1").unwrap();
            b1.finish().unwrap();
        }

        // 2. Build newer SSTable: ["b": "new_val", "d": "1"]
        {
            let mut b2 = SsTableBuilder::new(newer_path, 64).unwrap();
            b2.add(b"b", b"new_val").unwrap();
            b2.add(b"d", b"1").unwrap();
            b2.finish().unwrap();
        }

        // 3. Compact them!
        {
            let mut older = SsTable::open(older_path).unwrap();
            let mut newer = SsTable::open(newer_path).unwrap();
            compact_two_tables(&mut older, &mut newer, compacted_path, 64).unwrap();
        }

        // 4. Verify compacted table: "b" MUST have "new_val"!
        {
            let mut compacted = SsTable::open(compacted_path).unwrap();
            assert_eq!(compacted.get(b"a").unwrap(), Some(b"1".to_vec()));
            assert_eq!(compacted.get(b"b").unwrap(), Some(b"new_val".to_vec())); // Verified!
            assert_eq!(compacted.get(b"c").unwrap(), Some(b"1".to_vec()));
            assert_eq!(compacted.get(b"d").unwrap(), Some(b"1".to_vec()));
        }

        let _ = std::fs::remove_file(older_path);
        let _ = std::fs::remove_file(newer_path);
        let _ = std::fs::remove_file(compacted_path);
    }
}
