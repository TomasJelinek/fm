//! Windows: the change endpoint protection leaves on a file, an alternate data stream added
//! with the file's LastWriteTime set back afterwards.
use super::*;

#[test]
fn an_added_stream_with_its_write_time_set_back_is_accepted_after_the_content_check() {
    let fixture = Fixture::new();
    let pin = fixture.pin();
    let exact = CapturedFile::capture(&fixture.path).unwrap();
    let before = modified(&fixture.path);
    std::thread::sleep(Duration::from_millis(50));
    let mut stream = fixture.path.clone().into_os_string();
    stream.push(":Zone.Identifier");
    fs::write(&stream, b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
    set_write_time(&fixture.path, before);
    let error = exact.revalidate().unwrap_err();
    assert!(error.0.contains("stage=open"), "{error}");
    pin.revalidate().unwrap();
    assert_eq!(pin.rereads(), 1);
    assert_eq!(pin.bytes(), MANIFEST);
    assert_eq!(fs::read(&fixture.path).unwrap(), MANIFEST);
}
