use super::*;
use std::cell::Cell;

#[test]
fn selected_explicit_automatic_stops_before_a_later_forced_rule() {
    use crate::{FileComparatorPolicySource, policy_test_support::FakePolicySource};
    use midnight_core::{
        ExternalCompareSessionCredit, FileComparatorRule, FileComparatorTarget, FilterMode,
    };

    struct Credit;
    impl ExternalCompareSessionCredit for Credit {}

    let source = FakePolicySource::new();
    let automatic = FileComparatorRule {
        enabled: true,
        mode: FilterMode::Masks,
        expression: "*".into(),
        case_sensitive: false,
        target: FileComparatorTarget::Automatic,
    };
    let mut later = automatic.clone();
    later.target = FileComparatorTarget::BuiltInText;
    let (snapshot, probe) = source.publish_rules(vec![automatic, later], Some(0));
    let policy = source.pin(&snapshot, std::sync::Arc::new(Credit)).unwrap();
    let paths = [PathBuf::from("before.c"), PathBuf::from("after.cpp")];
    assert!(matches!(
        select(policy.as_ref(), &paths),
        Target::BuiltIn(BuiltInTarget::Automatic)
    ));
    assert_eq!(*probe.seen.lock().unwrap(), vec![paths]);
}

#[test]
fn forced_text_does_not_discover_or_fall_back_to_images() {
    let discovered = Cell::new(false);
    let paths = [PathBuf::from("left.png"), PathBuf::from("right.png")];
    let result = plan(BuiltInTarget::Text, &paths, &[], || {
        discovered.set(true);
        Err("This decoder must never be queried.".into())
    });
    assert!(matches!(result, Ok(BuiltInPlan::Text)));
    assert!(!discovered.get());
}

#[test]
fn forced_images_report_discovery_failure_instead_of_reading_text() {
    let paths = [PathBuf::from("left.txt"), PathBuf::from("right.txt")];
    let result = plan(BuiltInTarget::Images, &paths, &[], || {
        Err("provider registration refused".into())
    });
    assert!(matches!(result, Err(message) if message.contains("provider registration refused")));
}

#[test]
fn forced_images_report_missing_coverage_instead_of_reading_text() {
    let paths = [PathBuf::from("left.png"), PathBuf::from("right.png")];
    let result = plan(BuiltInTarget::Images, &paths, &[], || Ok(Arc::from([])));
    assert!(matches!(result, Err(message) if message.contains("enabled installed image decoders")));
}

#[test]
fn automatic_keeps_text_available_when_optional_image_discovery_fails() {
    let paths = [PathBuf::from("left.txt"), PathBuf::from("right.txt")];
    let result = plan(BuiltInTarget::Automatic, &paths, &[], || {
        Err("optional image runtime absent".into())
    });
    assert!(matches!(result, Ok(BuiltInPlan::Text)));
}
