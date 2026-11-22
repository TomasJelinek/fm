//! GitHub's heading slugs (`link_slug.rs`).
use super::*;

#[test]
fn slugs_follow_github_lowercase_hyphens_and_dropped_punctuation() {
    let mut slugs = Slugger::default();
    for (heading, expected) in [
        ("Installation Guide", "installation-guide"),
        ("What's new in 2.0?", "whats-new-in-20"),
        ("C++ & Rust: a comparison", "c--rust-a-comparison"),
        ("snake_case and kebab-case", "snake_case-and-kebab-case"),
        ("Žluťoučký kůň", "žluťoučký-kůň"),
        ("日本語の見出し", "日本語の見出し"),
        ("  leading and trailing  ", "--leading-and-trailing--"),
        ("Emoji 🚀 rocket", "emoji--rocket"),
    ] {
        assert_eq!(slugs.slug(heading), expected, "{heading}");
    }
}

#[test]
fn repeated_headings_are_numbered_in_document_order() {
    let mut slugs = Slugger::default();
    let given: Vec<_> = ["Usage", "Usage", "Usage-1", "Usage"]
        .into_iter()
        .map(|heading| slugs.slug(heading))
        .collect();
    assert_eq!(given, ["usage", "usage-1", "usage-1-1", "usage-2"]);
}

#[test]
fn a_fragment_written_with_capitals_still_names_its_heading() {
    let mut slugs = Slugger::default();
    assert!(slugs.matches("Getting Started", "Getting-Started"));
    let mut slugs = Slugger::default();
    assert!(!slugs.matches("Getting Started", "getting_started"));
}
