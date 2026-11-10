use super::*;
use std::collections::HashSet;

fn request(names: &[&str], template: &str) -> FileListRequest {
    let entries: Arc<[Entry]> = names
        .iter()
        .enumerate()
        .map(|(index, name)| Entry::new(EntryId(index as u64 + 1), (*name).into(), EntryKind::File))
        .collect::<Vec<_>>()
        .into();
    let selected: Arc<HashSet<_>> = Arc::new(entries.iter().map(|e| e.id).collect());
    FileListRequest {
        id: DesktopRequestId(1),
        generation: 1,
        template: template.into(),
        destination: FileListDestination::Preview,
        capture: Arc::new(FileListCapture {
            directory: String::new(),
            source: FileListSource::Archive {
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                selected,
                fallback: None,
            },
        }),
    }
}

#[test]
fn preview_is_bounded_and_max_width_measures_rows_outside_preview() {
    let names = (0..20)
        .map(|index| format!("file-{index}.txt"))
        .collect::<Vec<_>>();
    let mut names = names.iter().map(String::as_str).collect::<Vec<_>>();
    names[19] = "longest-name-outside-preview.txt";
    let result = generate(
        &request(&names, "$(FileName:max)|$(LF)"),
        &Cancellation::default(),
    )
    .unwrap();
    let Generated::Preview {
        text,
        items,
        truncated,
    } = result
    else {
        panic!()
    };
    assert_eq!(items, 20);
    assert!(truncated);
    assert_eq!(text.lines().count(), 12);
    assert_eq!(
        text.lines().next().unwrap(),
        "file-0.txt                      |"
    );
    assert!(!text.contains("longest-name"));
}

#[test]
fn preview_truncation_preserves_unicode_boundaries_and_total_count() {
    let name = "文".repeat(10_000);
    let result = generate(&request(&[&name], "$(FileName)"), &Cancellation::default()).unwrap();
    let Generated::Preview {
        text,
        items,
        truncated,
    } = result
    else {
        panic!()
    };
    assert_eq!(items, 1);
    assert!(truncated);
    assert_eq!(text.len(), 16_383);
    assert!(text.chars().all(|value| value == '文'));
}

#[test]
fn malformed_templates_empty_selection_and_cancelled_generation_fail_without_output() {
    assert!(generate(&request(&["file.txt"], ""), &Cancellation::default()).is_err());
    assert!(
        generate(
            &request(&["file.txt"], "$(NotAVariable)"),
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(generate(&request(&[], "$(FileName)"), &Cancellation::default()).is_err());
    let cancel = Cancellation::default();
    cancel.cancel();
    let mut request = request(&["file.txt"], "$(FileName)");
    request.destination = FileListDestination::Clipboard;
    assert!(generate(&request, &cancel).is_err());
}

#[test]
fn unicode_output_and_explicit_line_endings_are_exact() {
    let request = request(
        &["Žluťoučký.txt", "文档.rs"],
        "$(FileName)$(TAB)$(FileExtension)$(CRLF)",
    );
    let Generated::Preview {
        text,
        items,
        truncated,
    } = generate(&request, &Cancellation::default()).unwrap()
    else {
        panic!()
    };
    assert_eq!(text, "Žluťoučký.txt\ttxt\r\n文档.rs\trs\r\n");
    assert_eq!(items, 2);
    assert!(!truncated);
}
