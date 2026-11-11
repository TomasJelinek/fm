use super::*;

fn record(name: &str) -> FileListRecord {
    FileListRecord {
        file_name: name.into(),
        name_part: "sample".into(),
        extension: "txt".into(),
        size: "12345".into(),
        date: "9. 11. 2026".into(),
        time: "17:45:06".into(),
        attributes: "AC".into(),
        drive: "C:".into(),
        path: "\\資料\\".into(),
        environment: BTreeMap::from([("USER".into(), "Alice".into())]),
    }
}

#[test]
fn expands_unicode_variables_and_maps_legacy_dos_tokens_to_full_values() {
    let template = FileListTemplate::parse(concat!(
        "$(FileName)|$(FileNamePart)|$(FileExtension)|$(FileSize)|",
        "$(FileDate)|$(FileTime)|$(FileAttributes)|$(FileDOSName)|",
        "$(Drive)|$(Path)|$(DOSPath)|$(LF)|$(CR)|$(CRLF)|$(TAB)"
    ))
    .unwrap();
    assert_eq!(
        template.render(&record("文档.txt"), &[]).unwrap(),
        "文档.txt|sample|txt|12345|9. 11. 2026|17:45:06|AC|文档.txt|C:|\\資料\\|\\資料\\|\n|\r|\r\n|\t"
    );
}

#[test]
fn names_and_max_are_case_insensitive_and_escaped_dollars_stay_literal() {
    let template = FileListTemplate::parse("$$30 $(fIlEnAmE:MaX) $$[USER] $(crlf)").unwrap();
    let row = record("文档.txt");
    let mut widths = Vec::new();
    template.measure(&row, &mut widths).unwrap();
    assert_eq!(
        template.render(&row, &widths).unwrap(),
        "$30 文档.txt $[USER] \r\n"
    );
}

#[test]
fn fixed_width_left_aligns_and_explicitly_truncates_at_unicode_scalar_boundaries() {
    let template = FileListTemplate::parse("[$(FileName:0002)][$ (literal)]");
    assert!(template.is_err());
    let template = FileListTemplate::parse("[$(FileName:0002)]/[$(FileExtension:5)]").unwrap();
    assert_eq!(
        template.render(&record("é🚀文.txt"), &[]).unwrap(),
        "[é🚀]/[txt  ]"
    );
}

#[test]
fn max_width_is_measured_per_occurrence_across_all_rows_without_truncation() {
    let template =
        FileListTemplate::parse("$(FileName:max)|$(FileSize:max)|$(FileName:max)\n").unwrap();
    let short = record("é");
    let mut long = record("你好.rs");
    long.size = "7".into();
    let mut widths = Vec::new();
    template.measure(&short, &mut widths).unwrap();
    template.measure(&long, &mut widths).unwrap();
    assert_eq!(widths, [5, 5, 5]);
    assert_eq!(
        template.render(&short, &widths).unwrap(),
        "é    |12345|é    \n"
    );
    assert_eq!(
        template.render(&long, &widths).unwrap(),
        "你好.rs|7    |你好.rs\n"
    );
    assert!(template.render(&long, &[]).is_err());
    assert!(template.render(&long, &[1, 1, 1]).is_err());
}

#[test]
fn environment_is_explicit_snapshot_data_and_never_recursively_expanded() {
    let template = FileListTemplate::parse("$[USER]|$[PAYLOAD]|$[USER]").unwrap();
    assert_eq!(
        template.environment_names().collect::<Vec<_>>(),
        ["PAYLOAD", "USER"]
    );
    let mut row = record("not expanded");
    assert!(template.render(&row, &[]).unwrap_err().contains("PAYLOAD"));
    row.environment
        .insert("PAYLOAD".into(), "$(FileName); & run 'quoted'".into());
    assert_eq!(
        template.render(&row, &[]).unwrap(),
        "Alice|$(FileName); & run 'quoted'|Alice"
    );
    row.environment.insert("USER".into(), String::new());
    assert_eq!(
        template.render(&row, &[]).unwrap(),
        "|$(FileName); & run 'quoted'|"
    );
}

#[test]
fn invalid_template_is_rejected_with_an_explanation_instead_of_partial_output() {
    for source in [
        "$",
        "$x",
        "$(",
        "$[",
        "$[]",
        "$(Unknown)",
        "$(FileName:0)",
        "$(FileName:10000)",
        "$(FileName:-2)",
        "$(FileName:+2)",
        "$(FileName:)",
        "$(FileName:00001)",
        "$(FileName::2)",
        "$(FileName:2:max)",
        "\0",
    ] {
        assert!(
            FileListTemplate::parse(source).is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn ordinary_unicode_long_names_and_paths_have_no_max_path_truncation() {
    let template = FileListTemplate::parse("$(Drive)$(Path)$(FileName)$(CRLF)").unwrap();
    let mut row = record(&"資料🚀".repeat(200));
    row.path = format!("\\{}\\", "long folder\\".repeat(100));
    let expected = format!("{}{}{}\r\n", row.drive, row.path, row.file_name);
    assert_eq!(template.render(&row, &[]).unwrap(), expected);
}

#[test]
fn resource_budgets_fail_explicitly_and_never_return_truncated_generated_rows() {
    assert!(FileListTemplate::parse(&"x".repeat(MAX_FILE_LIST_TEMPLATE_BYTES + 1)).is_err());
    let template = FileListTemplate::parse(&"$(FileName)".repeat(100)).unwrap();
    let row = record(&"x".repeat(MAX_FILE_LIST_ROW_BYTES / 50));
    assert!(template.render(&row, &[]).unwrap_err().contains("budget"));
    let template = FileListTemplate::parse("$(FileName:max)").unwrap();
    assert!(template.measure(&record("x"), &mut vec![1, 2]).is_err());
    assert!(template.render(&record("bad\0name"), &[8]).is_err());
}

#[test]
fn literals_empty_templates_and_zero_length_max_values_are_valid() {
    assert_eq!(
        FileListTemplate::parse("")
            .unwrap()
            .render(&record("x"), &[])
            .unwrap(),
        ""
    );
    assert_eq!(
        FileListTemplate::parse("plain 文字")
            .unwrap()
            .render(&record("x"), &[])
            .unwrap(),
        "plain 文字"
    );
    let template = FileListTemplate::parse("$(FileName:max)|").unwrap();
    let row = record("");
    let mut widths = Vec::new();
    template.measure(&row, &mut widths).unwrap();
    assert_eq!(widths, [0]);
    assert_eq!(template.render(&row, &widths).unwrap(), "|");
}
