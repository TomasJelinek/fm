//! The release client over a scripted transport: limits, redirects, rate limits,
//! offline and resumed downloads. No test touches the network.
use crate::support::{Answer, FakeTransport};
use midnight_update::{
    client::{Endpoints, FetchError, UpdateClient},
    transport::{Head, TransportError, UrlPolicy},
};
use std::sync::{Arc, atomic::AtomicBool};

const API: &str = "https://api.github.com/repos/TomasJelinek/fm/releases?per_page=30";

fn client(transport: &Arc<FakeTransport>) -> UpdateClient {
    UpdateClient::new(transport.clone(), Endpoints::github())
}

#[test]
fn the_listing_is_read_from_githubs_api() {
    let transport = FakeTransport::new();
    transport.answer(
        API,
        Answer::ok(br#"[{"tag_name":"v0.5.1","html_url":"x"}]"#.to_vec()),
    );
    let releases = client(&transport)
        .releases(&AtomicBool::new(false))
        .unwrap();
    assert_eq!(releases.len(), 1);
    let request = transport.requests.lock().unwrap()[0].clone();
    assert_eq!(request.accept, "application/vnd.github+json");
}

#[test]
fn a_rate_limit_answer_reports_when_to_retry() {
    let transport = FakeTransport::new();
    transport.answer(
        API,
        Answer {
            head: Head {
                status: 403,
                rate_remaining: Some(0),
                rate_reset: Some(4_000_000_000),
                ..Head::default()
            },
            body: Vec::new(),
            cut_after: None,
        },
    );
    match client(&transport).releases(&AtomicBool::new(false)) {
        Err(FetchError::RateLimited { retry_at }) => assert_eq!(retry_at, 4_000_000_000),
        other => panic!("expected a rate limit, got {other:?}"),
    }
    let transport = FakeTransport::new();
    transport.answer(
        API,
        Answer {
            head: Head {
                status: 429,
                retry_after: Some(120),
                ..Head::default()
            },
            body: Vec::new(),
            cut_after: None,
        },
    );
    assert!(matches!(
        client(&transport).releases(&AtomicBool::new(false)),
        Err(FetchError::RateLimited { .. })
    ));
    // A server's wait is taken as it says, even one past the end of time.
    let transport = FakeTransport::new();
    transport.answer(
        API,
        Answer {
            head: Head {
                status: 429,
                retry_after: Some(u64::MAX),
                ..Head::default()
            },
            body: Vec::new(),
            cut_after: None,
        },
    );
    match client(&transport).releases(&AtomicBool::new(false)) {
        Err(FetchError::RateLimited { retry_at }) => assert_eq!(retry_at, u64::MAX),
        other => panic!("expected a rate limit, got {other:?}"),
    }
    // A 403 that is not the rate limit is an ordinary failure.
    let transport = FakeTransport::new();
    transport.answer(API, Answer::status(403));
    assert!(matches!(
        client(&transport).releases(&AtomicBool::new(false)),
        Err(FetchError::Status(403))
    ));
}

#[test]
fn offline_is_reported_as_unreachable() {
    let transport = FakeTransport::new();
    transport.set_offline(true);
    let error = client(&transport)
        .releases(&AtomicBool::new(false))
        .unwrap_err();
    assert!(matches!(
        error,
        FetchError::Transport(TransportError::Unreachable(_))
    ));
    assert!(error.to_string().contains("cannot be reached"));
}

#[test]
fn redirects_are_followed_only_to_github_over_https() {
    let transport = FakeTransport::new();
    let asset = "https://github.com/TomasJelinek/fm/releases/download/v0.5.1/SHA256SUMS";
    let stored = "https://objects.githubusercontent.com/github-production-release-asset/1?sig=x";
    transport.answer(asset, Answer::redirect(stored));
    transport.answer(stored, Answer::ok(b"sums".to_vec()));
    let body = client(&transport)
        .fetch_small(
            asset,
            "application/octet-stream",
            100,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(body, b"sums");

    for target in [
        "http://objects.githubusercontent.com/a",
        "https://evil.example/a",
        "https://objects.githubusercontent.com:8443/a",
        "https://user@github.com/a",
    ] {
        let transport = FakeTransport::new();
        transport.answer(asset, Answer::redirect(target));
        let error = client(&transport)
            .fetch_small(
                asset,
                "application/octet-stream",
                100,
                &AtomicBool::new(false),
            )
            .unwrap_err();
        assert!(
            matches!(error, FetchError::Refused(_)),
            "{target}: {error:?}"
        );
        assert_eq!(transport.request_count(), 1, "{target} was requested");
    }
}

#[test]
fn a_redirect_loop_ends() {
    let transport = FakeTransport::new();
    let asset = "https://github.com/a";
    transport.answer(asset, Answer::redirect("/a"));
    assert!(matches!(
        client(&transport).fetch_small(asset, "x", 100, &AtomicBool::new(false)),
        Err(FetchError::TooManyRedirects)
    ));
}

#[test]
fn small_files_have_a_size_limit() {
    let transport = FakeTransport::new();
    let asset = "https://github.com/big";
    transport.answer(asset, Answer::ok(vec![b'x'; 5000]));
    assert!(matches!(
        client(&transport).fetch_small(asset, "x", 4096, &AtomicBool::new(false)),
        Err(FetchError::TooLarge)
    ));
}

#[test]
fn a_broken_download_resumes_from_its_partial_file() {
    let transport = FakeTransport::new();
    let asset = "https://github.com/archive.zip";
    let content: Vec<u8> = (0..50_000u32).map(|value| value as u8).collect();
    transport.answer(
        asset,
        Answer {
            cut_after: Some(20_000),
            ..Answer::ok(content.clone())
        },
    );
    transport.answer(asset, Answer::ok(content.clone()));
    let folder = tempfile::tempdir().unwrap();
    let destination = folder.path().join("archive.zip");
    let mut seen = Vec::new();
    client(&transport)
        .download(
            asset,
            &destination,
            content.len() as u64,
            &AtomicBool::new(false),
            &mut |received, _| seen.push(received),
        )
        .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), content);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].range_from, 20_000);
    assert_eq!(seen.last(), Some(&(content.len() as u64)));
    assert!(!folder.path().join("archive.zip.part").exists());
}

#[test]
fn a_download_longer_than_announced_is_refused() {
    let transport = FakeTransport::new();
    let asset = "https://github.com/archive.zip";
    transport.answer(asset, Answer::ok(vec![1u8; 2000]));
    let folder = tempfile::tempdir().unwrap();
    let result = client(&transport).download(
        asset,
        &folder.path().join("archive.zip"),
        1000,
        &AtomicBool::new(false),
        &mut |_, _| {},
    );
    assert!(matches!(result, Err(FetchError::TooLarge)), "{result:?}");
}

#[test]
fn a_cancelled_download_stops() {
    let transport = FakeTransport::new();
    let asset = "https://github.com/archive.zip";
    transport.answer(asset, Answer::ok(vec![1u8; 2000]));
    let folder = tempfile::tempdir().unwrap();
    let result = client(&transport).download(
        asset,
        &folder.path().join("archive.zip"),
        2000,
        &AtomicBool::new(true),
        &mut |_, _| {},
    );
    assert!(matches!(result, Err(FetchError::Cancelled)), "{result:?}");
}

#[test]
fn the_github_policy_refuses_plain_http_and_other_hosts() {
    let policy = UrlPolicy::GitHub;
    assert!(policy.check("https://api.github.com/repos/x").is_ok());
    assert!(policy.check("http://api.github.com/repos/x").is_err());
    assert!(policy.check("http://127.0.0.1:8080/releases").is_err());
    assert!(
        policy
            .check("https://api.github.com.evil.example/")
            .is_err()
    );
    assert!(policy.check("https://api.github.com:444/").is_err());
    assert!(policy.check("ftp://github.com/").is_err());
    let loopback = UrlPolicy::LoopbackTest;
    assert!(loopback.check("http://127.0.0.1:8080/releases").is_ok());
    assert!(loopback.check("http://localhost:8080/releases").is_err());
    assert!(loopback.check("https://api.github.com/").is_err());
}

/// The test-only loopback endpoint exists in debug builds; a release build
/// (`cargo test --release`) proves it cannot be enabled there.
#[test]
fn only_debug_builds_accept_a_loopback_test_server() {
    let endpoints = Endpoints::loopback_test("http://127.0.0.1:1234");
    if cfg!(debug_assertions) {
        assert_eq!(endpoints.unwrap().policy, UrlPolicy::LoopbackTest);
    } else {
        // `from_environment` takes its test server only through `loopback_test`.
        assert!(
            endpoints.is_none(),
            "a release build accepted a plain-HTTP server"
        );
    }
    assert!(Endpoints::loopback_test("http://10.0.0.1:1234").is_none());
}
