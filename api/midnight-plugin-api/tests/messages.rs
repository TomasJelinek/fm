use midnight_plugin_api::{
    codec::{self, Model},
    messages::*,
    viewer::*,
};

fn rejected(model: &impl Model) {
    assert!(codec::encode(model).is_err());
}

#[test]
fn requests_validate_identity_modes_and_work_budgets() {
    let open = Open {
        reader: 42,
        length: Some(1 << 40),
        revision: 7,
        mode: MODE_TEXT,
        display_name: "文档.txt".into(),
    };
    assert!(codec::encode(&open).is_ok());
    rejected(&Open {
        reader: 0,
        ..open.clone()
    });
    rejected(&Open { mode: 99, ..open });
    rejected(&Viewport {
        source_anchor: 0,
        backwards: false,
        max_bytes: u32::MAX,
        max_lines: 1,
    });
    rejected(&Viewport {
        source_anchor: 0,
        backwards: false,
        max_bytes: 1,
        max_lines: 0,
    });
    rejected(&Search {
        query: String::new(),
        source_anchor: 0,
        backwards: false,
        case_sensitive: false,
    });
    rejected(&Search {
        query: "x".repeat(4097),
        source_anchor: 0,
        backwards: false,
        case_sensitive: false,
    });
}

#[test]
fn reads_are_bounded_and_never_wrap_u64_addresses() {
    let read = ReadAt {
        reader: 1,
        revision: 1,
        offset: 1 << 40,
        length: 256 * 1024,
    };
    assert!(codec::encode(&read).is_ok());
    rejected(&ReadAt {
        offset: u64::MAX,
        ..read.clone()
    });
    rejected(&ReadAt {
        reader: 0,
        ..read.clone()
    });
    rejected(&ReadAt {
        length: 0,
        ..read.clone()
    });
    rejected(&ReadAt {
        length: u32::MAX,
        ..read
    });
    rejected(&ReadReply {
        revision: 1,
        offset: u64::MAX,
        bytes: 1,
        eof: true,
        status: 0,
    });
    rejected(&CloseReader { reader: 0 });
}

#[test]
fn incomplete_progress_and_short_reads_are_explicit_without_fabricated_totals() {
    assert!(
        codec::encode(&Progress {
            completed: 10,
            total: None,
            phase: 1
        })
        .is_ok()
    );
    rejected(&Progress {
        completed: 11,
        total: Some(10),
        phase: 1,
    });
    let partial = ReadReply {
        revision: 2,
        offset: 4096,
        bytes: 32,
        eof: false,
        status: 0,
    };
    let decoded: ReadReply = codec::decode(&codec::encode(&partial).unwrap()).unwrap();
    assert!(!decoded.eof);
    assert_eq!(decoded.bytes, 32);
    rejected(&ReadReply {
        status: 1,
        ..partial
    });
    rejected(&Opened {
        mode: MODE_TEXT,
        length: Some(1),
        initial_anchor: 2,
        estimated_lines: None,
    });
    rejected(&Failure {
        code: 1,
        message: "x".repeat(4097),
    });
    rejected(&SearchMatch {
        source: SourceRange {
            offset: u64::MAX,
            length: 1,
        },
        preview: String::new(),
    });
    rejected(&SearchMatch {
        source: SourceRange {
            offset: 0,
            length: 0,
        },
        preview: String::new(),
    });
}
