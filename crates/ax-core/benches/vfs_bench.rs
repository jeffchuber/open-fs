use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use tempfile::TempDir;
use tokio::runtime::Runtime;

use ax_config::VfsConfig;
use ax_core::Vfs;

fn create_vfs(temp_dir: &TempDir) -> Vfs {
    let rt = Runtime::new().unwrap();
    let yaml = format!(
        r#"
name: bench-vfs
backends:
  local:
    type: fs
    root: {}
mounts:
  - path: /workspace
    backend: local
"#,
        temp_dir.path().to_str().unwrap()
    );
    let config = VfsConfig::from_yaml(&yaml).unwrap();
    rt.block_on(async { Vfs::from_config(config).await.unwrap() })
}

fn vfs_write_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let vfs = create_vfs(&temp_dir);

    let content = vec![0u8; 1024]; // 1KB

    c.bench_function("vfs_write_1kb", |b| {
        b.to_async(&rt).iter(|| async {
            vfs.write(black_box("/workspace/bench.txt"), black_box(&content)).await.unwrap();
        });
    });
}

fn vfs_read_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let vfs = create_vfs(&temp_dir);

    // Create test file
    let content = vec![0u8; 1024];
    rt.block_on(async {
        vfs.write("/workspace/bench.txt", &content).await.unwrap();
    });

    c.bench_function("vfs_read_1kb", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = vfs.read(black_box("/workspace/bench.txt")).await.unwrap();
        });
    });
}

fn vfs_write_sizes_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let vfs = create_vfs(&temp_dir);

    let mut group = c.benchmark_group("vfs_write_sizes");

    for size in [64, 256, 1024, 4096, 16384, 65536].iter() {
        let content = vec![0u8; *size];

        group.bench_with_input(BenchmarkId::new("write", size), size, |b, _| {
            b.to_async(&rt).iter(|| async {
                vfs.write(black_box("/workspace/bench.txt"), black_box(&content)).await.unwrap();
            });
        });
    }

    group.finish();
}

fn vfs_list_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let vfs = create_vfs(&temp_dir);

    // Create test files
    rt.block_on(async {
        for i in 0..100 {
            vfs.write(&format!("/workspace/file{}.txt", i), b"content").await.unwrap();
        }
    });

    c.bench_function("vfs_list_100_files", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = vfs.list(black_box("/workspace")).await.unwrap();
        });
    });
}

fn vfs_exists_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let vfs = create_vfs(&temp_dir);

    // Create test file
    rt.block_on(async {
        vfs.write("/workspace/exists.txt", b"content").await.unwrap();
    });

    c.bench_function("vfs_exists_true", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = vfs.exists(black_box("/workspace/exists.txt")).await.unwrap();
        });
    });

    c.bench_function("vfs_exists_false", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = vfs.exists(black_box("/workspace/nonexistent.txt")).await.unwrap();
        });
    });
}

criterion_group!(
    benches,
    vfs_write_benchmark,
    vfs_read_benchmark,
    vfs_write_sizes_benchmark,
    vfs_list_benchmark,
    vfs_exists_benchmark,
);
criterion_main!(benches);
