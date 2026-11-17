use midnight_thumbnail_sdk::{
    Broker,
    api::{self, Message},
};
use std::io::{Cursor, Read, Seek, SeekFrom};

#[test]
fn broker_reads_only_bounded_ranges_and_rejects_seek_past_source() {
    let mut reply = Vec::new();
    api::write_message(&mut reply, &Message::Data(vec![3, 4])).unwrap();
    let mut input = Cursor::new(reply);
    let mut output = Vec::new();
    let mut broker = Broker::new(&mut input, &mut output, 4);
    assert_eq!(broker.seek(SeekFrom::End(-2)).unwrap(), 2);
    let mut bytes = [0; 64];
    assert_eq!(broker.read(&mut bytes).unwrap(), 2);
    assert_eq!(&bytes[..2], &[3, 4]);
    assert_eq!(broker.read(&mut bytes).unwrap(), 0);
    assert!(broker.seek(SeekFrom::End(1)).is_err());
    assert!(broker.seek(SeekFrom::Current(i64::MIN)).is_err());
    assert_eq!(
        api::read_message(&mut Cursor::new(output)).unwrap(),
        Message::Read {
            offset: 2,
            length: 2
        }
    );
}

#[test]
fn broker_requires_exactly_the_requested_length() {
    for reply in [
        Message::Data(vec![]),
        // A short nonempty reply means the source changed; it is not a partial read.
        Message::Data(vec![1]),
        Message::Data(vec![1; 3]),
        Message::Failed,
    ] {
        let mut input = Vec::new();
        api::write_message(&mut input, &reply).unwrap();
        let mut input = Cursor::new(input);
        let mut output = Vec::new();
        assert!(
            Broker::new(&mut input, &mut output, 2)
                .read(&mut [0; 2])
                .is_err(),
            "{reply:?}"
        );
    }
}
