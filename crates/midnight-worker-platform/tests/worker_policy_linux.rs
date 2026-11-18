#![cfg(target_os = "linux")]
use midnight_worker_platform::{CpuBudget, ExitStatus, ResourcePolicy};
use rustix::process::{Resource, Rlimit, setrlimit};

#[test]
fn no_additional_cpu_limit_preserves_inherited_hard_limits() {
    const CHILD: &str = "MIDNIGHT_INHERITED_CPU_POLICY_TEST";
    if std::env::var_os(CHILD).is_some() {
        setrlimit(
            Resource::Cpu,
            Rlimit {
                current: Some(60),
                maximum: Some(60),
            },
        )
        .unwrap();
        let mut worker = midnight_worker_platform::spawn_with_policy(
            std::path::Path::new(env!("CARGO_BIN_EXE_midnight-worker-probe")),
            ResourcePolicy {
                memory_bytes: 512 * 1024 * 1024,
                cpu: CpuBudget::NoAdditionalLimit,
            },
        )
        .unwrap();
        midnight_worker_sdk::read_policy_ready(&mut worker.output).unwrap();
        worker.process.kill().unwrap();
        // The running worker was killed: its exit names SIGKILL.
        assert_eq!(worker.process.wait_status().unwrap(), ExitStatus::Signal(9));
        assert!(!worker.process.wait().unwrap());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "no_additional_cpu_limit_preserves_inherited_hard_limits",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
