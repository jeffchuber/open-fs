use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::time::Duration;
use tokio::runtime::Runtime;

use ax_core::{CacheConfig, LruCache};

fn cache_put_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let cache = LruCache::new(CacheConfig {
        max_entries: 10000,
        max_size: 100 * 1024 * 1024,
        ttl: Duration::from_secs(300),
        enabled: true,
    });

    let content = vec![0u8; 1024]; // 1KB content

    c.bench_function("cache_put_1kb", |b| {
        b.to_async(&rt).iter(|| async {
            cache.put(black_box("/test/file.txt"), black_box(content.clone())).await;
        });
    });
}

fn cache_get_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let cache = LruCache::new(CacheConfig {
        max_entries: 10000,
        max_size: 100 * 1024 * 1024,
        ttl: Duration::from_secs(300),
        enabled: true,
    });

    // Pre-populate cache
    let content = vec![0u8; 1024];
    rt.block_on(async {
        for i in 0..1000 {
            cache.put(&format!("/test/file{}.txt", i), content.clone()).await;
        }
    });

    c.bench_function("cache_get_hit", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = cache.get(black_box("/test/file500.txt")).await;
        });
    });

    c.bench_function("cache_get_miss", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = cache.get(black_box("/test/nonexistent.txt")).await;
        });
    });
}

fn cache_size_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("cache_put_sizes");

    for size in [64, 256, 1024, 4096, 16384].iter() {
        let cache = LruCache::new(CacheConfig {
            max_entries: 10000,
            max_size: 100 * 1024 * 1024,
            ttl: Duration::from_secs(300),
            enabled: true,
        });

        let content = vec![0u8; *size];

        group.bench_with_input(BenchmarkId::new("put", size), size, |b, _| {
            b.to_async(&rt).iter(|| async {
                cache.put(black_box("/test/file.txt"), black_box(content.clone())).await;
            });
        });
    }

    group.finish();
}

fn cache_eviction_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.bench_function("cache_eviction", |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let cache = LruCache::new(CacheConfig {
                max_entries: 100,
                max_size: 100 * 1024,
                ttl: Duration::from_secs(300),
                enabled: true,
            });

            let content = vec![0u8; 512];
            let start = std::time::Instant::now();

            for i in 0..iters {
                cache.put(&format!("/test/file{}.txt", i), content.clone()).await;
            }

            start.elapsed()
        });
    });
}

criterion_group!(
    benches,
    cache_put_benchmark,
    cache_get_benchmark,
    cache_size_benchmark,
    cache_eviction_benchmark,
);
criterion_main!(benches);
