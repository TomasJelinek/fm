use crate::size_warnings::{ReportLog, ReportRange};
use midnight_core::{
    ArchiveAddress, ArchiveMount, ArchiveNode, Cancellation, EntryKind, FolderSize, FsError,
    VirtualEntryId,
};
use std::{collections::HashSet, fmt, io, path::Path, sync::Arc};

pub(super) const MAX_NODES: usize = 100_000;
const MAX_NAMES: usize = 8 * 1024 * 1024;

struct Child {
    id: VirtualEntryId,
    raw_name: Arc<[u8]>,
}
struct Frame {
    id: VirtualEntryId,
    children: std::vec::IntoIter<Child>,
    size: FolderSize,
    report: Arc<ReportRange>,
}
impl Frame {
    fn publication(&mut self) -> FolderSize {
        self.size.warnings = self.report.handle();
        self.size.clone()
    }
}

/// A diagnostic representation only. Archive components never become a native path.
struct Location<'a> {
    address: &'a ArchiveAddress,
    path: &'a [Arc<[u8]>],
    provider_path: Option<&'a Path>,
}
impl fmt::Display for Location<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "archive {:?}; container {:?}; members [",
            self.address.provider, self.address.container
        )?;
        let mut separator = "";
        for raw in self
            .address
            .components
            .iter()
            .map(Vec::as_slice)
            .chain(self.path.iter().map(AsRef::as_ref))
        {
            write!(f, "{separator}{}", RawName(raw))?;
            separator = ", ";
        }
        f.write_str("]")?;
        if let Some(path) = self
            .provider_path
            .filter(|path| !path.as_os_str().is_empty())
        {
            write!(f, "; provider path {path:?}")?;
        }
        Ok(())
    }
}
struct RawName<'a>(&'a [u8]);
impl fmt::Display for RawName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(text) = std::str::from_utf8(self.0) {
            return write!(f, "{text:?}");
        }
        f.write_str("\"")?;
        for byte in self.0 {
            match byte {
                b'\n' => f.write_str("\\n")?,
                b'\r' => f.write_str("\\r")?,
                b'\t' => f.write_str("\\t")?,
                b'\\' => f.write_str("\\\\")?,
                b'"' => f.write_str("\\\"")?,
                b' '..=b'~' => write!(f, "{}", char::from(*byte))?,
                _ => write!(f, "\\x{byte:02x}")?,
            }
        }
        f.write_str("\"")
    }
}
fn valid_component(raw: &[u8]) -> bool {
    !raw.is_empty()
        && raw != b"."
        && raw != b".."
        && !raw.iter().any(|byte| b"/\\:\0".contains(byte))
}

struct Scan<'a> {
    mount: &'a dyn ArchiveMount,
    address: &'a ArchiveAddress,
    cancel: &'a Cancellation,
    log: ReportLog,
    seen: HashSet<u64>,
    names: usize,
    path: Vec<Arc<[u8]>>,
}
impl Scan<'_> {
    fn issue(&self, size: &mut FolderSize, operation: &str, kind: io::ErrorKind, reason: &str) {
        size.errors = size.errors.saturating_add(1);
        self.log.record(
            Location {
                address: self.address,
                path: &self.path,
                provider_path: None,
            },
            operation,
            kind,
            reason,
        );
    }
    fn failure(&self, size: &mut FolderSize, error: &FsError) -> bool {
        if error.kind == io::ErrorKind::Interrupted && self.cancel.is_cancelled() {
            return false;
        }
        size.errors = size.errors.saturating_add(1);
        self.log.record(
            Location {
                address: self.address,
                path: &self.path,
                provider_path: Some(&error.path),
            },
            error.operation,
            error.kind,
            &error.message,
        );
        true
    }
    fn validate(&self, size: &mut FolderSize) -> bool {
        if self.cancel.is_cancelled() {
            return false;
        }
        match self.mount.validate() {
            Ok(()) => !self.cancel.is_cancelled(),
            Err(error) => {
                self.failure(size, &error);
                false
            }
        }
    }
    fn add(&self, total: &mut u64, amount: u64, errors: &mut u64, field: &str) {
        if let Some(value) = total.checked_add(amount) {
            *total = value;
        } else {
            *total = u64::MAX;
            *errors = errors.saturating_add(1);
            self.log.record(
                Location {
                    address: self.address,
                    path: &self.path,
                    provider_path: None,
                },
                field,
                io::ErrorKind::InvalidData,
                "Archive size total exceeds the supported integer range",
            );
        }
    }
    fn merge(&self, total: &mut FolderSize, child: &FolderSize) {
        total.errors = total.errors.saturating_add(child.errors);
        self.add(
            &mut total.bytes,
            child.bytes,
            &mut total.errors,
            "sum archive bytes",
        );
        self.add(
            &mut total.files,
            child.files,
            &mut total.errors,
            "sum archive files",
        );
        self.add(
            &mut total.skipped_links,
            child.skipped_links,
            &mut total.errors,
            "sum skipped archive links",
        );
    }
    fn frame(&mut self, id: VirtualEntryId) -> (Frame, bool) {
        let mut frame = Frame {
            id,
            children: Vec::new().into_iter(),
            size: FolderSize::default(),
            report: self.log.mark().report(),
        };
        if !self.validate(&mut frame.size) {
            return (frame, false);
        }
        if !valid_component(self.path.last().expect("current directory")) {
            self.issue(
                &mut frame.size,
                "validate archive member",
                io::ErrorKind::InvalidData,
                "Invalid archive member component",
            );
            return (frame, true);
        }
        let children = match self.mount.list(id.entry, self.cancel) {
            Ok(rows) => rows,
            Err(error) => {
                self.failure(&mut frame.size, &error);
                let keep_going = self.validate(&mut frame.size);
                return (frame, keep_going);
            }
        };
        if !self.validate(&mut frame.size) {
            return (frame, false);
        }
        if children.len() > MAX_NODES {
            self.issue(
                &mut frame.size,
                "list archive directory",
                io::ErrorKind::OutOfMemory,
                "Archive directory listing exceeds the existing node budget",
            );
            return (frame, true);
        }
        let mut directories = Vec::new();
        for node in children {
            if self.cancel.is_cancelled() {
                return (frame, false);
            }
            self.names = self
                .names
                .saturating_add(node.raw_name.len())
                .saturating_add(node.display_name.len());
            self.path.push(node.raw_name.clone());
            if self.names > MAX_NAMES || self.seen.len() >= MAX_NODES {
                self.issue(
                    &mut frame.size,
                    "retain archive metadata",
                    io::ErrorKind::OutOfMemory,
                    "Archive size traversal exceeds the existing name or node budget",
                );
                self.path.pop();
                break;
            }
            if !valid_component(&node.raw_name) {
                self.issue(
                    &mut frame.size,
                    "validate archive member",
                    io::ErrorKind::InvalidData,
                    "Invalid archive member component",
                );
            } else if node.parent != id.entry
                || node.id.mount != self.mount.id()
                || node.id.entry <= 1
                || !self.seen.insert(node.id.entry)
            {
                self.issue(&mut frame.size, "validate archive member", io::ErrorKind::InvalidData, "Archive member has a foreign parent or mount, reserved identity, duplicate identity, or cycle");
            } else {
                match node.kind {
                    EntryKind::Directory => directories.push(Child {
                        id: node.id,
                        raw_name: node.raw_name,
                    }),
                    EntryKind::File => {
                        self.add(
                            &mut frame.size.files,
                            1,
                            &mut frame.size.errors,
                            "count archive files",
                        );
                        if let Some(length) = node.size {
                            self.add(
                                &mut frame.size.bytes,
                                length,
                                &mut frame.size.errors,
                                "sum archive bytes",
                            );
                        } else {
                            self.issue(
                                &mut frame.size,
                                "read archive size",
                                io::ErrorKind::InvalidData,
                                "Archive file metadata has no length",
                            );
                        }
                    }
                    EntryKind::DirectoryLink | EntryKind::Symlink => self.add(
                        &mut frame.size.skipped_links,
                        1,
                        &mut frame.size.errors,
                        "count skipped archive links",
                    ),
                    EntryKind::Other => {}
                }
            }
            self.path.pop();
        }
        frame.children = directories.into_iter();
        (frame, true)
    }
    /// Flush each unfinished ancestor once, merging the partial child exactly as
    /// on normal completion. A cancelled attempt never creates a diagnostic.
    fn stop(
        &mut self,
        mut stack: Vec<Frame>,
        emit: &mut dyn FnMut(VirtualEntryId, FolderSize) -> bool,
    ) {
        while let Some(mut frame) = stack.pop() {
            frame.report.finish(self.cancel.is_cancelled());
            let _ = emit(frame.id, frame.publication());
            self.path.pop();
            if let Some(parent) = stack.last_mut() {
                self.merge(&mut parent.size, &frame.size);
            }
        }
    }
}

pub(crate) struct ScanScope<'a> {
    pub address: &'a ArchiveAddress,
    pub ancestors: &'a [u64],
    pub reserved: &'a [u64],
}

pub(crate) fn compute_checked(
    mount: &dyn ArchiveMount,
    scope: ScanScope<'_>,
    roots: &[ArchiveNode],
    cancel: &Cancellation,
    emit: &mut dyn FnMut(VirtualEntryId, FolderSize) -> bool,
    reports: &crate::ReportArena,
) -> Result<bool, FsError> {
    let ScanScope {
        address,
        ancestors,
        reserved,
    } = scope;
    let Some(parent) = ancestors.last().copied() else {
        return Ok(false);
    };
    if address.validate().is_err()
        || address.components.len() + 1 != ancestors.len()
        || cancel.is_cancelled()
        || mount.id() == 0
        || parent == 0
        || roots.len() > MAX_NODES
    {
        return Ok(false);
    }
    let mut seen: HashSet<u64> = ancestors.iter().copied().collect();
    if ancestors.len() > 257
        || ancestors.first() != Some(&1)
        || seen.len() != ancestors.len()
        || reserved.len() > MAX_NODES
    {
        return Ok(false);
    }
    seen.extend(reserved.iter().copied());
    let mut selected = HashSet::new();
    for root in roots {
        if root.kind != EntryKind::Directory
            || root.parent != parent
            || root.id.mount != mount.id()
            || root.id.entry <= 1
            || ancestors.contains(&root.id.entry)
            || !selected.insert(root.id.entry)
        {
            return Ok(false);
        }
        seen.insert(root.id.entry);
    }
    let mut scan = Scan {
        mount,
        address,
        cancel,
        log: ReportLog::for_scan(reports),
        seen,
        names: 0,
        path: Vec::new(),
    };
    for (root_index, root) in roots.iter().enumerate() {
        if !emit(root.id, FolderSize::default()) {
            return Ok(false);
        }
        scan.path.push(root.raw_name.clone());
        let (initial, proceed) = scan.frame(root.id);
        let mut stack = vec![initial];
        if !proceed {
            scan.stop(stack, emit);
            return Ok(false);
        }
        while let Some(current) = stack.last_mut() {
            if cancel.is_cancelled() {
                scan.stop(stack, emit);
                return Ok(false);
            }
            if let Some(child) = current.children.next() {
                if !emit(current.id, current.publication())
                    || !emit(child.id, FolderSize::default())
                {
                    scan.stop(stack, emit);
                    return Ok(false);
                }
                scan.path.push(child.raw_name);
                let (frame, proceed) = scan.frame(child.id);
                stack.push(frame);
                if !proceed {
                    scan.stop(stack, emit);
                    return Ok(false);
                }
            } else {
                if !scan.validate(&mut current.size) {
                    scan.stop(stack, emit);
                    return Ok(false);
                }
                let mut completed = stack.pop().expect("current DFS frame");
                if stack.is_empty() && root_index + 1 == roots.len() {
                    // Keep the last root provisional until the final revision
                    // fence. Its open range can retain that exact failure without
                    // changing any previously emitted immutable report handle.
                    if !emit(completed.id, completed.publication()) || cancel.is_cancelled() {
                        scan.stop(vec![completed], emit);
                        return Ok(false);
                    }
                    if let Err(error) = mount.validate() {
                        let actual_error = scan.failure(&mut completed.size, &error);
                        completed.report.finish(cancel.is_cancelled());
                        let _ = emit(completed.id, completed.publication());
                        return if actual_error { Err(error) } else { Ok(false) };
                    }
                    if cancel.is_cancelled() {
                        scan.stop(vec![completed], emit);
                        return Ok(false);
                    }
                }
                completed.size.finished = true;
                completed.report.finish(false);
                let published = emit(completed.id, completed.publication());
                scan.path.pop();
                if let Some(parent) = stack.last_mut() {
                    scan.merge(&mut parent.size, &completed.size);
                }
                if !published {
                    scan.stop(stack, emit);
                    return Ok(false);
                }
            }
        }
    }
    Ok(!cancel.is_cancelled())
}

#[cfg(test)]
pub(crate) fn compute(
    mount: &dyn ArchiveMount,
    address: &ArchiveAddress,
    ancestors: &[u64],
    reserved: &[u64],
    roots: &[ArchiveNode],
    cancel: &Cancellation,
    emit: &mut dyn FnMut(VirtualEntryId, FolderSize) -> bool,
) -> bool {
    compute_checked(
        mount,
        ScanScope {
            address,
            ancestors,
            reserved,
        },
        roots,
        cancel,
        emit,
        &crate::ReportArena::testing(),
    )
    .unwrap_or(false)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod warnings_tests;
