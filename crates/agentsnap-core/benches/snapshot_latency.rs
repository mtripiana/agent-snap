use std::time::Instant;

use agentsnap_core::{SnapshotConfig, SnapshotSession};
use tempfile::tempdir;

fn main() {
    let workspace = tempdir().expect("workspace tempdir");
    let scratch = tempdir().expect("scratch tempdir");

    std::fs::write(workspace.path().join("seed.txt"), "seed").expect("write seed file");

    let config = SnapshotConfig {
        base_dir: workspace.path().to_path_buf(),
        scratch_dir: scratch.path().to_path_buf(),
        read_only: false,
    };

    let iterations = 50;
    let mut durations_ms = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let started = Instant::now();
        let session = SnapshotSession::create(&config).expect("create snapshot");
        durations_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        session.rollback().expect("rollback snapshot");
    }

    durations_ms.sort_by(|left, right| left.partial_cmp(right).expect("finite latency values"));
    let p95_index = ((iterations as f64) * 0.95).ceil() as usize - 1;
    let p95 = durations_ms[p95_index];
    let avg = durations_ms.iter().sum::<f64>() / durations_ms.len() as f64;

    println!("snapshot latency avg={avg:.3}ms p95={p95:.3}ms");

    assert!(
        p95 < 10.0,
        "snapshot latency benchmark failed: p95={p95:.3}ms (samples: {durations_ms:?})"
    );
}
