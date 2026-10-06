# lsm_raft_engine

A high-performance Distributed LSM-Tree Storage Engine with Raft Consensus written from scratch in Rust, inspired by RocksDB, TiKV, and Cassandra.

---

## Architecture Overview

- **Binary Codec:** Zero-copy byte slicing and little-endian primitive encoding.
- **Write-Ahead Log (WAL):** Sequential append-only file with SIMD-accelerated CRC32 checksums for crash recovery and torn-write protection.
- **In-Memory MemTable:** Sorted `BTreeMap` supporting concurrent reads/writes, dynamic memory accounting, and tombstone deletions.
- **SSTable Storage:**
  - **Data Blocks:** ~4 KB fixed chunks with internal offset arrays for binary search.
  - **Sparse Index:** First-key indexing to pinpoint target disk blocks.
  - **Bloom Filter:** Space-efficient bit array (10 bits/key) using Kirsch-Mitzenmacher dual hashing to avoid disk I/O on missing keys.
  - **Fixed Footer:** 40-byte trailer at EOF for $O(1)$ table opening.
- **Compaction:** 2-way merge sort running on a background worker thread to purge stale keys and tombstones.
- **Raft Consensus:** Distributed replicated state machine featuring leader election, log replication, commit quorum ($N/2 + 1$), and network partition healing.

---

## Benchmark Results (Criterion)

Benchmarked on AMD/Intel x86-64 hardware:

| Component | Benchmark | Latency | Throughput |
| :--- | :--- | :--- | :--- |
| **Bloom Filter** | Negative Lookup (`miss`) | **~23.6 ns** | ~42.2 M ops/sec |
| **Bloom Filter** | Positive Lookup (`hit`) | **~49.5 ns** | ~20.1 M ops/sec |
| **Data Block** | 4 KB Block Binary Search | **~47.3 ns** | ~21.1 M lookups/sec |
| **MemTable** | In-Memory Write (`put`) | **~29.6 ns** | ~33.7 M inserts/sec |
| **MemTable** | In-Memory Read (`get`) | **~38.3 ns** | ~26.1 M reads/sec |
| **Full Engine** | Durable Write (`WAL + fsync`) | **~529.6 µs** | ~1,888 durable writes/sec |

---

## Quickstart

### Run Tests
```bash
cargo test
```

### Run Benchmarks
```bash
cargo bench
```
