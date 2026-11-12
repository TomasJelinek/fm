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
    )
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
