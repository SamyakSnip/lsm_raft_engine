use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};

use crate::compaction::compact_two_tables;
use crate::error::Result;
use crate::memtable::{MemTable, ValueState};
use crate::sstable::{SsTable, SsTableBuilder};
use crate::wal::Wal;

enum BgTask {
    Flush {
        memtable: MemTable,
        table_id: u64,
    },
    Shutdown,
}

pub struct LsmEngine {
    dir: PathBuf,
    memtable: Arc<RwLock<MemTable>>,
    wal: Arc<Mutex<Wal>>,
    tables: Arc<RwLock<Vec<Mutex<SsTable>>>>,
    flush_threshold: usize,
    next_table_id: Arc<AtomicU64>,
    bg_sender: Option<Sender<BgTask>>,
    bg_thread: Option<JoinHandle<()>>,
}

impl LsmEngine {
    /// Opens or creates an LSM storage engine directory.
    pub fn open<P: AsRef<Path>>(dir: P, flush_threshold: usize) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;

        let wal_path = dir.join("wal.log");

        // 1. Crash Recovery: Replay WAL records into a fresh MemTable
        let mut memtable = MemTable::new();
        let recovered = Wal::recover(&wal_path)?;
        for (k, v) in recovered {
            memtable.put(&k, &v);
        }

        // 2. Open append-only WAL
        let wal = Wal::open(&wal_path)?;

        let memtable = Arc::new(RwLock::new(memtable));
        let wal = Arc::new(Mutex::new(wal));
        let tables = Arc::new(RwLock::new(Vec::new()));
        let next_table_id = Arc::new(AtomicU64::new(1));

        // 3. Start Background Flusher Worker Thread
        let (sender, receiver) = channel::<BgTask>();
        let bg_dir = dir.clone();
        let bg_tables = Arc::clone(&tables);

        let bg_thread = thread::spawn(move || {
            while let Ok(task) = receiver.recv() {
                match task {
                    BgTask::Flush { memtable, table_id } => {
                        let table_path = bg_dir.join(format!("{:06}.sst", table_id));
                        // Flush memtable to SSTable file on disk
                        let mut builder = SsTableBuilder::new(&table_path, 4096).unwrap();
                        for (k, v_opt) in memtable.iter() {
                            if let Some(v) = v_opt {
                                builder.add(k, v).unwrap();
                            }
                        }
                        builder.finish().unwrap();

                        // Open new SSTable and register it
                        let sstable = SsTable::open(&table_path).unwrap();
                        let mut tables_lock = bg_tables.write().unwrap();
                        tables_lock.push(Mutex::new(sstable));

                        // If we have 2 or more SSTables, compact them!
                        if tables_lock.len() >= 2 {
                            let older_path = bg_dir.join(format!("{:06}.sst", table_id - 1));
                            let newer_path = table_path;
                            let compacted_path = bg_dir.join(format!("compacted_{:06}.sst", table_id));

                            if let (Ok(mut older), Ok(mut newer)) = (
                                SsTable::open(&older_path),
                                SsTable::open(&newer_path),
                            ) {
                                if compact_two_tables(&mut older, &mut newer, &compacted_path, 4096).is_ok() {
                                    // Remove old files
                                    let _ = fs::remove_file(&older_path);
                                    let _ = fs::remove_file(&newer_path);
                                    let _ = fs::rename(&compacted_path, bg_dir.join("000001.sst"));
                                }
                            }
                        }
                    }
                    BgTask::Shutdown => break,
                }
            }
        });

        Ok(Self {
            dir,
            memtable,
            wal,
            tables,
            flush_threshold,
            next_table_id,
            bg_sender: Some(sender),
            bg_thread: Some(bg_thread),
        })
    }

    /// Appends to WAL and inserts into MemTable.
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        // 1. Write-Ahead Log append & sync
        {
            let mut wal_lock = self.wal.lock().unwrap();
            wal_lock.append(key, value)?;
            wal_lock.sync()?;
        }

        // 2. Insert into MemTable
        {
            let mut mem_lock = self.memtable.write().unwrap();
            mem_lock.put(key, value);
        }

        // 3. Trigger flush if MemTable exceeds threshold
        self.check_flush_trigger();

        Ok(())
    }

    /// Deletes a key by logging and inserting a tombstone.
    pub fn delete(&self, key: &[u8]) -> Result<()> {
        let mut mem_lock = self.memtable.write().unwrap();
        mem_lock.delete(key);
        Ok(())
    }

    /// Reads a key following the complete LSM hierarchy.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // 1. Check in-memory MemTable
        {
            let mem_lock = self.memtable.read().unwrap();
            match mem_lock.get(key) {
                ValueState::Value(v) => return Ok(Some(v.to_vec())),
                ValueState::Tombstone => return Ok(None),
                ValueState::NotFound => {}
            }
        }

        // 2. Check SSTables from newest to oldest
        let tables_lock = self.tables.read().unwrap();
        for table_mutex in tables_lock.iter().rev() {
            let mut table = table_mutex.lock().unwrap();
            if let Some(v) = table.get(key)? {
                return Ok(Some(v));
            }
        }

        Ok(None)
    }

    fn check_flush_trigger(&self) {
        let current_size = self.memtable.read().unwrap().approximate_size();
        if current_size >= self.flush_threshold {
            // Swap out active memtable with a fresh one!
            let mut mem_lock = self.memtable.write().unwrap();
            if mem_lock.approximate_size() >= self.flush_threshold {
                let full_memtable = std::mem::replace(&mut *mem_lock, MemTable::new());
                let table_id = self.next_table_id.fetch_add(1, Ordering::SeqCst);

                if let Some(ref sender) = self.bg_sender {
                    let _ = sender.send(BgTask::Flush {
                        memtable: full_memtable,
                        table_id,
                    });
                }
            }
        }
    }
}

impl Drop for LsmEngine {
    fn drop(&mut self) {
        if let Some(sender) = self.bg_sender.take() {
            let _ = sender.send(BgTask::Shutdown);
        }
        if let Some(handle) = self.bg_thread.take() {
            let _ = handle.join();
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_concurrent_reads_and_writes() {
        let test_dir = "test_lsm_engine";
        let _ = fs::remove_dir_all(test_dir);

        let engine = Arc::new(LsmEngine::open(test_dir, 128).unwrap());

        // Spawn 4 writer threads
        let mut handles = Vec::new();
        for t in 0..4 {
            let eng = Arc::clone(&engine);
            handles.push(thread::spawn(move || {
                for i in 0..20 {
                    let key = format!("user:{}:{}", t, i);
                    let val = format!("val:{}", i);
                    eng.put(key.as_bytes(), val.as_bytes()).unwrap();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Verify written keys
        for t in 0..4 {
            for i in 0..20 {
                let key = format!("user:{}:{}", t, i);
                let expected = format!("val:{}", i);
                assert_eq!(
                    engine.get(key.as_bytes()).unwrap(),
                    Some(expected.into_bytes())
                );
            }
        }

        let _ = fs::remove_dir_all(test_dir);
    }
}
