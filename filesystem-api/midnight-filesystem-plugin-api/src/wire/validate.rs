use crate::*;

fn require(value: bool) -> Result<(), WireError> {
    if value {
        Ok(())
    } else {
        Err(WireError::Malformed)
    }
}
fn native(bytes: &[u8], windows: bool) -> Result<(), WireError> {
    if windows {
        let (units, remainder) = bytes.as_chunks::<2>();
        require(remainder.is_empty() && !units.contains(&[0, 0]))
    } else {
        require(!bytes.contains(&0))
    }
}
pub(super) fn address(value: &Address) -> Result<(), WireError> {
    match value.encoding {
        AddressEncoding::Opaque => Ok(()),
        AddressEncoding::WindowsUtf16Le => {
            require(!value.bytes.is_empty())?;
            native(&value.bytes, true)
        }
        AddressEncoding::UnixBytes => {
            require(!value.bytes.is_empty())?;
            native(&value.bytes, false)
        }
    }
}
fn node(value: &Node) -> Result<(), WireError> {
    require(value.id != 0)?;
    address(&value.address)?;
    match value.name_encoding {
        NameEncoding::Opaque => {}
        NameEncoding::Utf8 => {
            require(std::str::from_utf8(&value.raw_name).is_ok())?;
        }
        NameEncoding::WindowsUtf16Le => native(&value.raw_name, true)?,
        NameEncoding::UnixBytes => native(&value.raw_name, false)?,
    }
    require(
        value
            .metadata
            .modified
            .is_none_or(|time| time.nanos < 1_000_000_000),
    )?;
    require(value.kind == Kind::File || value.metadata.size.is_none())
}
pub(super) fn request(value: &Request) -> Result<(), WireError> {
    match value {
        Request::Resolve { address: value }
        | Request::StorageKind { address: value }
        | Request::SpaceInfo { address: value }
        | Request::VolumeLabel { address: value }
        | Request::VolumeInformation { address: value } => address(value),
        Request::Stat { mount, node }
        | Request::Activate { mount, node }
        | Request::Refresh { mount, node }
        | Request::Parent { mount, node }
        | Request::OpenRead { mount, node }
        | Request::PrepareRename { mount, node } => require(*mount != 0 && *node != 0),
        Request::ListBegin {
            mount,
            directory,
            max_entries,
        } => require(*mount != 0 && *directory != 0 && (1..=MAX_BATCH).contains(max_entries)),
        Request::ListNext {
            cursor,
            expected_page,
            max_entries,
        } => require(*cursor != 0 && *expected_page != 0 && (1..=MAX_BATCH).contains(max_entries)),
        Request::CloseList { cursor } => require(*cursor != 0),
        Request::CloseMount { mount } => require(*mount != 0),
        Request::CloseRead { reader } | Request::ValidateRead { reader } => require(*reader != 0),
        Request::ReadAt {
            reader,
            offset,
            length,
        } => require(
            *reader != 0
                && (1..=MAX_READ).contains(length)
                && offset.checked_add(u64::from(*length)).is_some(),
        ),
        Request::Rename {
            prepared,
            attempt,
            name,
            policy,
        } => {
            require(*prepared != 0 && *attempt != 0)?;
            name.validate_rename_basename()?;
            match policy {
                RenamePolicy::NoReplace => Ok(()),
                RenamePolicy::RecoverableCurrentEntry { conflict } => require(*conflict != 0),
            }
        }
        Request::GetRenameState { prepared } | Request::CloseRename { prepared } => {
            require(*prepared != 0)
        }
        Request::Volumes => Ok(()),
        Request::ResolveText { text } => require(!text.is_empty() && !text.contains('\0')),
    }
}
pub(super) fn response(value: &Response) -> Result<(), WireError> {
    match value {
        Response::RenamePrepared(value) => value.validate(),
        Response::RenameState(value) => value.validate(),
        Response::Activated(target) => {
            let (ActivationTarget::WithinProvider(value) | ActivationTarget::Native(value)) =
                target;
            address(value)?;
            target.validate()
        }
        Response::Unit
        | Response::Parent(None)
        | Response::StorageKind(_)
        | Response::SpaceInfo { .. } => Ok(()),
        Response::Resolved { mount, node: value } => {
            require(*mount != 0)?;
            node(value)
        }
        Response::Node(value) | Response::Parent(Some(value)) => node(value),
        Response::Page {
            cursor,
            entries,
            done,
            ..
        } => {
            require(
                *cursor != 0
                    && (entries.len() <= MAX_BATCH as usize)
                    && (!entries.is_empty() || *done),
            )?;
            for entry in entries {
                node(entry)?;
            }
            Ok(())
        }
        Response::Opened {
            reader, revision, ..
        } => require(*reader != 0 && (1..=MAX_REVISION).contains(&revision.len())),
        Response::Content { bytes, eof } => {
            require(bytes.len() <= MAX_READ as usize && (!bytes.is_empty() || *eof))
        }
        Response::Volumes(values) => {
            require(values.len() <= 4096)?;
            for value in values {
                require(!value.id.is_empty())?;
                address(&value.root)?;
            }
            Ok(())
        }
        Response::Failed(failure) => require(failure.message.len() <= MAX_DIAGNOSTIC),
        Response::VolumeLabel(value) => native_text(value),
        Response::VolumeInformation(value) => {
            if let Some(root) = &value.root {
                address(root)?;
            }
            native_text(&value.label)?;
            native_text(&value.filesystem)?;
            native_text(&value.source)?;
            if let Some(total) = value.capacity.total {
                require(value.capacity.free.is_none_or(|free| free <= total))?;
                require(
                    value
                        .capacity
                        .available
                        .is_none_or(|available| available <= total),
                )?;
            }
            require(value.properties.len() <= 14)?;
            for (index, property) in value.properties.iter().enumerate() {
                require(!value.properties[..index].contains(property))?;
            }
            require(value.unavailable.len() <= MAX_VOLUME_DIAGNOSTICS)?;
            for diagnostic in &value.unavailable {
                require(diagnostic.len() <= MAX_DIAGNOSTIC)?;
            }
            Ok(())
        }
    }
}
fn native_text(value: &Option<NativeText>) -> Result<(), WireError> {
    let Some(value) = value else { return Ok(()) };
    require(value.bytes.len() <= MAX_VOLUME_TEXT)?;
    match value.encoding {
        NameEncoding::Opaque => Err(WireError::Unsupported),
        NameEncoding::Utf8 => {
            require(std::str::from_utf8(&value.bytes).is_ok())?;
            native(&value.bytes, false)
        }
        NameEncoding::WindowsUtf16Le => native(&value.bytes, true),
        NameEncoding::UnixBytes => native(&value.bytes, false),
    }
}
