use std::io::{BufRead, BufReader, Write};

#[test]
fn native_worker_denies_private_file_and_network_access_but_keeps_broker_pipes() {
    std::fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let secret = root.path().join("private-čínština-秘密.txt");
    std::fs::write(&secret, "host-only source").unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut worker = midnight_worker_platform::spawn(
        std::path::Path::new(env!("CARGO_BIN_EXE_midnight-worker-probe")),
        midnight_worker_platform::Limits {
            memory_bytes: 512 * 1024 * 1024,
            cpu_seconds: 5,
        },
    )
    .unwrap();
    let mut output = BufReader::new(worker.output);
    if let Err(error) = midnight_worker_sdk::read_ready(&mut output) {
        panic!("Startup: {error}; exit: {:?}", worker.process.wait());
    }
    writeln!(worker.input, "{}", secret.display()).unwrap();
    writeln!(worker.input, "{}", listener.local_addr().unwrap().port()).unwrap();
    worker.input.flush().unwrap();
    let mut reply = String::new();
    output.read_line(&mut reply).unwrap();
    assert_eq!(reply.trim(), "true true");
    assert!(worker.process.wait().unwrap());
}
