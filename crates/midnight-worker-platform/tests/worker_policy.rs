use midnight_worker_platform::{CpuBudget, ResourcePolicy};
use std::io::{BufRead, BufReader, Write};

#[test]
fn explicit_policies_preserve_file_network_and_process_containment() {
    let root = tempfile::tempdir().unwrap();
    let secret = root.path().join("private-source");
    std::fs::write(&secret, b"private").unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    for cpu in [CpuBudget::Seconds(5), CpuBudget::NoAdditionalLimit] {
        let mut worker = midnight_worker_platform::spawn_with_policy(
            std::path::Path::new(env!("CARGO_BIN_EXE_midnight-worker-probe")),
            ResourcePolicy {
                memory_bytes: 512 * 1024 * 1024,
                cpu,
            },
        )
        .unwrap_or_else(|error| panic!("{cpu:?}: {error}"));
        let mut output = BufReader::new(worker.output);
        midnight_worker_sdk::read_policy_ready(&mut output).unwrap();
        writeln!(worker.input, "{}", secret.display()).unwrap();
        writeln!(worker.input, "{}", listener.local_addr().unwrap().port()).unwrap();
        worker.input.flush().unwrap();
        let mut reply = String::new();
        output.read_line(&mut reply).unwrap();
        assert_eq!(reply.trim(), "true true");
        assert!(worker.process.wait().unwrap());
    }
}

#[test]
fn invalid_explicit_policy_fails_before_starting_an_executable() {
    let path = std::path::Path::new(env!("CARGO_BIN_EXE_midnight-worker-probe"));
    assert!(
        midnight_worker_platform::spawn_with_policy(
            path,
            ResourcePolicy {
                memory_bytes: 1,
                cpu: CpuBudget::NoAdditionalLimit
            }
        )
        .is_err()
    );
    assert!(
        midnight_worker_platform::spawn_with_policy(
            path,
            ResourcePolicy {
                memory_bytes: 64 * 1024 * 1024,
                cpu: CpuBudget::Seconds(0)
            }
        )
        .is_err()
    );
}
