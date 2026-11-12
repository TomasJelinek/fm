//! Native desktop file chooser portal; filesystem bytes never round-trip through display text.
use crate::LinuxFileSystem;
use futures_lite::StreamExt;
use midnight_core::{
    Cancellation, FsError, PathPickerKind, PathPickerOwner, PathPickerRequest, PathPickerService,
};
use std::{
    collections::HashMap,
    io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use zbus::{
    Connection, MatchRule, MessageStream, Proxy,
    message::Type,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const REQUEST: &str = "org.freedesktop.portal.Request";
const CALL_LIMIT: Duration = Duration::from_secs(15);

impl PathPickerService for LinuxFileSystem {
    fn command_icon_resource_requires_import(&self) -> bool {
        true
    }

    fn store_command_icon(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
        cancel: &Cancellation,
    ) -> Result<PathBuf, FsError> {
        let result =
            midnight_platform_common::icon_snapshot::SnapshotPixels::from_rgba(width, height, rgba)
                .and_then(|pixels| {
                    crate::icon_storage::save(self.home.as_deref(), &pixels, cancel)
                });
        result.map_err(|cause| FsError::from_io("store command icon", Path::new(""), cause))
    }

    fn pick_path(
        &self,
        request: &PathPickerRequest,
        cancel: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        request.validate()?;
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(error)?;
        if request.kind == PathPickerKind::CommandIconResource {
            let mut artwork = request.clone();
            artwork.initial_directory = crate::icons::theme_directory(self.home.as_deref());
            let Some(path) = runtime.block_on(choose(&artwork, cancel))? else {
                return Ok(None);
            };
            return crate::icon_picker::snapshot(&path, self.home.as_deref(), cancel)
                .map_err(error);
        }
        runtime.block_on(choose(request, cancel))
    }
}

fn error(cause: impl std::fmt::Display) -> FsError {
    FsError::from_io(
        "choose path",
        Path::new(""),
        io::Error::other(cause.to_string()),
    )
}

async fn bounded<T>(
    future: impl std::future::Future<Output = zbus::Result<T>>,
) -> Result<T, FsError> {
    tokio::time::timeout(CALL_LIMIT, future)
        .await
        .map_err(|_| error("The desktop file chooser did not respond."))?
        .map_err(error)
}

fn picker_options<'a>(
    input: &PathPickerRequest,
    token: &'a str,
    modal: bool,
) -> (&'static str, HashMap<&'static str, Value<'a>>) {
    let mut options = HashMap::from([
        ("handle_token", Value::from(token)),
        ("modal", Value::from(modal)),
    ]);
    let method = if let Some(name) = input.kind.suggested_save_name() {
        options.insert("current_name", Value::from(name));
        "SaveFile"
    } else {
        options.insert("multiple", Value::from(false));
        options.insert(
            "directory",
            Value::from(input.kind == PathPickerKind::Directory),
        );
        "OpenFile"
    };
    if let Some(folder) = &input.initial_directory {
        let mut bytes = folder.as_os_str().as_bytes().to_vec();
        bytes.push(0);
        options.insert("current_folder", Value::from(bytes));
    }
    let specific = match input.kind {
        PathPickerKind::Image | PathPickerKind::CommandIconImage => {
            Some(("Images", vec![(1u32, "image/*".to_owned())]))
        }
        PathPickerKind::CommandIconResource => Some((
            "PNG or SVG icons",
            vec![
                (1u32, "image/png".to_owned()),
                (1u32, "image/svg+xml".to_owned()),
            ],
        )),
        PathPickerKind::ChecksumOpen => Some((
            "Checksum files",
            ["*.sfv", "*.md5", "*.sha1", "*.sha256", "*.sha512"]
                .into_iter()
                .map(|pattern| (0, pattern.to_owned()))
                .collect(),
        )),
        PathPickerKind::ChecksumSave(algorithm) => Some((
            algorithm.label(),
            vec![(0, format!("*.{}", algorithm.extension()))],
        )),
        PathPickerKind::TextSave => Some(("Text files", vec![(0, "*.txt".to_owned())])),
        _ => None,
    };
    if let Some(specific) = specific {
        if input.kind.save_extension().is_some() {
            options.insert("current_filter", Value::from(specific.clone()));
        }
        options.insert(
            "filters",
            Value::from(vec![specific, ("All files", vec![(0, "*".to_owned())])]),
        );
    }
    (method, options)
}

fn finish_selection(kind: PathPickerKind, mut path: PathBuf) -> PathBuf {
    // The portal offers a suggested name but no default-extension option. Match
    // native Save behavior without re-encoding the user's path or replacing an
    // explicitly entered suffix. The export worker still validates any conflict.
    if let Some(extension) = kind.save_extension()
        && path.extension().is_none()
    {
        path.set_extension(extension);
    }
    path
}

async fn choose(
    input: &PathPickerRequest,
    cancel: &Cancellation,
) -> Result<Option<PathBuf>, FsError> {
    let connection = bounded(Connection::session()).await?;
    let unique = connection
        .unique_name()
        .ok_or_else(|| error("The desktop bus has no connection identity."))?;
    let sender = unique.as_str().trim_start_matches(':').replace('.', "_");
    let namespace = format!("/org/freedesktop/portal/desktop/request/{sender}");
    // Subscribe before showing: a fast chooser may reply before its method returns.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(PORTAL)
        .map_err(error)?
        .interface(REQUEST)
        .map_err(error)?
        .member("Response")
        .map_err(error)?
        .path_namespace(namespace.as_str())
        .map_err(error)?
        .build();
    let mut responses = bounded(MessageStream::for_match_rule(rule, &connection, Some(8))).await?;
    let chooser = bounded(Proxy::new(
        &connection,
        PORTAL,
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.FileChooser",
    ))
    .await?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let token = format!("midnight_{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let predicted_path = format!("{namespace}/{token}");
    let predicted_request = bounded(Proxy::new(
        &connection,
        PORTAL,
        predicted_path.as_str(),
        REQUEST,
    ))
    .await?;
    let parent = match input.owner {
        PathPickerOwner::X11(id) if id != 0 => format!("x11:{id:x}"),
        _ => String::new(),
    };
    let (method, options) = picker_options(input, &token, !parent.is_empty());
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let handle: OwnedObjectPath =
        match bounded(chooser.call(method, &(parent, &input.title, options))).await {
            Ok(handle) => handle,
            Err(cause) => {
                // The method may have reached the portal before its reply timed out.
                close(&predicted_request).await;
                return Err(cause);
            }
        };
    if !handle.as_str().starts_with(&(namespace.clone() + "/")) {
        close(&predicted_request).await;
        return Err(error(
            "The desktop returned a chooser owned by another request.",
        ));
    }
    let request = bounded(Proxy::new(&connection, PORTAL, handle.as_str(), REQUEST)).await?;
    let result = loop {
        tokio::select! {
            message = responses.next() => {
                let Some(message) = message else { break Err(error("The desktop chooser disconnected.")) };
                let message = match message { Ok(message) => message, Err(cause) => break Err(error(cause)) };
                if message.header().path().map(|path| path.as_str()) != Some(handle.as_str()) { continue; }
                let response = message.body().deserialize::<(u32, HashMap<String, OwnedValue>)>().map_err(error);
                // A Response completes/removes the portal request. Avoid a second
                // native round-trip before returning the user's selected path.
                return if cancel.is_cancelled() { Ok(None) } else {
                    response.and_then(decode_response).map(|path| path.map(|path| finish_selection(input.kind, path)))
                };
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if cancel.is_cancelled() { break Ok(None); }
            }
        }
    };
    close(&request).await;
    if cancel.is_cancelled() {
        Ok(None)
    } else {
        result
    }
}

async fn close(request: &Proxy<'_>) {
    let _: Result<zbus::Result<()>, _> =
        tokio::time::timeout(Duration::from_secs(2), request.call("Close", &())).await;
}

fn decode_response(
    (status, mut values): (u32, HashMap<String, OwnedValue>),
) -> Result<Option<PathBuf>, FsError> {
    if status == 1 {
        return Ok(None);
    }
    if status != 0 {
        return Err(error("The desktop could not select a path."));
    }
    let uris = Vec::<String>::try_from(
        values
            .remove("uris")
            .ok_or_else(|| error("The desktop returned no file path."))?,
    )
    .map_err(error)?;
    if uris.len() != 1 {
        return Err(error("Choose exactly one file or folder."));
    }
    let mut paths = crate::file_clipboard::decode_uris(uris[0].as_bytes())?;
    if paths.len() != 1 {
        return Err(error("The desktop returned an invalid single path."));
    }
    Ok(paths.pop())
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::ChecksumAlgorithm;
    use std::{
        ffi::OsString,
        os::unix::ffi::{OsStrExt, OsStringExt},
    };

    fn request(kind: PathPickerKind) -> PathPickerRequest {
        PathPickerRequest {
            kind,
            title: "Checksum soubory 中文".into(),
            initial_directory: Some(PathBuf::from(OsString::from_vec(
                b"/tmp/raw\xff folder".to_vec(),
            ))),
            owner: PathPickerOwner::X11(42),
        }
    }

    #[test]
    fn checksum_portal_open_requests_all_formats_and_preserves_raw_initial_folder() {
        let (method, mut options) =
            picker_options(&request(PathPickerKind::ChecksumOpen), "test", true);
        assert_eq!(method, "OpenFile");
        assert!(!bool::try_from(options.remove("multiple").unwrap()).unwrap());
        assert!(!bool::try_from(options.remove("directory").unwrap()).unwrap());
        assert!(bool::try_from(options.remove("modal").unwrap()).unwrap());
        assert_eq!(
            Vec::<u8>::try_from(options.remove("current_folder").unwrap()).unwrap(),
            b"/tmp/raw\xff folder\0"
        );
        let filters =
            Vec::<(String, Vec<(u32, String)>)>::try_from(options.remove("filters").unwrap())
                .unwrap();
        assert_eq!(
            filters,
            vec![
                (
                    "Checksum files".into(),
                    vec![
                        (0, "*.sfv".into()),
                        (0, "*.md5".into()),
                        (0, "*.sha1".into()),
                        (0, "*.sha256".into()),
                        (0, "*.sha512".into()),
                    ]
                ),
                ("All files".into(), vec![(0, "*".into())]),
            ]
        );
        assert!(!options.contains_key("current_name"));
    }

    #[test]
    fn checksum_portal_save_suggests_the_selected_format_and_uses_savefile() {
        for (algorithm, name, pattern) in [
            (ChecksumAlgorithm::Crc32, "checksums.sfv", "*.sfv"),
            (ChecksumAlgorithm::Md5, "checksums.md5", "*.md5"),
            (ChecksumAlgorithm::Sha1, "checksums.sha1", "*.sha1"),
            (ChecksumAlgorithm::Sha256, "checksums.sha256", "*.sha256"),
            (ChecksumAlgorithm::Sha512, "checksums.sha512", "*.sha512"),
        ] {
            let (method, mut options) = picker_options(
                &request(PathPickerKind::ChecksumSave(algorithm)),
                "test",
                false,
            );
            assert_eq!(method, "SaveFile");
            assert_eq!(
                String::try_from(options.remove("current_name").unwrap()).unwrap(),
                name
            );
            assert!(!options.contains_key("multiple"));
            assert!(!options.contains_key("directory"));
            let filters =
                Vec::<(String, Vec<(u32, String)>)>::try_from(options.remove("filters").unwrap())
                    .unwrap();
            assert_eq!(filters[0].1, vec![(0, pattern.into())]);
            let selected =
                <(String, Vec<(u32, String)>)>::try_from(options.remove("current_filter").unwrap())
                    .unwrap();
            assert_eq!(selected, filters[0]);
        }
    }

    #[test]
    fn text_portal_save_uses_text_filter_and_savefile_without_open_only_options() {
        let (method, mut options) =
            picker_options(&request(PathPickerKind::TextSave), "test", true);
        assert_eq!(method, "SaveFile");
        assert_eq!(
            String::try_from(options.remove("current_name").unwrap()).unwrap(),
            "file-list.txt"
        );
        assert!(!options.contains_key("multiple"));
        assert!(!options.contains_key("directory"));
        assert!(bool::try_from(options.remove("modal").unwrap()).unwrap());
        let filters =
            Vec::<(String, Vec<(u32, String)>)>::try_from(options.remove("filters").unwrap())
                .unwrap();
        assert_eq!(
            filters,
            vec![
                ("Text files".into(), vec![(0, "*.txt".into())]),
                ("All files".into(), vec![(0, "*".into())]),
            ]
        );
        let selected =
            <(String, Vec<(u32, String)>)>::try_from(options.remove("current_filter").unwrap())
                .unwrap();
        assert_eq!(selected, filters[0]);
    }

    #[test]
    fn text_save_adds_only_a_missing_suffix_and_keeps_raw_native_path_bytes() {
        let kind = PathPickerKind::TextSave;
        let original = PathBuf::from(OsString::from_vec(b"/tmp/raw\xff name".to_vec()));
        assert_eq!(
            finish_selection(kind, original).as_os_str().as_bytes(),
            b"/tmp/raw\xff name.txt"
        );
        let explicit = PathBuf::from("/tmp/list.custom");
        assert_eq!(finish_selection(kind, explicit.clone()), explicit);
    }

    #[test]
    fn checksum_save_supplies_a_missing_extension_without_changing_native_path_bytes() {
        let kind = PathPickerKind::ChecksumSave(ChecksumAlgorithm::Sha256);
        let path = finish_selection(
            kind,
            PathBuf::from(OsString::from_vec(b"/tmp/raw\xff name".to_vec())),
        );
        assert_eq!(path.as_os_str().as_bytes(), b"/tmp/raw\xff name.sha256");
        for kind in [kind, PathPickerKind::ChecksumOpen] {
            let original = PathBuf::from("/tmp/checksums.CUSTOM");
            assert_eq!(finish_selection(kind, original.clone()), original);
        }
        assert_eq!(
            finish_selection(PathPickerKind::ChecksumOpen, PathBuf::from("/tmp/manifest")),
            PathBuf::from("/tmp/manifest")
        );
    }

    fn response(uri: &str) -> (u32, HashMap<String, OwnedValue>) {
        (
            0,
            HashMap::from([(
                "uris".into(),
                OwnedValue::try_from(Value::from(vec![uri])).unwrap(),
            )]),
        )
    }

    #[test]
    fn portal_selection_preserves_native_non_unicode_bytes_and_long_paths() {
        let path = decode_response(response("file:///tmp/raw%FF%20name"))
            .unwrap()
            .unwrap();
        assert_eq!(path.as_os_str().as_bytes(), b"/tmp/raw\xff name");
        let uri = format!("file:///{}leaf", "folder/".repeat(600));
        assert!(
            decode_response(response(&uri))
                .unwrap()
                .unwrap()
                .as_os_str()
                .len()
                > 4096
        );
    }

    #[test]
    fn portal_cancel_is_distinct_from_malformed_or_remote_results() {
        assert!(decode_response((1, HashMap::new())).unwrap().is_none());
        for uri in [
            "https://example.org/a",
            "file://server/share/a",
            "file:///tmp/a%00b",
            "file:///tmp/%GG",
            "file:///one\nfile:///two",
        ] {
            assert!(decode_response(response(uri)).is_err(), "{uri}");
        }
        assert!(decode_response((2, HashMap::new())).is_err());
    }
}
