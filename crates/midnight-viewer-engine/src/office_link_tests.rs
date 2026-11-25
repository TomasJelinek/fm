//! Office HTML transport URLs are relative to the captured document's private directory.
use super::*;

fn read_html(file: &CapturedFile) -> String {
    let mut bytes = Vec::new();
    while (bytes.len() as u64) < file.length() {
        let chunk = file.read_at(bytes.len() as u64, RESOURCE_READ).unwrap();
        assert!(!chunk.is_empty());
        bytes.extend_from_slice(&chunk);
    }
    assert_eq!(bytes.len() as u64, file.length());
    assert!(file.read_at(file.length(), 1).unwrap().is_empty());
    String::from_utf8(bytes).unwrap()
}

fn hrefs(html: &str) -> Vec<&str> {
    html.split("<a href=\"")
        .skip(1)
        .map(|anchor| anchor.split_once('"').unwrap().0)
        .collect()
}

// The generated subset uses only path references. Resolve their RFC dot segments
// independently of the adapter's link emission; no network or filesystem is accessed.
fn browser_url(base: &str, href: &str) -> String {
    let (scheme, rest) = base.split_once("://").unwrap();
    let (authority, path) = rest.split_once('/').unwrap();
    let path = if href.starts_with('/') {
        href.to_owned()
    } else {
        format!(
            "{}/{}",
            path.rsplit_once('/').map_or("", |(dir, _)| dir),
            href
        )
    };
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    format!("{scheme}://{authority}/{}", segments.join("/"))
}

#[test]
fn office_real_generated_links_resolve_from_bootstrap_catalog_and_nested_sheets() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("navigation.xlsx");
    std::fs::write(
        &path,
        include_bytes!("office_native_fixture/two-sheet-windows.xlsx"),
    )
    .unwrap();
    let prepared = prepare(
        super::viewer_capture(&path),
        OfficeFormat::Xlsx,
        Cancellation::default(),
    )
    .unwrap();
    let initial = read_html(&prepared.main);
    assert!(initial.contains("NATIVE_ALPHA_A1_V1"));
    assert!(hrefs(&initial).contains(&"office/v1/catalog.html"));
    for root in [
        "https://private.invalid/mewv-document/",
        "https://private.invalid/arbitrary/nested-root/",
    ] {
        let legacy = browser_url(root, "/office/v1/catalog.html");
        assert!(
            !legacy.starts_with(root),
            "old absolute link escapes the document directory"
        );
        assert_eq!(
            browser_url(root, "office/v1/catalog.html"),
            format!("{root}office/v1/catalog.html")
        );
        for (target, prefix, required) in [
            (
                OfficeTarget::Catalog,
                "../../",
                "office/v1/sheet/1/first.html",
            ),
            (
                OfficeTarget::SheetFirst { sheet: SheetId(1) },
                "../../../../",
                "office/v1/sheet/0/first.html",
            ),
            (
                OfficeTarget::SheetWindow {
                    sheet: SheetId(0),
                    row: 129,
                    column: 1,
                },
                "../../../../../../../",
                "office/v1/sheet/0/row/257/col/1.html",
            ),
        ] {
            let file = prepared
                .resources
                .capture(&segments(&target.path()))
                .unwrap();
            let html = read_html(&file);
            let links = hrefs(&html);
            assert!(links.contains(&format!("{prefix}{required}").as_str()));
            let base = format!("{root}{}", target.path().trim_start_matches('/'));
            for link in links {
                let resolved = browser_url(&base, link);
                assert!(resolved.starts_with(root), "{base} + {link} = {resolved}");
                let canonical = format!("/{}", resolved.strip_prefix(root).unwrap());
                let granted = prepared.resources.capture(&segments(&canonical)).unwrap();
                granted.release_on_worker().unwrap();
            }
            file.release_on_worker().unwrap();
        }
    }
    release(prepared);
}

struct LinkGeneration {
    bytes: Arc<Mutex<Arc<[u8]>>>,
    sequence: u64,
}

impl ExternalGeneration for LinkGeneration {
    fn initial_target(&self) -> OfficeTarget {
        OfficeTarget::SlideWindow {
            slide: SlideId(0),
            block: 0,
        }
    }
    fn parse_target(&self, path: &str) -> Result<OfficeTarget, String> {
        [
            self.initial_target(),
            OfficeTarget::Catalog,
            OfficeTarget::SlideWindow {
                slide: SlideId(1),
                block: 0,
            },
        ]
        .into_iter()
        .find(|target| target.path() == path)
        .ok_or_else(|| "unknown controlled link target".into())
    }
    fn render(
        &mut self,
        target: OfficeTarget,
        _: &Cancellation,
    ) -> Result<ImmutableHtml, PreviewError> {
        self.sequence += 1;
        Ok(ImmutableHtml {
            bytes: self.bytes.lock().unwrap().clone(),
            target,
            generation: self.sequence,
        })
    }
}

#[test]
fn office_slide_transport_keeps_escaped_text_and_canonical_resource_identity() {
    let body = r#"<p>Žluťoučký &lt;a href=&quot;/office/v1/catalog.html&quot;&gt;</p><a href="/office/v1/slide/1/block/0.html">Next slide</a>"#;
    let bytes = Arc::new(Mutex::new(Arc::from(body.as_bytes())));
    let fixture = Fixture::new();
    let prepared = prepare_external(
        fixture.capture(),
        Cancellation::default(),
        Box::new(LinkGeneration { bytes, sequence: 0 }),
    )
    .unwrap();
    let initial = read_html(&prepared.main);
    assert_eq!(
        initial,
        r#"<p>Žluťoučký &lt;a href=&quot;/office/v1/catalog.html&quot;&gt;</p><a href="office/v1/slide/1/block/0.html">Next slide</a>"#
    );
    let target = OfficeTarget::SlideWindow {
        slide: SlideId(1),
        block: 0,
    };
    let file = prepared
        .resources
        .capture(&segments(&target.path()))
        .unwrap();
    let html = read_html(&file);
    assert_eq!(
        hrefs(&html),
        ["../../../../../office/v1/slide/1/block/0.html"]
    );
    assert!(html.contains("&lt;a href=&quot;/office/v1/catalog.html&quot;&gt;"));
    assert!(
        file.read_at(0, RESOURCE_READ)
            .unwrap()
            .starts_with("<p>Žluťoučký".as_bytes())
    );
    assert!(
        prepared
            .resources
            .capture(&segments("/office/v1/slide/01/block/0.html"))
            .is_err()
    );
    file.release_on_worker().unwrap();
    release(prepared);
}

#[test]
fn office_adapted_link_growth_is_bounded_and_failed_render_returns_its_credit() {
    let mut body = b"<a href=\"/office/v1/catalog.html\">All slides</a>".to_vec();
    body.resize(MIB, b' ');
    let bytes = Arc::new(Mutex::new(Arc::from(body)));
    let fixture = Fixture::new();
    let lifetime = Cancellation::default();
    let prepared = prepare_external(
        fixture.capture(),
        lifetime.clone(),
        Box::new(LinkGeneration {
            bytes: bytes.clone(),
            sequence: 0,
        }),
    )
    .unwrap();
    assert_eq!(prepared.main.length(), (MIB - 1) as u64);
    let target = OfficeTarget::SlideWindow {
        slide: SlideId(1),
        block: 0,
    };
    assert!(
        prepared
            .resources
            .capture(&segments(&target.path()))
            .is_err()
    );
    *bytes.lock().unwrap() =
        Arc::from(b"<a href=\"/office/v1/catalog.html\">All slides</a>".as_slice());
    let retained = (0..3)
        .map(|_| {
            prepared
                .resources
                .capture(&segments(&target.path()))
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        prepared
            .resources
            .capture(&segments(&target.path()))
            .is_err()
    );
    for file in retained {
        assert!(read_html(&file).len() < MIB);
        file.release_on_worker().unwrap();
    }
    let recovered = prepared
        .resources
        .capture(&segments(&target.path()))
        .unwrap();
    recovered.release_on_worker().unwrap();
    lifetime.cancel();
    assert!(
        prepared
            .resources
            .capture(&segments(&target.path()))
            .is_err()
    );
    release(prepared);
}
