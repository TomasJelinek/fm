#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::{midnight_plugin_entry_v1 as entry, syntax};
use support::Host;

fn open(host: &mut Host, length: usize, name: &str, mode: u16) {
    let id = host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(length as u64),
            mode,
            display_name: name.into(),
        },
    );
    let events = host.finish();
    let kinds: Vec<_> = events
        .iter()
        .filter(|e| e.id == id)
        .map(|e| e.kind)
        .collect();
    assert_eq!(kinds, [abi::EVENT_OPENED, abi::EVENT_COMPLETED]);
    let opened: Opened = codec::decode(&events[0].payload).unwrap();
    assert_eq!(opened.mode, mode);
    assert_eq!(opened.length, Some(length as u64));
}

fn page(host: &mut Host, target: &Viewport) -> TextPage {
    let id = host.submit(abi::REQUEST_VIEWPORT, target);
    let events = host.finish();
    let events: Vec<_> = events.iter().filter(|e| e.id == id).collect();
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [abi::EVENT_TEXT_PAGE, abi::EVENT_COMPLETED]
    );
    let page: TextPage = codec::decode(&events[0].payload).unwrap();
    page.validate().unwrap();
    assert_eq!(page.revision, 3);
    page
}

fn viewport(anchor: u64, backwards: bool) -> Viewport {
    Viewport {
        source_anchor: anchor,
        backwards,
        max_bytes: 8192,
        max_lines: 80,
    }
}

fn styled(text: &str, spans: &[Span], needle: &str, role: u16) {
    let start = text.find(needle).expect("owned fixture token");
    assert!(
        spans.iter().any(|s| s.role == role
            && s.start as usize <= start
            && s.end as usize >= start + needle.len()),
        "missing role {role} for {needle:?}"
    );
}

fn plain(text: &str, spans: &[Span], needle: &str) {
    let start = text.find(needle).unwrap();
    let end = start + needle.len();
    assert!(
        !spans
            .iter()
            .any(|s| (s.start as usize) < end && s.end as usize > start),
        "identifier {needle:?} must stay plain"
    );
}

#[test]
fn manifest_and_usda_defaults_advertise_real_profiles_without_binary_usd() {
    let manifest = include_str!("../plugin.toml");
    let extensions = manifest
        .lines()
        .find(|l| l.starts_with("extensions ="))
        .unwrap();
    for (extension, language) in [
        ("manifest", syntax::detect("sample.xml", "").unwrap()),
        (
            "usda",
            syntax::detect("sample.usda", "").expect("actual USDA profile"),
        ),
    ] {
        assert_eq!(extensions.matches(&format!("\"{extension}\"")).count(), 1);
        assert_eq!(
            syntax::detect(&format!("sample.{}", extension.to_uppercase()), ""),
            Some(language)
        );
    }
    assert_ne!(
        syntax::detect("sample.usda", ""),
        syntax::detect("sample.cpp", "")
    );
    assert_ne!(
        syntax::detect("sample.usda", ""),
        syntax::detect("sample.toml", "")
    );
    for extension in ["usd", "usdc", "usdz", "html", "svg"] {
        assert!(!extensions.contains(&format!("\"{extension}\"")));
        assert_eq!(syntax::detect(&format!("sample.{extension}"), ""), None);
    }
    assert!(
        manifest
            .lines()
            .any(|l| l == "modes = [\"source\", \"text\"]")
    );
}

#[test]
fn windows_manifest_uses_xml_styles_through_public_source_mode() {
    let text = "<?xml version='1.0'?>\n<assembly xmlns='urn:owned:manifest'>\n<!-- inert -->\n<assemblyIdentity name='č😀'/>\n</assembly>\n";
    let mut host = Host::new(text.as_bytes(), entry);
    host.short_reads(7);
    open(&mut host, text.len(), "owned.MANIFEST", MODE_SOURCE);
    let page = page(&mut host, &viewport(0, false));
    assert_eq!(page.text, text);
    for (needle, role) in [
        ("assemblyIdentity", STYLE_KEYWORD),
        ("name", STYLE_TYPE),
        ("'č😀'", STYLE_STRING),
        ("<!-- inert -->", STYLE_COMMENT),
    ] {
        styled(&page.text, &page.spans, needle, role);
    }
    assert_eq!(page.anchors.first().unwrap().source_offset, 0);
    assert_eq!(
        page.anchors.last().unwrap().source_offset,
        text.len() as u64
    );
}

#[test]
fn usda_real_grammar_tokens_have_native_public_source_spans() {
    let text = "#usda 1.0\n(\n subLayers = [@owned/base.usda@]\n)\ndef Xform \"Owned\" {\n uniform token purpose = \"render\"\n point3f[] points = [(1.25, -.5, 2e+3)]\n rel target = </Owned/Child.property>\n asset path = @@@owned/a@b.usda@@@\n double bad = -inf\n // line comment\n /* block\n comment */\n}\n";
    let mut host = Host::new(text.as_bytes(), entry);
    open(&mut host, text.len(), "owned.usda", MODE_SOURCE);
    let page = page(&mut host, &viewport(0, false));
    assert_eq!(page.text, text);
    for (needle, role) in [
        ("#usda 1.0", STYLE_COMMENT),
        ("subLayers", STYLE_KEYWORD),
        ("def", STYLE_KEYWORD),
        ("Xform", STYLE_TYPE),
        ("uniform", STYLE_KEYWORD),
        ("token", STYLE_TYPE),
        ("point3f", STYLE_TYPE),
        ("\"render\"", STYLE_STRING),
        ("1.25", STYLE_NUMBER),
        ("-.5", STYLE_NUMBER),
        ("2e+3", STYLE_NUMBER),
        ("-inf", STYLE_NUMBER),
        ("</Owned/Child.property>", STYLE_STRING),
        ("@owned/base.usda@", STYLE_STRING),
        ("@@@owned/a@b.usda@@@", STYLE_STRING),
        ("// line comment", STYLE_COMMENT),
        ("/* block\n comment */", STYLE_COMMENT),
        ("=", STYLE_OPERATOR),
    ] {
        styled(&page.text, &page.spans, needle, role);
    }
    assert_eq!(
        page.anchors.last().unwrap().source_offset,
        text.len() as u64
    );
}

#[test]
fn usda_literals_namespaces_and_malformed_line_literals_keep_correct_context() {
    let text = "def Scope \"Owned\" {\n string doc = '''first\n# literal @asset@ </path>\nlast'''\n asset a = @@@a\\@@@b@@@\n asset b = @@@two@@at@@@@@\n token c = 'unterminated\n def Xform \"Restored\" {}\n asset d = @@@unterminated\n over \"RestoredAgain\" {}\n floatSuffix primvars:float names::def nanValue\n}\n";
    let language = syntax::detect("owned.usda", "").expect("USDA profile");
    let lexed = syntax::highlight(text, language, &mut syntax::State::default(), || false).unwrap();
    for (needle, role) in [
        ("'''first\n# literal @asset@ </path>\nlast'''", STYLE_STRING),
        ("@@@a\\@@@b@@@", STYLE_STRING),
        ("@@@two@@at@@@@@", STYLE_STRING),
        ("Xform", STYLE_TYPE),
        ("over", STYLE_KEYWORD),
    ] {
        styled(text, &lexed.spans, needle, role);
    }
    for needle in ["floatSuffix", "primvars:float", "names::def", "nanValue"] {
        plain(text, &lexed.spans, needle);
    }
    let next = "def Scope \"Next\" {}";
    let mut state = syntax::State::default();
    syntax::highlight("/* carried\n", language, &mut state, || false).unwrap();
    let result = syntax::highlight("end */ def Scope \"Next\" {}", language, &mut state, || {
        false
    })
    .unwrap();
    styled(
        "end */ def Scope \"Next\" {}",
        &result.spans,
        "end */",
        STYLE_COMMENT,
    );
    let fresh = syntax::highlight(next, language, &mut state, || false).unwrap();
    styled(next, &fresh.spans, "def", STYLE_KEYWORD);
}

#[test]
fn manifest_and_usda_utf16_short_reads_preserve_styles_and_raw_anchors() {
    for (name, text, needle, role) in [
        (
            "owned.manifest",
            "<assembly name='č😀'/>\r\n",
            "assembly",
            STYLE_KEYWORD,
        ),
        (
            "owned.usda",
            "#usda 1.0\r\ndef Xform \"č😀\" {}\r\n",
            "Xform",
            STYLE_TYPE,
        ),
    ] {
        for big_endian in [false, true] {
            let mut bytes = if big_endian {
                vec![0xfe, 0xff]
            } else {
                vec![0xff, 0xfe]
            };
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&if big_endian {
                    unit.to_be_bytes()
                } else {
                    unit.to_le_bytes()
                });
            }
            let mut host = Host::new(&bytes, entry);
            host.short_reads(1);
            open(&mut host, bytes.len(), name, MODE_SOURCE);
            let page = page(&mut host, &viewport(0, false));
            assert_eq!(page.text, text.replace("\r\n", "\n"));
            styled(&page.text, &page.spans, needle, role);
            assert_eq!(page.source.offset, 2);
            assert_eq!(page.anchors.first().unwrap().source_offset, 2);
            assert_eq!(
                page.anchors.last().unwrap().source_offset,
                bytes.len() as u64
            );
            let emoji_text = page.text.find('😀').unwrap() as u32;
            let emoji_raw = 2 + text[..text.find('😀').unwrap()].encode_utf16().count() as u64 * 2;
            assert!(
                page.anchors
                    .iter()
                    .any(|a| a.text_offset == emoji_text && a.source_offset == emoji_raw)
            );
        }
    }
}

#[test]
fn explicit_text_mode_keeps_new_extensions_plain_and_malformed_bytes_readable() {
    for name in ["owned.manifest", "owned.usda"] {
        let input = b"def Xform \"owned\" {}\r\n<assembly/>\xff\n";
        let mut host = Host::new(input, entry);
        host.short_reads(3);
        open(&mut host, input.len(), name, MODE_TEXT);
        let page = page(&mut host, &viewport(0, false));
        assert_eq!(page.text, "def Xform \"owned\" {}\n<assembly/>�\n");
        assert!(page.spans.is_empty());
        assert_eq!(
            page.anchors.last().unwrap().source_offset,
            input.len() as u64
        );
    }
    let input = b"asset x = @broken\xff\ndef Xform \"recovered\" {}\n";
    let mut host = Host::new(input, entry);
    open(&mut host, input.len(), "malformed.usda", MODE_SOURCE);
    let page = page(&mut host, &viewport(0, false));
    assert!(page.text.contains('�'));
    styled(&page.text, &page.spans, "def", STYLE_KEYWORD);
    assert_eq!(
        page.anchors.last().unwrap().source_offset,
        input.len() as u64
    );
}

#[test]
fn usda_dense_directional_pages_keep_styles_inside_existing_wire_budget() {
    let input = format!("{}\n", "float x=1;".repeat(2000));
    for backwards in [false, true] {
        let mut host = Host::new(input.as_bytes(), entry);
        open(&mut host, input.len(), "dense.usda", MODE_SOURCE);
        let mut target = viewport(if backwards { input.len() as u64 } else { 0 }, backwards);
        target.max_lines = 512;
        let before = host.reads().len();
        let page = page(&mut host, &target);
        assert!(!page.text.is_empty());
        assert!(
            page.text.len() < 8192,
            "dense aggregate wire items must crop the page"
        );
        assert!(
            codec::encode(&page).unwrap().len()
                <= midnight_plugin_sdk::api::wire::MAX_MESSAGE_BYTES
        );
        assert!(!page.spans.is_empty());
        assert!(page.spans.iter().any(|s| s.role == STYLE_TYPE));
        let start = page.source.offset as usize;
        let end = start + page.source.length as usize;
        assert_eq!(page.text, input[start..end]);
        if backwards {
            assert_eq!(end, input.len());
        } else {
            assert_eq!(start, 0);
        }
        assert!(
            host.reads()[before..]
                .iter()
                .map(|r| u64::from(r.length))
                .sum::<u64>()
                <= 65536
        );
    }
}

#[test]
fn usda_distant_multiline_string_recovers_only_after_bounded_preparation() {
    let input = format!(
        "#usda 1.0\nstring doc = \"\"\"\n{}inside\"\"\"\ndef Xform \"Done\" {{}}\n",
        "owned text\n".repeat(12000)
    );
    let target = viewport(input.find("inside").unwrap() as u64, false);
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "distant.usda", MODE_SOURCE);
    let cold = page(&mut host, &target);
    assert!(
        cold.spans.is_empty(),
        "cold foreground must retain its bounded context limit"
    );
    let mut frontier = 0;
    for quantum in 0..32 {
        let before = host.reads().len();
        let id = host.submit(
            abi::REQUEST_PREPARE_SOURCE,
            &PrepareSource {
                revision: 3,
                viewport: target.clone(),
                max_scan_bytes: 8192,
            },
        );
        let events = host.finish();
        let events: Vec<_> = events.iter().filter(|e| e.id == id).collect();
        assert_eq!(
            events.iter().map(|e| e.kind).collect::<Vec<_>>(),
            [abi::EVENT_SOURCE_PREPARED, abi::EVENT_COMPLETED]
        );
        let prepared: SourcePrepared = codec::decode(&events[0].payload).unwrap();
        assert_eq!(prepared.revision, 3);
        assert!(prepared.frontier >= frontier);
        frontier = prepared.frontier;
        let reads = host.reads();
        let reads = &reads[before..];
        assert!(reads.len() <= 64);
        assert!(reads.iter().all(|r| r.length <= 16384));
        assert!(reads.iter().map(|r| u64::from(r.length)).sum::<u64>() <= 8192);
        if prepared.ready {
            assert!(quantum > 0);
            let page = page(&mut host, &target);
            styled(&page.text, &page.spans, "inside\"\"\"", STYLE_STRING);
            styled(&page.text, &page.spans, "def", STYLE_KEYWORD);
            styled(&page.text, &page.spans, "Xform", STYLE_TYPE);
            return;
        }
    }
    panic!("owned USDA preparation did not finish in its bounded quanta");
}
