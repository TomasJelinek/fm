use super::*;
use midnight_plugin_host::api::viewer;

#[test]
fn optional_html_provider_uses_associations_and_basic_stays_inert() {
    let native = vec![
        native("basic", &[], &[viewer::MODE_TEXT], 0),
        native("source", &["html"], &[viewer::MODE_SOURCE], 0),
    ];
    let web = [("html", vec!["html".into(), "htm".into(), "xhtml".into()])];
    assert_eq!(
        route(
            &native,
            &web,
            "页面.HTML",
            false,
            &[],
            &ViewerAssociations::default()
        ),
        Some(Route::Process { index: 0 })
    );
    assert_eq!(
        route(
            &native,
            &web,
            "页面.html",
            true,
            &[],
            &ViewerAssociations::default()
        ),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_TEXT,
            detect_binary: false
        })
    );
    assert_eq!(
        route(
            &native,
            &web,
            "页面.html",
            false,
            &["html".into()],
            &ViewerAssociations::default()
        ),
        Some(Route::Native {
            index: 1,
            mode: viewer::MODE_SOURCE,
            detect_binary: false
        })
    );
    let associations = ViewerAssociations {
        overrides: [("html".into(), vec!["page".into()])].into(),
        order: vec![],
    };
    assert_eq!(
        route(&native, &web, "document.page", false, &[], &associations),
        Some(Route::Process { index: 0 })
    );
    assert_eq!(
        route(&native, &web, "document.html", false, &[], &associations),
        Some(Route::Native {
            index: 1,
            mode: viewer::MODE_SOURCE,
            detect_binary: false
        })
    );
}

fn native(id: &str, extensions: &[&str], modes: &[u16], priority: i32) -> PluginInfo {
    PluginInfo {
        id: id.into(),
        name: None,
        version: "1".into(),
        description: String::new(),
        models: vec![],
        modes: modes.to_vec(),
        extensions: extensions.iter().map(|value| (*value).into()).collect(),
        priority,
        package_dir: "unused".into(),
        entry_library: "unused".into(),
    }
}

fn route(
    native: &[PluginInfo],
    process: &[(&str, Vec<String>)],
    name: &str,
    basic: bool,
    disabled: &[String],
    associations: &ViewerAssociations,
) -> Option<Route> {
    choose(
        native,
        process
            .iter()
            .map(|(id, extensions)| ProcessCandidate { id, extensions }),
        Path::new(name),
        basic,
        disabled,
        associations,
        None,
    )
}

#[test]
fn video_after_image_and_web_preserves_process_indices_and_user_routing_policy() {
    let video = midnight_media_host::video_installation::VIDEO_PROVIDER_ID;
    let native = vec![
        native("basic", &[], &[viewer::MODE_TEXT, viewer::MODE_BINARY], 0),
        native("source", &["mp4"], &[viewer::MODE_SOURCE], 0),
    ];
    let process = [
        ("midnight.viewer.image", vec!["png".into()]),
        (HTML_ID, vec!["html".into()]),
        (video, vec!["mp4".into(), "webm".into()]),
    ];
    let mut associations = ViewerAssociations::default();
    for (name, index) in [
        ("image.PNG", 0),
        ("page.html", 1),
        ("movie.MP4", 2),
        ("movie.webm", 2),
    ] {
        assert_eq!(
            route(&native, &process, name, false, &[], &associations),
            Some(Route::Process { index })
        );
    }
    let source = Some(Route::Native {
        index: 1,
        mode: viewer::MODE_SOURCE,
        detect_binary: false,
    });
    assert_eq!(
        route(
            &native,
            &process,
            "movie.mp4",
            false,
            &[video.into()],
            &associations
        ),
        source
    );
    associations.order = vec!["source".into(), video.into()];
    assert_eq!(
        route(&native, &process, "movie.mp4", false, &[], &associations),
        source
    );
    associations.order.reverse();
    assert_eq!(
        route(&native, &process, "movie.mp4", false, &[], &associations),
        Some(Route::Process { index: 2 })
    );

    associations
        .overrides
        .insert(video.into(), vec!["movie".into()]);
    assert_eq!(
        route(&native, &process, "custom.MOVIE", false, &[], &associations),
        Some(Route::Process { index: 2 })
    );
    assert_eq!(
        route(&native, &process, "movie.mp4", false, &[], &associations),
        source,
        "an override replaces rather than extends the installed video suffixes"
    );
    for name in ["movie.mp4", "custom.movie"] {
        assert_eq!(
            route(&native, &process, name, true, &[], &associations),
            Some(Route::Native {
                index: 0,
                mode: viewer::MODE_TEXT,
                detect_binary: false
            }),
            "Basic never selects a decoder or probes binary content"
        );
    }
    associations.overrides.insert(video.into(), vec![]);
    assert_eq!(
        route(&native, &process, "movie.mp4", false, &[], &associations),
        source
    );
    assert_eq!(
        route(&native, &process, "custom.movie", false, &[], &associations),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_TEXT,
            detect_binary: true
        })
    );
    associations
        .overrides
        .insert(video.into(), vec!["movie".into()]);
    assert_eq!(
        route(
            &native,
            &process[..2],
            "custom.movie",
            false,
            &[],
            &associations
        ),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_TEXT,
            detect_binary: true
        }),
        "retained preferences cannot manufacture an unavailable video provider"
    );
}

#[test]
fn routing_user_order_overrides_the_native_process_split_but_never_prefers_fallback() {
    let native = vec![
        native("basic", &[], &[viewer::MODE_TEXT, viewer::MODE_BINARY], 999),
        native("source", &["png"], &[viewer::MODE_SOURCE], 999),
    ];
    let process = [("image", vec!["png".into()])];
    let mut associations = ViewerAssociations::default();
    assert_eq!(
        route(&native, &process, "photo.png", false, &[], &associations),
        Some(Route::Process { index: 0 })
    );
    associations.order = vec!["basic".into(), "source".into()];
    assert_eq!(
        route(&native, &process, "photo.PNG", false, &[], &associations),
        Some(Route::Native {
            index: 1,
            mode: viewer::MODE_SOURCE,
            detect_binary: false
        })
    );
    assert_eq!(
        route(
            &native,
            &process,
            "photo.png",
            false,
            &["source".into()],
            &associations
        ),
        Some(Route::Process { index: 0 })
    );
}

#[test]
fn routing_overrides_replace_process_suffixes_and_longest_suffix_wins() {
    let native = vec![native("source", &["gz"], &[viewer::MODE_SOURCE], 0)];
    let process = [("pdf", vec!["pdf".into()])];
    let associations = ViewerAssociations {
        overrides: [("pdf".into(), vec!["tar.gz".into(), "paper".into()])].into(),
        order: vec!["source".into()],
    };
    assert_eq!(
        route(
            &native,
            &process,
            "bundle.TAR.GZ",
            false,
            &[],
            &associations
        ),
        Some(Route::Process { index: 0 })
    );
    assert_eq!(
        route(
            &native,
            &process,
            "document.paper",
            false,
            &[],
            &associations
        ),
        Some(Route::Process { index: 0 })
    );
    assert_eq!(
        route(&native, &process, "document.pdf", false, &[], &associations),
        None
    );
    assert_eq!(
        route(
            &native,
            &process,
            "folder.paper/document",
            false,
            &[],
            &associations
        ),
        None
    );
    assert_eq!(
        route(
            &native,
            &process,
            "documentpaper",
            false,
            &[],
            &associations
        ),
        None
    );
}

#[test]
fn routing_empty_overrides_preserve_only_the_original_basic_fallback_role() {
    let native = vec![
        native("basic", &[], &[viewer::MODE_TEXT, viewer::MODE_BINARY], 0),
        native(
            "source",
            &["rs"],
            &[viewer::MODE_TEXT, viewer::MODE_SOURCE],
            999,
        ),
    ];
    let associations = ViewerAssociations {
        overrides: [("basic".into(), vec![]), ("source".into(), vec![])].into(),
        order: vec!["source".into()],
    };
    assert_eq!(
        route(&native, &[], "code.rs", false, &[], &associations),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_TEXT,
            detect_binary: true
        })
    );
    assert_eq!(
        route(
            &native,
            &[],
            "code.rs",
            false,
            &["basic".into()],
            &associations
        ),
        None
    );
}

#[test]
fn routing_basic_and_generated_text_ignore_user_mappings_and_never_probe_binary() {
    let native = vec![
        native("basic", &[], &[viewer::MODE_TEXT, viewer::MODE_BINARY], 0),
        native("markdown", &["md"], &[viewer::MODE_DOCUMENT], 999),
    ];
    let process = [("image", vec!["png".into()])];
    let associations = ViewerAssociations {
        overrides: [
            ("basic".into(), vec!["foo".into()]),
            ("image".into(), vec!["md".into(), "txt".into()]),
        ]
        .into(),
        order: vec!["image".into(), "markdown".into()],
    };
    for name in ["file.md", "generated.txt", "photo.png"] {
        assert_eq!(
            route(&native, &process, name, true, &[], &associations),
            Some(Route::Native {
                index: 0,
                mode: viewer::MODE_TEXT,
                detect_binary: false
            })
        );
        assert_eq!(
            route(
                &native,
                &process,
                name,
                true,
                &["basic".into()],
                &associations
            ),
            None
        );
    }
}

#[test]
fn routing_source_binary_plugin_keeps_selected_source_mode_and_native_defaults() {
    let native = vec![
        native(
            "source",
            &["md"],
            &[viewer::MODE_SOURCE, viewer::MODE_BINARY],
            999,
        ),
        native("markdown", &["md"], &[viewer::MODE_DOCUMENT], -999),
    ];
    let mut associations = ViewerAssociations::default();
    assert_eq!(
        route(&native, &[], "file.md", false, &[], &associations),
        Some(Route::Native {
            index: 1,
            mode: viewer::MODE_DOCUMENT,
            detect_binary: false
        })
    );
    associations.order = vec!["source".into()];
    assert_eq!(
        route(&native, &[], "file.md", false, &[], &associations),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_SOURCE,
            detect_binary: false
        })
    );
}

const OFFICE_ID: &str = "midnight.viewer.office-preview";
const HTML_ID: &str = "midnight.viewer.html";

fn office_route(
    native: &[PluginInfo],
    process: &[(&str, Vec<String>)],
    name: &str,
    basic: bool,
    disabled: &[String],
    associations: &ViewerAssociations,
    office_renderer: Option<usize>,
) -> Option<Route> {
    choose(
        native,
        process
            .iter()
            .map(|(id, extensions)| ProcessCandidate { id, extensions }),
        Path::new(name),
        basic,
        disabled,
        associations,
        office_renderer,
    )
}

#[test]
fn routing_office_builtin_requires_actual_enabled_html_renderer_and_builtin() {
    let native = [native(
        "basic",
        &[],
        &[viewer::MODE_TEXT, viewer::MODE_BINARY],
        0,
    )];
    let process = [(HTML_ID, vec!["html".into()])];
    let preferences = ViewerAssociations::default();
    assert_eq!(
        office_route(
            &native,
            &process,
            "čísla.XLSX",
            false,
            &[],
            &preferences,
            Some(0)
        ),
        Some(Route::Office { renderer: 0 }),
    );
    for disabled in [vec![HTML_ID.into()], vec![OFFICE_ID.into()]] {
        assert_eq!(
            office_route(
                &native,
                &process,
                "čísla.xlsx",
                false,
                &disabled,
                &preferences,
                Some(0)
            ),
            Some(Route::Native {
                index: 0,
                mode: viewer::MODE_TEXT,
                detect_binary: true
            }),
        );
    }
    for (candidates, renderer) in [
        (&process[..], None),
        (&process[..], Some(5)),
        (&[][..], Some(0)),
    ] {
        assert_eq!(
            office_route(
                &native,
                candidates,
                "čísla.xlsx",
                false,
                &[],
                &preferences,
                renderer
            ),
            Some(Route::Native {
                index: 0,
                mode: viewer::MODE_TEXT,
                detect_binary: true
            }),
        );
    }
}

#[test]
fn routing_office_builtin_defaults_cover_only_six_xml_container_extensions() {
    let process = [(HTML_ID, vec!["html".into()])];
    let preferences = ViewerAssociations::default();
    for extension in ["docx", "xlsx", "pptx", "odt", "ods", "odp"] {
        assert_eq!(
            office_route(
                &[],
                &process,
                &format!("report.{extension}"),
                false,
                &[],
                &preferences,
                Some(0)
            ),
            Some(Route::Office { renderer: 0 }),
            "{extension}",
        );
    }
    // These routing matches do not claim parser support for unimplemented formats.
    for extension in [
        "doc", "xls", "ppt", "xlsb", "docm", "xlsm", "pptm", "zip", "xlsx.exe", "xslx",
    ] {
        assert_eq!(
            office_route(
                &[],
                &process,
                &format!("report.{extension}"),
                false,
                &[],
                &preferences,
                Some(0)
            ),
            None,
            "{extension}",
        );
    }
}

#[test]
fn routing_office_basic_and_explicit_disabled_choices_remain_authoritative() {
    let native = [native(
        "basic",
        &[],
        &[viewer::MODE_TEXT, viewer::MODE_BINARY],
        0,
    )];
    let process = [(HTML_ID, vec!["html".into()])];
    let preferences = ViewerAssociations {
        overrides: Default::default(),
        order: vec![OFFICE_ID.into()],
    };
    assert_eq!(
        office_route(
            &native,
            &process,
            "report.xlsx",
            true,
            &[],
            &preferences,
            Some(0)
        ),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_TEXT,
            detect_binary: false
        }),
    );
    assert_eq!(
        office_route(
            &native,
            &process,
            "report.xlsx",
            true,
            &["basic".into()],
            &preferences,
            Some(0)
        ),
        None,
    );
    assert_eq!(
        office_route(
            &[],
            &process,
            "report.xlsx",
            false,
            &[OFFICE_ID.into()],
            &preferences,
            Some(0)
        ),
        None,
    );
}

#[test]
fn routing_office_builtin_participates_in_suffix_user_order_and_empty_overrides() {
    let native = [native(
        "source",
        &["xlsx", "report.xlsx"],
        &[viewer::MODE_SOURCE],
        999,
    )];
    let process = [(HTML_ID, vec!["html".into()])];
    let mut preferences = ViewerAssociations::default();
    assert_eq!(
        office_route(
            &native,
            &process,
            "annual.xlsx",
            false,
            &[],
            &preferences,
            Some(0)
        ),
        Some(Route::Office { renderer: 0 }),
    );
    preferences.order = vec!["source".into(), OFFICE_ID.into()];
    assert_eq!(
        office_route(
            &native,
            &process,
            "annual.xlsx",
            false,
            &[],
            &preferences,
            Some(0)
        ),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_SOURCE,
            detect_binary: false
        }),
    );
    preferences.order = vec![OFFICE_ID.into(), "source".into()];
    assert_eq!(
        office_route(
            &native,
            &process,
            "annual.report.xlsx",
            false,
            &[],
            &preferences,
            Some(0)
        ),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_SOURCE,
            detect_binary: false
        }),
    );
    preferences.overrides.insert(OFFICE_ID.into(), vec![]);
    assert_eq!(
        office_route(
            &native,
            &process,
            "annual.xlsx",
            false,
            &[],
            &preferences,
            Some(0)
        ),
        Some(Route::Native {
            index: 0,
            mode: viewer::MODE_SOURCE,
            detect_binary: false
        }),
    );
}

#[test]
fn routing_office_uses_selected_html_renderer_index_without_confusing_image_transport() {
    let process = [
        ("midnight.viewer.image", vec!["png".into()]),
        (HTML_ID, vec!["html".into()]),
    ];
    let preferences = ViewerAssociations::default();
    assert_eq!(
        office_route(
            &[],
            &process,
            "report.xlsx",
            false,
            &[],
            &preferences,
            Some(1)
        ),
        Some(Route::Office { renderer: 1 }),
    );
    assert_eq!(
        office_route(
            &[],
            &process,
            "report.xlsx",
            false,
            &[],
            &preferences,
            Some(0)
        ),
        None,
        "an image candidate cannot become the Office HTML renderer",
    );
    assert_eq!(
        office_route(
            &[],
            &process,
            "photo.png",
            false,
            &[],
            &preferences,
            Some(1)
        ),
        Some(Route::Process { index: 0 }),
    );
    assert_eq!(
        office_route(
            &[],
            &process,
            "page.html",
            false,
            &[],
            &preferences,
            Some(1)
        ),
        Some(Route::Process { index: 1 }),
    );
}
