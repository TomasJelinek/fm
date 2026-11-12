use midnight_core::*;
use midnight_platform_common::filter::{CompiledFilter, CompiledPanelFilter};
use std::path::PathBuf;

fn expression(source: &str) -> PanelFilter {
    PanelFilter {
        enabled: true,
        mode: FilterMode::Expression,
        expression: source.into(),
        case_sensitive: false,
    }
}

#[test]
fn metadata_expression_evaluates_captured_entry_and_filters_directories_explicitly() {
    let filter = CompiledPanelFilter::new(&expression("kind = file and size >= 10MiB")).unwrap();
    let mut entry = Entry::new(EntryId(1), PathBuf::from("same.txt"), EntryKind::File);
    entry.metadata.size = Some(10 * 1024 * 1024);
    assert!(filter.matches_entry(&entry));
    entry.metadata.size = Some(1);
    assert!(!filter.matches_entry(&entry));
    entry.metadata.size = None;
    assert!(!filter.matches_entry(&entry));
    entry.kind = EntryKind::Directory;
    assert!(!filter.matches_entry(&entry));
}

#[test]
fn legacy_modes_keep_directories_and_disabled_expressions_match_everything() {
    let folder = Entry::new(EntryId(1), "folder".into(), EntryKind::Directory);
    for mode in [FilterMode::Masks, FilterMode::Regex] {
        let filter = PanelFilter {
            enabled: true,
            mode,
            expression: "nomatch".into(),
            case_sensitive: true,
        };
        assert!(
            CompiledPanelFilter::new(&filter)
                .unwrap()
                .matches_entry(&folder)
        );
    }
    let mut filter = expression("kind = file");
    filter.enabled = false;
    assert!(
        CompiledPanelFilter::new(&filter)
            .unwrap()
            .matches_entry(&folder)
    );
}

#[test]
fn name_only_consumers_refuse_metadata_expressions_even_when_disabled() {
    let mut filter = expression("missing(attributes)");
    for enabled in [true, false] {
        filter.enabled = enabled;
        assert!(CompiledFilter::new(&filter).is_err());
    }
    let rule = FilenameColorRule {
        mode: FilterMode::Expression,
        ..Default::default()
    };
    assert!(validate_filename_colors(&[rule]).is_err());
}

#[test]
fn missing_attributes_and_negation_do_not_treat_unknown_as_clear() {
    let entry = Entry::new(EntryId(1), "known.txt".into(), EntryKind::File);
    assert!(
        !CompiledPanelFilter::new(&expression("not attributes has hidden"))
            .unwrap()
            .matches_entry(&entry)
    );
    assert!(
        CompiledPanelFilter::new(&expression(
            "missing(attributes) or not attributes has hidden"
        ))
        .unwrap()
        .matches_entry(&entry)
    );
}
