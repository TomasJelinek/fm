use midnight_core::{MountRef, NodeRef, NodeToken, ProviderId, ProviderInstanceId, StoredLocation};
use std::{
    collections::HashSet,
    num::{NonZeroU32, NonZeroU64},
    sync::Arc,
};

#[test]
fn package_ids_reject_empty_path_and_unbounded_values_without_normalizing() {
    for invalid in [
        "",
        "../native",
        "native/windows",
        "native\\windows",
        "a:b",
        "a b",
        "nátive",
    ] {
        assert!(ProviderId::new(invalid).is_err(), "{invalid:?}");
    }
    assert!(ProviderId::new(&"a".repeat(129)).is_err());
    assert_eq!(
        ProviderId::new("Native.Windows-1").unwrap().as_str(),
        "Native.Windows-1"
    );
    assert_ne!(ProviderId::new("A").unwrap(), ProviderId::new("a").unwrap());
}

#[test]
fn identical_tokens_in_distinct_mount_lifetimes_are_distinct_nodes() {
    let one = NonZeroU64::new(1).unwrap();
    let two = NonZeroU64::new(2).unwrap();
    let base = MountRef {
        instance: ProviderInstanceId(one),
        mount: one,
        generation: one,
    };
    let references = [
        base,
        MountRef {
            instance: ProviderInstanceId(two),
            ..base
        },
        MountRef { mount: two, ..base },
        MountRef {
            generation: two,
            ..base
        },
    ];
    let nodes = references.map(|mount| NodeRef {
        mount,
        token: NodeToken::new(b"opaque").unwrap(),
    });
    assert_eq!(nodes.into_iter().collect::<HashSet<_>>().len(), 4);
}

#[test]
fn stored_addresses_preserve_long_and_non_unicode_provider_bytes() {
    let provider = ProviderId::new("native.windows").unwrap();
    let mut bytes = vec![0xff; 100_000];
    bytes.extend_from_slice(b"a/A/\\\0");
    let location = StoredLocation::new(
        provider,
        NonZeroU32::new(7).unwrap(),
        Arc::from(bytes.clone()),
    );
    assert_eq!(location.address(), bytes);
    assert_eq!(location.codec_version().get(), 7);
    let shorter = StoredLocation::new(
        location.provider().clone(),
        location.codec_version(),
        Arc::from(&bytes[..100_000]),
    );
    assert_ne!(location, shorter);
    assert!(!format!("{location:?}").contains("a/A"));
}

#[test]
fn tokens_are_bounded_opaque_and_debug_does_not_reveal_their_bytes() {
    assert!(NodeToken::new(&[]).is_err());
    assert!(NodeToken::new(&vec![0; NodeToken::MAX_BYTES + 1]).is_err());
    assert!(NodeToken::new(&vec![0; NodeToken::MAX_BYTES]).is_ok());
    let token = NodeToken::new(b"private-object").unwrap();
    assert_eq!(token.as_bytes(), b"private-object");
    assert!(!format!("{token:?}").contains("private-object"));
}
