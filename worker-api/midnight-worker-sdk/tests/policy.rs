use midnight_worker_sdk::{
    CpuBudget, Limits, ResourcePolicy, read_policy_ready, read_ready, write_limits, write_policy,
};

#[test]
fn legacy_startup_bytes_and_validation_remain_exact() {
    let mut bytes = Vec::new();
    write_limits(
        &mut bytes,
        Limits {
            memory_bytes: 64 * 1024 * 1024,
            cpu_seconds: 15,
        },
    )
    .unwrap();
    assert_eq!(
        bytes,
        [b'M', b'W', b'S', b'1', 0, 0, 0, 4, 0, 0, 0, 0, 15, 0, 0, 0]
    );
    for cpu_seconds in [0, 61, u32::MAX] {
        assert!(
            Limits {
                memory_bytes: 64 * 1024 * 1024,
                cpu_seconds
            }
            .validate()
            .is_err()
        );
    }
    read_ready(&mut &b"MWR1"[..]).unwrap();
    assert!(read_ready(&mut &b"MWR2"[..]).is_err());
}

#[test]
fn unlimited_cpu_has_an_explicit_new_startup_and_ready_version() {
    let mut bytes = Vec::new();
    write_policy(
        &mut bytes,
        ResourcePolicy {
            memory_bytes: 64 * 1024 * 1024,
            cpu: CpuBudget::NoAdditionalLimit,
        },
    )
    .unwrap();
    assert_eq!(bytes, [b'M', b'W', b'S', b'2', 0, 0, 0, 4, 0, 0, 0, 0, 0]);
    read_policy_ready(&mut &b"MWR2"[..]).unwrap();
    assert!(read_policy_ready(&mut &b"MWR1"[..]).is_err());
    bytes.clear();
    write_policy(
        &mut bytes,
        ResourcePolicy {
            memory_bytes: 64 * 1024 * 1024,
            cpu: CpuBudget::Seconds(15),
        },
    )
    .unwrap();
    assert_eq!(
        bytes,
        [
            b'M', b'W', b'S', b'2', 0, 0, 0, 4, 0, 0, 0, 0, 1, 15, 0, 0, 0
        ]
    );
}

#[test]
fn malformed_policy_is_rejected_before_any_startup_bytes_are_written() {
    for policy in [
        ResourcePolicy {
            memory_bytes: 0,
            cpu: CpuBudget::NoAdditionalLimit,
        },
        ResourcePolicy {
            memory_bytes: u64::MAX,
            cpu: CpuBudget::NoAdditionalLimit,
        },
        ResourcePolicy {
            memory_bytes: 64 * 1024 * 1024,
            cpu: CpuBudget::Seconds(0),
        },
        ResourcePolicy {
            memory_bytes: 64 * 1024 * 1024,
            cpu: CpuBudget::Seconds(61),
        },
    ] {
        let mut bytes = Vec::new();
        assert!(write_policy(&mut bytes, policy).is_err());
        assert!(bytes.is_empty());
    }
}
