use super::{Html, catalog, sheet};
use crate::{
    DocumentDescriptor, DocumentId, OfficeCatalog, OfficeError, OfficeFormat, OfficeTarget,
    SheetDescriptor, SheetId, SlideDescriptor, SlideId, document,
    sheet::{Cell, Window},
};
use std::collections::HashSet;

fn empty_catalog(format: OfficeFormat) -> OfficeCatalog {
    OfficeCatalog {
        format,
        sheets: Vec::new(),
        document: None,
        slides: Vec::new(),
    }
}

fn sheets(format: OfficeFormat, count: u16, name: &str) -> OfficeCatalog {
    let mut value = empty_catalog(format);
    value.sheets = (0..count)
        .map(|id| SheetDescriptor {
            id: SheetId(id),
            name: name.to_owned(),
        })
        .collect();
    value
}

fn links(html: &str) -> Vec<(&str, &str)> {
    html.split("<a href=\"")
        .skip(1)
        .map(|anchor| {
            let (path, rest) = anchor.split_once("\">").unwrap();
            let (label, _) = rest.split_once("</a>").unwrap();
            (path, label)
        })
        .collect()
}

#[test]
fn max_sheet_catalog_retains_all_ids_with_escaped_name_previews() {
    for format in [OfficeFormat::Xlsx, OfficeFormat::Ods] {
        let value = sheets(format, 1024, &"\"".repeat(1024));
        let bytes = catalog(&value).unwrap();
        assert!(bytes.len() <= 1024 * 1024);
        let html = std::str::from_utf8(&bytes).unwrap();
        assert!(html.contains("Long names are shortened"));
        assert!(!html.contains("&quot;".repeat(1024).as_str()));
        let anchors = links(html);
        assert_eq!(anchors.len(), 1024);
        let paths: HashSet<_> = anchors.iter().map(|(path, _)| *path).collect();
        assert_eq!(paths.len(), 1024);
        for (index, (path, label)) in anchors.iter().enumerate() {
            let target = OfficeTarget::SheetFirst {
                sheet: value.sheets[index].id,
            };
            assert_eq!(*path, target.path());
            assert_eq!(OfficeTarget::parse(path).unwrap(), target);
            assert!(label.starts_with(&format!("{}: ", index + 1)));
            assert!(label.ends_with('…'));
        }
    }
}

#[test]
fn max_slide_catalog_retains_all_ids_under_html_limit() {
    for format in [OfficeFormat::Pptx, OfficeFormat::Odp] {
        for maximum_names in [false, true] {
            let mut value = empty_catalog(format);
            value.slides = (0..4096)
                .map(|id| SlideDescriptor {
                    id: SlideId(id),
                    name: if maximum_names {
                        "\"".repeat(1024)
                    } else {
                        format!("Slide {}", id + 1)
                    },
                })
                .collect();
            let bytes = catalog(&value).unwrap();
            assert!(bytes.len() <= 1024 * 1024);
            let html = std::str::from_utf8(&bytes).unwrap();
            let anchors = links(html);
            assert_eq!(anchors.len(), 4096);
            let paths: HashSet<_> = anchors.iter().map(|(path, _)| *path).collect();
            assert_eq!(paths.len(), 4096);
            for (index, (path, label)) in anchors.iter().enumerate() {
                let target = OfficeTarget::SlideWindow {
                    slide: value.slides[index].id,
                    block: 0,
                };
                assert_eq!(OfficeTarget::parse(path).unwrap(), target);
                assert!(label.starts_with(&format!("{}: ", index + 1)));
                if maximum_names {
                    assert!(label.ends_with('…'));
                } else {
                    assert_eq!(*label, format!("{}: Slide {}", index + 1, index + 1));
                }
            }
        }
    }
}

#[test]
fn max_empty_sheet_keeps_full_heading_and_bounded_fixed_navigation() {
    for selected in [0, 1022, 1023] {
        let mut value = sheets(OfficeFormat::Xlsx, 1024, &"\"".repeat(1024));
        let window = Window {
            row: 1_000_000,
            column: 16_000,
            cells: Vec::new(),
            next: Some((1_048_576, 16_384)),
            omitted: false,
        };
        let bytes = sheet(&value, SheetId(selected), &window).unwrap();
        assert!(bytes.len() < 16 * 1024, "navigation exhausted its frame");
        let html = std::str::from_utf8(&bytes).unwrap();
        assert!(html.contains(&format!("<h1>{}</h1>", "&quot;".repeat(1024))));
        assert!(html.contains("Empty cell range."));
        let anchors = links(html);
        assert!(anchors.len() <= 9);
        let targets: Vec<_> = anchors
            .iter()
            .map(|(path, _)| OfficeTarget::parse(path).unwrap())
            .collect();
        assert!(targets.contains(&OfficeTarget::Catalog));
        assert!(targets.contains(&OfficeTarget::SheetFirst {
            sheet: SheetId(selected)
        }));
        for (row, column) in [
            (999_872, 16_000),
            (1_000_128, 16_000),
            (1_000_000, 15_968),
            (1_000_000, 16_032),
            (1_048_576, 16_384),
        ] {
            assert!(targets.contains(&OfficeTarget::SheetWindow {
                sheet: SheetId(selected),
                row,
                column,
            }));
        }
        for label in [
            "All sheets",
            "First cells",
            "Previous rows",
            "Next rows",
            "Previous columns",
            "Next columns",
            "Next populated region",
        ] {
            assert!(anchors.iter().any(|(_, actual)| *actual == label));
        }
        assert_eq!(html.contains("Previous sheet</a>"), selected > 0);
        assert_eq!(html.contains("Next sheet</a>"), selected < 1023);
        if selected > 0 {
            assert!(targets.contains(&OfficeTarget::SheetFirst {
                sheet: SheetId(selected - 1)
            }));
        }
        if selected < 1023 {
            assert!(targets.contains(&OfficeTarget::SheetFirst {
                sheet: SheetId(selected + 1)
            }));
        }
        for descriptor in &mut value.sheets {
            if descriptor.id != SheetId(selected) {
                descriptor.name = "<unselected name must not enter this page>".to_owned();
            }
        }
        assert_eq!(sheet(&value, SheetId(selected), &window).unwrap(), bytes);
    }
}

#[test]
fn catalog_preview_preserves_utf8_and_complete_escape_entities() {
    let name = "&<\"'é界😀suffix";
    for (budget, expected) in [
        (0, "…"),
        (4, "…"),
        (5, "&amp;…"),
        (8, "&amp;…"),
        (9, "&amp;&lt;…"),
        (14, "&amp;&lt;…"),
        (15, "&amp;&lt;&quot;…"),
        (20, "&amp;&lt;&quot;&#39;…"),
        (21, "&amp;&lt;&quot;&#39;…"),
        (22, "&amp;&lt;&quot;&#39;é…"),
        (24, "&amp;&lt;&quot;&#39;é…"),
        (25, "&amp;&lt;&quot;&#39;é界…"),
        (28, "&amp;&lt;&quot;&#39;é界…"),
        (29, "&amp;&lt;&quot;&#39;é界😀…"),
        (35, "&amp;&lt;&quot;&#39;é界😀suffix"),
    ] {
        let mut html = Html::new();
        html.raw("<div>").unwrap();
        html.preview(name, budget).unwrap();
        html.raw("</div>").unwrap();
        let bytes = html.finish().unwrap();
        let html = std::str::from_utf8(&bytes).unwrap();
        assert!(html.contains(&format!("<div>{expected}</div>")), "{budget}");
    }
    let mixed = "<script>\"é界😀&'".repeat(45);
    assert!(mixed.len() <= 1024);
    let value = sheets(OfficeFormat::Xlsx, 1024, &mixed);
    let bytes = catalog(&value).unwrap();
    assert!(bytes.len() <= 1024 * 1024);
    let html = std::str::from_utf8(&bytes).unwrap();
    assert!(!html.contains("<script>"));
    assert!(!html.contains("title="));
    assert!(links(html).iter().all(|(_, label)| label.ends_with('…')));
}

#[test]
fn short_catalog_names_remain_complete_and_selected_name_is_unshortened() {
    let name = "é界😀 & <tag> \"'";
    let escaped = "é界😀 &amp; &lt;tag&gt; &quot;&#39;";
    let value = sheets(OfficeFormat::Ods, 1, name);
    let bytes = catalog(&value).unwrap();
    let html = std::str::from_utf8(&bytes).unwrap();
    assert_eq!(links(html)[0].1, format!("1: {escaped}"));
    let bytes = sheet(
        &value,
        SheetId(0),
        &Window {
            row: 1,
            column: 1,
            cells: Vec::new(),
            next: None,
            omitted: false,
        },
    )
    .unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains(&format!("<h1>{escaped}</h1>"))
    );
}

#[test]
fn catalogs_only_expose_the_selected_format_family() {
    for format in [OfficeFormat::Docx, OfficeFormat::Odt] {
        let mut value = sheets(format, 1, "foreign sheet");
        value.document = Some(DocumentDescriptor {
            id: DocumentId(0),
            name: "Actual document".to_owned(),
        });
        value.slides.push(SlideDescriptor {
            id: SlideId(0),
            name: "foreign slide".to_owned(),
        });
        let bytes = catalog(&value).unwrap();
        let html = std::str::from_utf8(&bytes).unwrap();
        let anchors = links(html);
        assert_eq!(anchors.len(), 1);
        assert_eq!(anchors[0].1, "1: Actual document");
        assert_eq!(
            OfficeTarget::parse(anchors[0].0).unwrap(),
            OfficeTarget::DocumentWindow {
                document: DocumentId(0),
                block: 0
            }
        );
        assert!(!html.contains("foreign"));
    }
    for format in [
        OfficeFormat::Xlsx,
        OfficeFormat::Ods,
        OfficeFormat::Docx,
        OfficeFormat::Odt,
        OfficeFormat::Pptx,
        OfficeFormat::Odp,
    ] {
        let bytes = catalog(&empty_catalog(format)).unwrap();
        assert!(links(std::str::from_utf8(&bytes).unwrap()).is_empty());
    }
}

#[test]
fn sheet_body_escape_overflow_remains_typed_without_partial_html() {
    let value = sheets(OfficeFormat::Xlsx, 1024, &"\"".repeat(1024));
    let cells = (1..=128)
        .flat_map(|row| {
            (1..=32).map(move |column| Cell {
                row,
                column,
                text: "\"".repeat(128),
            })
        })
        .collect();
    let result = sheet(
        &value,
        SheetId(0),
        &Window {
            row: 1,
            column: 1,
            cells,
            next: None,
            omitted: false,
        },
    );
    assert!(matches!(
        result,
        Err(OfficeError::LimitExceeded("HTML resource"))
    ));
}

#[test]
fn document_slide_metadata_frame_fits_existing_body_reserve() {
    for target in [
        OfficeTarget::DocumentWindow {
            document: DocumentId(0),
            block: 999_999,
        },
        OfficeTarget::SlideWindow {
            slide: SlideId(4094),
            block: 999_999,
        },
    ] {
        let mut value = empty_catalog(match target {
            OfficeTarget::DocumentWindow { .. } => OfficeFormat::Docx,
            _ => OfficeFormat::Pptx,
        });
        value.document = Some(DocumentDescriptor {
            id: DocumentId(0),
            name: "\"".repeat(1024),
        });
        value.slides = (0..4096)
            .map(|id| SlideDescriptor {
                id: SlideId(id),
                name: "\"".repeat(1024),
            })
            .collect();
        let bytes = document::render(
            &value,
            target,
            &document::Window {
                start: 999_999,
                blocks: Vec::new(),
                next: Some(1_000_063),
                omitted: true,
            },
        )
        .unwrap();
        assert!(bytes.len() < 16 * 1024);
        let html = std::str::from_utf8(&bytes).unwrap();
        assert!(html.contains(&format!("<h1>{}</h1>", "&quot;".repeat(1024))));
        // Objects without text are named as such, never shown as an empty window.
        assert!(html.contains("contains only pictures, charts or other objects"));
        assert!(!html.contains("Empty content window."));
        assert!(links(html).len() <= 5);
    }
}

#[test]
fn footer_is_included_in_the_html_resource_limit() {
    let mut html = Html::new();
    let prefix = html.0.len();
    let footer = "</body></html>";
    html.raw(&"x".repeat(1024 * 1024 - prefix - footer.len()))
        .unwrap();
    assert_eq!(html.finish().unwrap().len(), 1024 * 1024);

    let mut html = Html::new();
    html.raw(&"x".repeat(1024 * 1024 - prefix - footer.len() + 1))
        .unwrap();
    assert!(matches!(
        html.finish(),
        Err(OfficeError::LimitExceeded("HTML resource"))
    ));
}
