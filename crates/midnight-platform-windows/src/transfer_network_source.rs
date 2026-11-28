//! Copy sources on a network filesystem without a usable file identifier.
use super::*;
use std::{
    fs::OpenOptions,
    os::windows::fs::OpenOptionsExt,
    path::{Component, Components, Prefix},
};

/// Each open participates in share checks (FILE_TRAVERSE), denying deletion and
/// rename. Holding the ancestors keeps the observed physical name bound to this path.
pub(super) struct Ancestors {
    _directories: Vec<File>,
}

/// Some servers identify the selected folder but omit IDs on individual children; FAT,
/// exFAT, CD and DVD volumes have none at all (`reads_without_ids`).
pub(super) fn missing_id(
    failure: &FsError,
    access: Access,
    file: &File,
    path: &Path,
) -> Result<bool, FsError> {
    Ok(!matches!(access, Access::Move(_))
        && failure.kind == io::ErrorKind::Unsupported
        && super::super::network_identity::reads_without_ids(file, path)?)
}

pub(super) fn open(
    parent: &File,
    parent_path: &Path,
    name: &OsStr,
    access: Access,
    cancel: &Cancellation,
) -> Result<Source, FsError> {
    let physical = super::super::destination::final_path(parent, VOLUME_NAME_DOS)
        .map_err(|e| error(parent_path, e))?;
    let (root, components) = split_root(&physical)?;
    checked(cancel, parent_path)?;
    let mut pinned = OpenOptions::new()
        .access_mode(FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root)
        .map_err(|e| error(parent_path, e))?;
    let mut directories = Vec::new();
    for component in components {
        checked(cancel, parent_path)?;
        let Component::Normal(name) = component else {
            return Err(invalid(parent_path, "invalid network source parent"));
        };
        let child = native::open_relative(
            &pinned,
            name,
            FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_DIRECTORY_FILE,
        )
        .map_err(|e| error(parent_path, e))?;
        if Native::query(&child)
            .map_err(|e| error(parent_path, e))?
            .link()
        {
            return Err(unsupported(
                parent_path,
                "a folder in the network source resolves through a link",
            ));
        }
        directories.push(pinned);
        pinned = child;
    }
    let binding = super::super::network_identity::binding(&pinned, parent_path)?;
    if super::super::network_identity::binding(parent, parent_path)? != binding {
        return Err(invalid(parent_path, "source parent changed while opening"));
    }
    Source::open_with_access(
        Parent {
            file: Arc::new(pinned),
            path: parent_path.to_owned(),
            identity: Some(binding),
            ancestry: None,
            network: Some(Arc::new(Ancestors {
                _directories: directories,
            })),
        },
        name,
        None,
        access,
        cancel,
    )
}

/// Windows can report a bare UNC share prefix for the share root. In particular,
/// Rust emits no separate RootDir component for that verbatim path.
fn split_root(physical: &Path) -> Result<(PathBuf, Components<'_>), FsError> {
    let mut components = physical.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(invalid(physical, "missing network source root"));
    };
    if components.clone().next() == Some(Component::RootDir) {
        components.next();
    } else if !matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..)) {
        return Err(invalid(physical, "missing network source root"));
    }
    let mut root = prefix.as_os_str().to_owned();
    root.push(r"\");
    Ok((PathBuf::from(root), components))
}

pub(super) fn revalidate_parent(path: &Path, expected: TransferIdentity) -> Result<(), FsError> {
    let visible = native::open_parent(path).map_err(|e| error(path, e))?;
    if super::super::network_identity::binding(&visible, path)? != expected {
        return Err(invalid(path, "network source parent changed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn unc_share_root_is_present_with_or_without_a_trailing_separator() {
        for prefix in [r"\\host\share", r"\\?\UNC\host\share"] {
            let expected = format!(r"{prefix}\");
            for suffix in ["", r"\", r"\folder\subfolder"] {
                let path = PathBuf::from(format!("{prefix}{suffix}"));
                let (root, remaining) = split_root(&path).unwrap();
                assert_eq!(root.as_os_str(), OsStr::new(&expected));
                assert_eq!(
                    remaining.as_path(),
                    Path::new(suffix.trim_start_matches('\\'))
                );
            }
        }
        for prefix in ["C:", r"\\?\C:"] {
            let path = PathBuf::from(format!(r"{prefix}\folder"));
            let (root, remaining) = split_root(&path).unwrap();
            assert_eq!(root.as_os_str(), OsStr::new(&format!(r"{prefix}\")));
            assert_eq!(remaining.as_path(), Path::new("folder"));
        }
    }

    #[cfg(feature = "test-fixtures")]
    #[test]
    fn a_network_server_can_omit_the_id_of_one_child_in_an_identified_folder() {
        use crate::test_network_ids::{Response, Scope};
        let temp = crate::test_fixtures::build_temp("network-child-id-");
        let file = temp.path().join("child");
        fs::write(&file, b"read this").unwrap();
        let _missing = Scope::new(&file, Response::AllZero).unwrap();
        let cancel = Cancellation::default();
        let mut root = crate::WindowsFileSystem::new(None)
            .prepare_transfer_source(temp.path(), &cancel)
            .unwrap();
        assert!(root.native_identity().is_some());
        let mut child = root.next_child(&cancel).unwrap().unwrap();
        assert!(child.native_identity().is_none());
        assert!(child.requires_copy_directory_guard());
        let mut data = [0; 9];
        assert_eq!(child.read_chunk(&mut data, &cancel).unwrap(), 9);
        assert_eq!(&data, b"read this");
    }

    /// A folder on a local FAT32 volume is read like a share without IDs (`reads_without_ids`),
    /// so its identity is a binding. A destination below it on the same volume must be
    /// recognized by the same kind of identity, or Copy would copy the folder into itself.
    #[cfg(feature = "test-fixtures")]
    #[test]
    fn fat_folder_copy_rejects_its_own_descendant_as_destination() {
        use crate::test_network_ids::{Response, Scope};
        let temp = crate::test_fixtures::build_temp("fat-descendant-");
        let stick = temp.path().join("stick");
        let source = stick.join("source");
        let child = source.join("child");
        fs::create_dir_all(child.join("grandchild")).unwrap();
        let _fat = Scope::new(&stick, Response::Fat).unwrap();
        let provider = crate::WindowsFileSystem::new(None);
        let cancel = Cancellation::default();
        let held = provider.prepare_transfer_source(&source, &cancel).unwrap();
        assert!(held.native_identity().is_none());
        assert!(held.requires_copy_directory_guard());
        let identity = held.entry().identity;
        for path in [child.clone(), child.join("grandchild"), source.clone()] {
            let target = provider
                .prepare_transfer_destination(&path, &cancel)
                .unwrap();
            let error = target
                .reject_source_ancestor(identity, &cancel)
                .unwrap_err();
            assert_eq!(
                error.kind,
                io::ErrorKind::InvalidInput,
                "{path:?}: {error:?}"
            );
            let ancestors = target.ancestor_identities(&cancel).unwrap().unwrap();
            assert!(ancestors.contains(&identity), "{path:?}");
        }
        // A sibling of the source on the same volume is no descendant.
        let sibling = stick.join("sibling");
        fs::create_dir(&sibling).unwrap();
        provider
            .prepare_transfer_destination(&sibling, &cancel)
            .unwrap()
            .reject_source_ancestor(identity, &cancel)
            .unwrap();
    }

    #[test]
    fn network_copy_holds_the_exact_source_through_read_retry_and_discovery() {
        let temp = crate::test_fixtures::build_temp("network-source-");
        let root = temp.path().join("directory");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("data.bin"), b"original contents").unwrap();
        let cancel = Cancellation::default();
        let mut directory = open(
            &native::open_parent(temp.path()).unwrap(),
            temp.path(),
            OsStr::new("directory"),
            Access::Copy(CopyLinkPolicy::FollowLinks),
            &cancel,
        )
        .unwrap();
        assert!(directory.native_namespace_scope(&cancel).unwrap().is_none());
        let mut listed = Vec::new();
        directory
            .discover(&cancel, &mut |item| listed.push(item.path.clone()))
            .unwrap();
        assert_eq!(listed, [root.clone(), root.join("data.bin")]);
        let mut child = directory.next_child(&cancel).unwrap().unwrap();
        assert!(directory.next_child(&cancel).unwrap().is_none());
        assert!(fs::rename(root.join("data.bin"), root.join("renamed")).is_err());
        assert!(fs::write(root.join("data.bin"), b"new data").is_err());
        let mut buffer = [0; 8];
        assert_eq!(child.read_chunk(&mut buffer, &cancel).unwrap(), 8);
        assert_eq!(&buffer, b"original");
        child.retry_read_from(9, &cancel).unwrap();
        assert_eq!(child.read_chunk(&mut buffer, &cancel).unwrap(), 8);
        assert_eq!(&buffer, b"contents");
        let guard = child.rewind_for_validation(&cancel).unwrap();
        guard.revalidate(&cancel).unwrap();
    }
}
