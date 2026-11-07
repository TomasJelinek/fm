//! Default mode is content based, including zero-delimited compiler arguments.
use midnight_plugin_host::{CapturedFile, api::viewer};

#[test]
fn zero_delimited_flags_open_as_text_and_keep_original_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("flags.nul");
    let content = b"-IC:\\sample\\include\0-DUNICODE=1\0-DUSE_FEATURE=1\0";
    std::fs::write(&path, content).unwrap();
    let source = CapturedFile::capture(&path).unwrap();
    assert_eq!(source.preferred_basic_mode().unwrap(), viewer::MODE_TEXT);
    assert_eq!(source.read_at(0, content.len() as u32).unwrap(), content);
}
