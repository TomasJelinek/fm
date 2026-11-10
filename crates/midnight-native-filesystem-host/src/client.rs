use crate::{Cancel, NativeProvider, discovery::invalid, paths};
use midnight_filesystem_plugin_api::{ErrorCode, Failure, Kind, Node, Request, Response, abi};
use std::path::Path;

#[derive(Debug)]
pub(crate) struct Lease {
    pub(crate) provider: NativeProvider,
    close: Request,
}
impl Lease {
    pub(crate) fn new(provider: &NativeProvider, close: Request) -> Self {
        Self {
            provider: provider.clone(),
            close,
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Cleanup is independent of the canceled operation. The caller owns no
        // mutable source operation; these calls release only plugin resources.
        let _ = self.provider.transport.call(&self.close, &|| false);
    }
}

pub(crate) fn check_cancel(cancelled: Cancel<'_>) -> Result<(), Failure> {
    if cancelled() {
        Err(Failure::new(
            ErrorCode::Cancelled,
            "Filesystem operation canceled",
        ))
    } else {
        Ok(())
    }
}

impl NativeProvider {
    pub(crate) fn exchange(
        &self,
        request: Request,
        cancelled: Cancel<'_>,
    ) -> Result<Response, Failure> {
        check_cancel(cancelled)?;
        let response = self.transport.call(&request, cancelled)?;
        if let Response::Failed(error) = response {
            return Err(error);
        }
        let valid = matches!(
            (&request, &response),
            (Request::Resolve { .. }, Response::Resolved { .. })
                | (Request::ResolveText { .. }, Response::Resolved { .. })
                | (Request::Parent { .. }, Response::Parent(_))
                | (Request::Activate { .. }, Response::Activated(_))
                | (Request::Refresh { .. }, Response::Unit)
                | (Request::Stat { .. }, Response::Node(_))
                | (
                    Request::ListBegin { .. } | Request::ListNext { .. },
                    Response::Page { .. }
                )
                | (Request::OpenRead { .. }, Response::Opened { .. })
                | (Request::ValidateRead { .. }, Response::Unit)
                | (Request::ReadAt { .. }, Response::Content { .. })
                | (Request::Volumes, Response::Volumes(_))
                | (Request::StorageKind { .. }, Response::StorageKind(_))
                | (Request::SpaceInfo { .. }, Response::SpaceInfo { .. })
                | (Request::VolumeLabel { .. }, Response::VolumeLabel(_))
                | (
                    Request::VolumeInformation { .. },
                    Response::VolumeInformation(_)
                )
        );
        if !valid {
            // Even a wrong reply variant can publish a lease; close it before
            // reporting a protocol failure instead of leaking until process exit.
            let close = match response {
                Response::Resolved { mount, .. } if mount != 0 => {
                    Some(Request::CloseMount { mount })
                }
                Response::Page { cursor, .. } if cursor != 0 => Some(Request::CloseList { cursor }),
                Response::Opened { reader, .. } if reader != 0 => {
                    Some(Request::CloseRead { reader })
                }
                _ => None,
            };
            if let Some(close) = close {
                drop(Lease::new(self, close));
            }
            return Err(invalid("Filesystem returned an unexpected response"));
        }
        Ok(response)
    }

    pub(crate) fn resolve(
        &self,
        path: &Path,
        cancelled: Cancel<'_>,
    ) -> Result<(Lease, u64, Node), Failure> {
        self.require(abi::CAP_NATIVE_ADDRESS)?;
        let address = paths::encode(path)?;
        let Response::Resolved { mount, node } =
            self.exchange(Request::Resolve { address }, cancelled)?
        else {
            return Err(invalid("Missing resolved filesystem node"));
        };
        if mount == 0 {
            return Err(invalid("Invalid filesystem mount"));
        }
        let lease = Lease::new(self, Request::CloseMount { mount });
        check_cancel(cancelled)?;
        validate_node(&node)?;
        Ok((lease, mount, node))
    }

    /// Stream bounded batches. A false callback stops immediately and releases
    /// the cursor before its mount; it does not cancel another panel's listing.
    pub fn read_directory(
        &self,
        path: &Path,
        cancelled: Cancel<'_>,
        emit: &mut dyn FnMut(midnight_core::Entry) -> bool,
    ) -> Result<(), Failure> {
        self.require(abi::CAP_BROWSE)?;
        let (_mount, mount, directory) = self.resolve(path, cancelled)?;
        if !matches!(
            directory.kind,
            Kind::Directory | Kind::DirectoryLink | Kind::Symlink
        ) {
            return Err(invalid("The selected node is not a directory"));
        }
        let parent = paths::decode(&directory.address)?;
        let mut response = self.exchange(
            Request::ListBegin {
                mount,
                directory: directory.id,
                max_entries: midnight_filesystem_plugin_api::MAX_BATCH,
            },
            cancelled,
        )?;
        let cursor = match &response {
            Response::Page { cursor, .. } if *cursor != 0 => *cursor,
            _ => return Err(invalid("Invalid directory cursor")),
        };
        let _cursor = Lease::new(self, Request::CloseList { cursor });
        let mut expected_page = 0;
        let mut seen = std::collections::HashSet::new();
        loop {
            let Response::Page {
                cursor: reply_cursor,
                page,
                done,
                entries,
            } = response
            else {
                return Err(invalid("Missing directory page"));
            };
            if reply_cursor != cursor && reply_cursor != 0 {
                drop(Lease::new(
                    self,
                    Request::CloseList {
                        cursor: reply_cursor,
                    },
                ));
            }
            if reply_cursor != cursor
                || page != expected_page
                || entries.len() > midnight_filesystem_plugin_api::MAX_BATCH as usize
                || (!done && entries.is_empty())
            {
                return Err(invalid("Invalid directory page identity or sequence"));
            }
            check_cancel(cancelled)?;
            for node in entries {
                check_cancel(cancelled)?;
                validate_node(&node)?;
                if seen.len() >= 1_000_000 || !seen.insert(node.id) {
                    return Err(invalid("Directory node limit or repeated identity"));
                }
                let entry = super::services::entry(node)?;
                if entry.path.parent() != Some(parent.as_path()) {
                    return Err(invalid("Directory entry is outside its parent"));
                }
                if !emit(entry) {
                    return Ok(());
                }
            }
            if done {
                return Ok(());
            }
            expected_page = expected_page
                .checked_add(1)
                .ok_or_else(|| invalid("Directory page counter exhausted"))?;
            response = self.exchange(
                Request::ListNext {
                    cursor,
                    expected_page,
                    max_entries: midnight_filesystem_plugin_api::MAX_BATCH,
                },
                cancelled,
            )?;
        }
    }
}

pub(crate) fn validate_node(node: &Node) -> Result<(), Failure> {
    if node.id == 0 {
        return Err(invalid("Invalid filesystem node ID"));
    }
    paths::decode(&node.address)?;
    paths::name(node.name_encoding, &node.raw_name)?;
    if node
        .metadata
        .modified
        .is_some_and(|time| time.nanos >= 1_000_000_000)
    {
        return Err(invalid("Invalid filesystem timestamp"));
    }
    Ok(())
}
