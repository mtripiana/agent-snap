use std::time::Instant;

use agentsnap_core::{SnapshotConfig, SnapshotSession};
use tempfile::tempdir;

#[test]
fn test_snapshot_creation_latency() {
    let workspace = tempdir().expect("workspace tempdir");
    let scratch = tempdir().expect("scratch tempdir");

    std::fs::write(workspace.path().join("seed.txt"), "seed").expect("write seed file");

    let config = SnapshotConfig {
        base_dir: workspace.path().to_path_buf(),
        scratch_dir: scratch.path().to_path_buf(),
        read_only: false,
    };

    let iterations = 25;
    let mut durations_ms = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let started = Instant::now();
        let session = SnapshotSession::create(&config).expect("create snapshot");
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        durations_ms.push(elapsed_ms);
        session.rollback().expect("rollback snapshot");
    }

    durations_ms.sort_by(|left, right| left.partial_cmp(right).expect("finite latency values"));
    let p95_index = ((iterations as f64) * 0.95).ceil() as usize - 1;
    let p95 = durations_ms[p95_index];

    assert!(
        p95 < 10.0,
        "snapshot creation p95 latency too high: {p95:.3}ms (samples: {durations_ms:?})"
    );
}
