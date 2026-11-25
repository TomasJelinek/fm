use super::*;
use crate::{
    adapter::{NodeKind, Record},
    api::{Address, AddressEncoding},
};
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

/// An adapter call: the target and the password offered, none for a plain activation.
type Call = (Vec<u8>, Option<Vec<u8>>);

thread_local! {
    static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
    /// The passwords the fixture's servers accept; any other sign-in is rejected.
    static ACCEPTED: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

/// Targets are `smb://<server>/<share>` keys; the scope is their server.
struct Fixture;
impl Adapter for Fixture {
    type Cursor = ();
    fn root() -> Record {
        Record {
            target: Target {
                kind: NodeKind::Root,
                key: b"network:///".to_vec(),
            },
            display: "Network".into(),
        }
    }
    fn validate(_: &Target) -> Result<(), Failure> {
        Ok(())
    }
    fn manual(_: &str) -> Result<Record, Failure> {
        unreachable!()
    }
    fn begin(_: &Target, _: &Context<'_>) -> Result<(), Failure> {
        unreachable!()
    }
    fn next(_: &mut (), _: &Context<'_>) -> Result<Option<Record>, Failure> {
        unreachable!()
    }
    fn activate(target: &Target, _: &Context<'_>) -> Result<ActivationTarget, Failure> {
        CALLS.with_borrow_mut(|calls| calls.push((target.key.clone(), None)));
        Ok(within(target))
    }
    fn sign_in(
        target: &Target,
        credentials: &NetworkCredentials,
        _: &Context<'_>,
    ) -> Result<ActivationTarget, Failure> {
        let password = credentials.password().to_vec();
        CALLS.with_borrow_mut(|calls| calls.push((target.key.clone(), Some(password.clone()))));
        if ACCEPTED.with_borrow(|accepted| accepted.contains(&password)) {
            Ok(within(target))
        } else {
            Err(Failure::new(
                ErrorCode::AuthenticationRequired,
                "This network location requires sign-in.",
            ))
        }
    }
    fn sign_in_scope(target: &Target) -> Option<Vec<u8>> {
        let rest = target.key.strip_prefix(b"smb://")?;
        let end = rest
            .iter()
            .position(|byte| *byte == b'/')
            .unwrap_or(rest.len());
        (target.kind != NodeKind::Container).then(|| rest[..end].to_vec())
    }
}

fn within(target: &Target) -> ActivationTarget {
    ActivationTarget::WithinProvider(Address {
        encoding: AddressEncoding::Opaque,
        bytes: target.key.clone(),
    })
}

fn share(key: &str) -> Target {
    Target {
        kind: NodeKind::Share,
        key: key.as_bytes().to_vec(),
    }
}

fn credentials(password: &str) -> NetworkCredentials {
    NetworkCredentials::new(
        "user".into(),
        "WORKGROUP".into(),
        password.as_bytes().to_vec(),
    )
}

fn accept(passwords: &[&str]) {
    ACCEPTED.with_borrow_mut(|accepted| {
        *accepted = passwords
            .iter()
            .map(|password| password.as_bytes().to_vec())
            .collect();
    });
}

/// The calls since the previous take, with each offered password as text.
fn calls() -> Vec<(String, Option<String>)> {
    CALLS
        .with_borrow_mut(std::mem::take)
        .into_iter()
        .map(|(key, password)| {
            (
                String::from_utf8(key).unwrap(),
                password.map(|password| String::from_utf8(password).unwrap()),
            )
        })
        .collect()
}

fn activate(
    sign_ins: &SignIns,
    target: &Target,
    submitted: Option<&NetworkCredentials>,
) -> Result<bool, ErrorCode> {
    let context = Context {
        cancelled: &|| false,
        deadline: Instant::now() + Duration::from_secs(2),
    };
    sign_ins
        .activate::<Fixture>(target, submitted, &context)
        .map(|(_, offered)| offered)
        .map_err(|error| error.code)
}

#[test]
fn a_sign_in_answers_later_activations_on_the_same_server_only() {
    let sign_ins = SignIns::default();
    accept(&["secret"]);
    // No sign-in yet: a plain activation, which this server answers by asking.
    assert_eq!(
        activate(&sign_ins, &share("smb://nas/one"), None),
        Ok(false)
    );
    assert_eq!(
        activate(
            &sign_ins,
            &share("smb://nas/one"),
            Some(&credentials("secret"))
        ),
        Ok(true)
    );
    assert_eq!(
        calls(),
        [
            ("smb://nas/one".into(), None),
            ("smb://nas/one".into(), Some("secret".into())),
        ]
    );
    // Another share and the server itself: the remembered sign-in, without a dialog.
    assert_eq!(activate(&sign_ins, &share("smb://nas/two"), None), Ok(true));
    let server = Target {
        kind: NodeKind::Server,
        key: b"smb://nas".to_vec(),
    };
    assert_eq!(activate(&sign_ins, &server, None), Ok(true));
    // Another server never receives it, nor does a container.
    assert_eq!(
        activate(&sign_ins, &share("smb://other/one"), None),
        Ok(false)
    );
    let container = Target {
        kind: NodeKind::Container,
        key: b"smb://nas/".to_vec(),
    };
    assert_eq!(activate(&sign_ins, &container, None), Ok(false));
    assert_eq!(
        calls(),
        [
            ("smb://nas/two".into(), Some("secret".into())),
            ("smb://nas".into(), Some("secret".into())),
            ("smb://other/one".into(), None),
            ("smb://nas/".into(), None),
        ]
    );
}

#[test]
fn a_rejected_sign_in_is_not_remembered_and_a_rejected_memory_is_forgotten() {
    let sign_ins = SignIns::default();
    accept(&["secret"]);
    assert_eq!(
        activate(
            &sign_ins,
            &share("smb://nas/one"),
            Some(&credentials("wrong"))
        ),
        Err(ErrorCode::AuthenticationRequired)
    );
    assert_eq!(
        activate(&sign_ins, &share("smb://nas/one"), None),
        Ok(false)
    );
    assert_eq!(
        activate(
            &sign_ins,
            &share("smb://nas/one"),
            Some(&credentials("secret"))
        ),
        Ok(true)
    );
    calls();
    // The password changes on the server: the remembered one is rejected once, then
    // forgotten, so that the next activation asks (the dialog) instead of retrying it.
    accept(&["changed"]);
    assert_eq!(
        activate(&sign_ins, &share("smb://nas/two"), None),
        Err(ErrorCode::AuthenticationRequired)
    );
    assert_eq!(
        activate(&sign_ins, &share("smb://nas/two"), None),
        Ok(false)
    );
    assert_eq!(
        calls(),
        [
            ("smb://nas/two".into(), Some("secret".into())),
            ("smb://nas/two".into(), None),
        ]
    );
    // A new sign-in replaces it.
    assert_eq!(
        activate(
            &sign_ins,
            &share("smb://nas/two"),
            Some(&credentials("changed"))
        ),
        Ok(true)
    );
    assert_eq!(activate(&sign_ins, &share("smb://nas/one"), None), Ok(true));
    assert_eq!(calls().last().unwrap().1.as_deref(), Some("changed"));
}

#[test]
fn a_newer_sign_in_survives_the_rejection_of_an_older_one() {
    let sign_ins = SignIns::default();
    sign_ins.remember(b"nas".to_vec(), &credentials("old"));
    let old = sign_ins.get(b"nas").unwrap();
    sign_ins.remember(b"nas".to_vec(), &credentials("new"));
    sign_ins.forget(b"nas", &old);
    assert_eq!(sign_ins.get(b"nas").unwrap().password(), b"new");
}

#[test]
fn at_most_thirty_two_servers_keep_a_sign_in() {
    let sign_ins = SignIns::default();
    for server in 0..=MAX_SERVERS {
        sign_ins.remember(
            format!("server{server}").into_bytes(),
            &credentials("secret"),
        );
    }
    assert!(sign_ins.get(b"server0").is_none());
    assert!(sign_ins.get(b"server1").is_some());
    assert!(
        sign_ins
            .get(format!("server{MAX_SERVERS}").as_bytes())
            .is_some()
    );
    assert_eq!(lock(&sign_ins.remembered).len(), MAX_SERVERS);
}
