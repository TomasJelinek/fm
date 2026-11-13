//! The same graph used for sysfs admission, with deterministic metadata probes.
use super::*;
use std::{cell::Cell, collections::HashMap, io};

fn path(name: &str) -> PathBuf {
    Path::new("/sys/devices").join(name)
}

fn classify(nodes: &[(&str, BlockNode)], root: &str) -> StorageKind {
    let nodes: HashMap<_, _> = nodes
        .iter()
        .map(|(name, node)| (path(name), node.clone()))
        .collect();
    classify_graph(path(root), &|| false, &mut |name| nodes.get(name).cloned())
}

#[test]
fn local_lvm_and_encryption_walk_backing_devices_and_normalize_partitions() {
    for kind in [StorageKind::LocalSsd, StorageKind::Rotational] {
        assert_eq!(
            classify(
                &[
                    ("dm-1", BlockNode::Backing(vec![path("dm-0")])),
                    ("dm-0", BlockNode::Backing(vec![path("sda3")])),
                    ("sda3", BlockNode::Partition(path("sda"))),
                    ("sda", BlockNode::Physical(kind)),
                ],
                "dm-1",
            ),
            kind
        );
    }
}

#[test]
fn every_backing_leaf_must_be_known_and_mixed_local_media_is_serial() {
    for (second, expected) in [
        (StorageKind::LocalSsd, StorageKind::LocalSsd),
        (StorageKind::Rotational, StorageKind::Rotational),
        (StorageKind::Unknown, StorageKind::Unknown),
        (StorageKind::Network, StorageKind::Unknown),
    ] {
        assert_eq!(
            classify(
                &[
                    (
                        "dm-0",
                        BlockNode::Backing(vec![path("nvme0n1"), path("sda")])
                    ),
                    ("nvme0n1", BlockNode::Physical(StorageKind::LocalSsd)),
                    ("sda", BlockNode::Physical(second)),
                ],
                "dm-0",
            ),
            expected
        );
    }
}

#[test]
fn shared_physical_backing_is_probed_once_without_becoming_a_false_cycle() {
    let nodes: HashMap<_, _> = [
        (
            path("dm-2"),
            BlockNode::Backing(vec![path("dm-0"), path("dm-1")]),
        ),
        (path("dm-0"), BlockNode::Backing(vec![path("sda1")])),
        (path("dm-1"), BlockNode::Backing(vec![path("sda2")])),
        (path("sda1"), BlockNode::Partition(path("sda"))),
        (path("sda2"), BlockNode::Partition(path("sda"))),
        (path("sda"), BlockNode::Physical(StorageKind::LocalSsd)),
    ]
    .into();
    let mut probes = HashMap::<PathBuf, usize>::new();
    let result = classify_graph(path("dm-2"), &|| false, &mut |name| {
        *probes.entry(name.to_owned()).or_default() += 1;
        nodes.get(name).cloned()
    });
    assert_eq!(result, StorageKind::LocalSsd);
    assert_eq!(probes[&path("sda")], 1);
}

#[test]
fn missing_empty_or_cyclic_backing_never_becomes_local() {
    assert_eq!(classify(&[], "missing"), StorageKind::Unknown);
    assert_eq!(
        classify(&[("dm-0", BlockNode::Backing(Vec::new()))], "dm-0"),
        StorageKind::Unknown
    );
    assert_eq!(
        classify(
            &[("dm-0", BlockNode::Backing(vec![path("missing")]))],
            "dm-0"
        ),
        StorageKind::Unknown
    );
    assert_eq!(
        classify(
            &[
                ("dm-0", BlockNode::Backing(vec![path("dm-1")])),
                ("dm-1", BlockNode::Backing(vec![path("dm-0")])),
            ],
            "dm-0",
        ),
        StorageKind::Unknown
    );
    assert_eq!(
        classify(&[("sda1", BlockNode::Partition(path("sda1")))], "sda1"),
        StorageKind::Unknown
    );
}

#[test]
fn graph_depth_and_total_node_budgets_bound_metadata_probes() {
    let depth_probes = Cell::new(0usize);
    let result = classify_graph(path("0"), &|| false, &mut |name| {
        depth_probes.set(depth_probes.get() + 1);
        let index: usize = name.file_name()?.to_str()?.parse().ok()?;
        Some(BlockNode::Backing(vec![path(&(index + 1).to_string())]))
    });
    assert_eq!(result, StorageKind::Unknown);
    assert!(depth_probes.get() <= 33);

    let node_probes = Cell::new(0usize);
    let result = classify_graph(path("root"), &|| false, &mut |name| {
        node_probes.set(node_probes.get() + 1);
        Some(if name == path("root") {
            BlockNode::Backing((0..128).map(|index| path(&index.to_string())).collect())
        } else {
            BlockNode::Physical(StorageKind::LocalSsd)
        })
    });
    assert_eq!(result, StorageKind::Unknown);
    assert!(node_probes.get() <= 128);
}

#[test]
fn canceled_graph_stops_before_probe_or_after_cancellation_during_probe() {
    assert_eq!(
        classify_graph(path("root"), &|| true, &mut |_| panic!("canceled probe")),
        StorageKind::Unknown
    );
    let cancelled = Cell::new(false);
    assert_eq!(
        classify_graph(path("root"), &|| cancelled.get(), &mut |_| {
            cancelled.set(true);
            Some(BlockNode::Physical(StorageKind::LocalSsd))
        }),
        StorageKind::Unknown
    );
    let probes = Cell::new(0usize);
    let cancelled = Cell::new(false);
    assert_eq!(
        classify_graph(path("partition"), &|| cancelled.get(), &mut |_| {
            probes.set(probes.get() + 1);
            cancelled.set(true);
            Some(BlockNode::Partition(path("disk")))
        }),
        StorageKind::Unknown
    );
    assert_eq!(probes.get(), 1);
}

#[test]
fn backing_iteration_is_bounded_even_when_the_metadata_iterator_never_ends() {
    let scanned = Cell::new(0usize);
    let entries = std::iter::from_fn(|| {
        scanned.set(scanned.get() + 1);
        Some(Ok(path("sda")))
    });
    assert!(collect_backing(entries, &|| false).is_none());
    assert!(scanned.get() <= 129);
    assert!(
        collect_backing(
            std::iter::once(Err(io::Error::other("unreadable"))),
            &|| false
        )
        .is_none()
    );
    assert!(collect_backing(std::iter::once(Ok(PathBuf::from("/outside"))), &|| false).is_none());
    assert!(collect_backing(std::iter::once(Ok(path("sda"))), &|| true).is_none());
}
