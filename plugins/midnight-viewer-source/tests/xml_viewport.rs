#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

fn open(host: &mut Host, length: u64, name: &str) {
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(length),
            mode: MODE_SOURCE,
            display_name: name.into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
}
fn page(host: &mut Host, anchor: u64, lines: u32) -> TextPage {
    let id = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards: false,
            max_bytes: 8192,
            max_lines: lines,
        },
    );
    codec::decode(
        &host
            .finish()
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("bounded XML page")
            .payload,
    )
    .unwrap()
}
fn styled(page: &TextPage, needle: &str, role: u16) {
    let start = page.text.find(needle).unwrap();
    assert!(
        page.spans.iter().any(|s| s.role == role
            && s.start as usize <= start
            && s.end as usize >= start + needle.len()),
        "{needle:?}: {:?}",
        page.spans
    );
}
#[test]
fn xml_short_reads_restore_multiline_styles_and_exact_unicode_anchors() {
    let input = "<?xml version='1.0'?>\r\n<根 key='čeština'>\r\n<!-- one\r\ntwo -->\r\n<![CDATA[three\r\n<literal>]]>\r\n</根>";
    let mut host = Host::new(input.as_bytes(), entry);
    host.short_reads(7);
    open(&mut host, input.len() as u64, "sample.XAML");
    let first = page(&mut host, 0, 4);
    styled(&first, "根", STYLE_KEYWORD);
    styled(&first, "key", STYLE_TYPE);
    styled(&first, "'čeština'", STYLE_STRING);
    let continued = page(&mut host, input.find("two -->").unwrap() as u64, 3);
    assert!(continued.text.starts_with("two -->"));
    styled(&continued, "two -->", STYLE_COMMENT);
    styled(&continued, "<literal>]]>", STYLE_STRING);
    assert_eq!(
        continued.anchors.first().unwrap().source_offset,
        input.find("two -->").unwrap() as u64
    );
    assert_eq!(
        continued.anchors.last().unwrap().source_offset,
        continued.source.offset + continued.source.length
    );
    let cdata = page(&mut host, input.find("<literal>").unwrap() as u64, 3);
    assert!(cdata.text.starts_with("<literal>]]>"));
    styled(&cdata, "<literal>]]>", STYLE_STRING);
    styled(&cdata, "根", STYLE_KEYWORD);
    assert!(host.reads().iter().all(|read| read.length <= 65536));
}
#[test]
fn xml_utf16_projects_keep_styles_and_source_byte_mapping() {
    let text = "<Project>\r\n<Property 名='č😀'/>\r\n</Project>";
    let mut input = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        input.extend_from_slice(&unit.to_le_bytes());
    }
    let mut host = Host::new(&input, entry);
    host.short_reads(3);
    open(&mut host, input.len() as u64, "test.csproj");
    let page = page(&mut host, 0, 80);
    styled(&page, "Project", STYLE_KEYWORD);
    styled(&page, "名", STYLE_TYPE);
    styled(&page, "'č😀'", STYLE_STRING);
    assert_eq!(page.source.offset, 2);
    assert_eq!(
        page.anchors.last().unwrap().source_offset,
        input.len() as u64
    );
}
#[test]
fn xml_huge_lines_and_ten_gigabyte_seeks_keep_existing_viewport_budgets() {
    let input = format!("<root value='{}'/>\n", "文".repeat(40_000));
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len() as u64, "huge.xml");
    for anchor in [0, 60_000] {
        let before = host.reads().len();
        let page = page(&mut host, anchor, 80);
        assert!(page.text.len() <= 8192);
        assert!(!page.text.is_empty());
        if anchor == 0 {
            styled(&page, "root", STYLE_KEYWORD);
        } else {
            assert!(
                page.spans.is_empty(),
                "uncached distant context stays plain"
            );
        }
        assert!(
            host.reads()[before..]
                .iter()
                .map(|r| u64::from(r.length))
                .sum::<u64>()
                <= 65536
        );
    }
    const LENGTH: u64 = 10_000_000_000;
    let mut host = Host::new(b"<last/>\n", entry);
    host.base(LENGTH - 8);
    open(&mut host, LENGTH, "virtual.xml");
    assert_eq!(
        host.reads()
            .iter()
            .map(|r| u64::from(r.length))
            .sum::<u64>(),
        512
    );
    for anchor in [9_000_000_000, LENGTH - 8] {
        let before = host.reads().len();
        let page = page(&mut host, anchor, 80);
        assert!(!page.text.is_empty());
        assert!(page.text.len() <= 8192 && page.spans.is_empty());
        assert!(
            host.reads()[before..]
                .iter()
                .map(|r| u64::from(r.length))
                .sum::<u64>()
                <= 65536
        );
    }
}
