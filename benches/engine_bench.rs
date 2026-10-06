use criterion::{black_box, criterion_group, criterion_main, Criterion};
use lsm_raft_engine::block::{Block, BlockBuilder};
use lsm_raft_engine::bloom::BloomFilter;
use lsm_raft_engine::engine::LsmEngine;
use lsm_raft_engine::memtable::MemTable;

fn bench_bloom_filter(c: &mut Criterion) {
    let keys: Vec<Vec<u8>> = (0..1000).map(|i| format!("key:{:06}", i).into_bytes()).collect();
    let key_refs: Vec<&[u8]> = keys.iter().map(|k| k.as_slice()).collect();
    let filter = BloomFilter::build_from_keys(&key_refs, 10);

    let mut group = c.benchmark_group("BloomFilter");
    group.bench_function("hit", |b| {
        b.iter(|| {
            filter.may_contain(black_box(b"key:000500"))
        })
    });
    group.bench_function("miss", |b| {
        b.iter(|| {
            filter.may_contain(black_box(b"non_existent_key"))
        })
    });
    group.finish();
}

fn bench_block_binary_search(c: &mut Criterion) {
    let mut builder = BlockBuilder::new(4096);
    for i in 0..100 {
        let key = format!("user:{:04}", i);
        let val = format!("val:{:04}", i);
        builder.add(key.as_bytes(), val.as_bytes());
    }
    let block = Block::decode(builder.build()).unwrap();

    c.bench_function("Block_BinarySearch_Hit", |b| {
        b.iter(|| {
            block.get(black_box(b"user:0050"))
        })
    });
}

fn bench_memtable(c: &mut Criterion) {
    let mut memtable = MemTable::new();
    for i in 0..1000 {
        let key = format!("k{:04}", i);
        memtable.put(key.as_bytes(), b"value");
    }

    let mut group = c.benchmark_group("MemTable");
    group.bench_function("put", |b| {
        let mut m = MemTable::new();
        b.iter(|| {
            m.put(black_box(b"key"), black_box(b"val"));
        })
    });
    group.bench_function("get", |b| {
        b.iter(|| {
            memtable.get(black_box(b"k0500"))
        })
    });
    group.finish();
}

fn bench_engine_writes(c: &mut Criterion) {
    let dir = "bench_lsm_engine";
    let _ = std::fs::remove_dir_all(dir);
    let engine = LsmEngine::open(dir, 1024 * 1024).unwrap();

    let mut i = 0u64;
    c.bench_function("LsmEngine_Write_Throughput", |b| {
        b.iter(|| {
            i += 1;
            let key = i.to_le_bytes();
            let val = [0u8; 64]; // 64-byte payload
            engine.put(black_box(&key), black_box(&val)).unwrap();
        })
    });

    let _ = std::fs::remove_dir_all(dir);
}

criterion_group!(benches, bench_bloom_filter, bench_block_binary_search, bench_memtable, bench_engine_writes);
criterion_main!(benches);
