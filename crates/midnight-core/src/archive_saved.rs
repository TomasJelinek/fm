//! Saved archive locations name archives inside archives by their members' raw names, so
//! that a folder several archives deep opens again after its sessions closed: Back and
//! Forward, Hot Paths and saved tabs. Display text is never parsed back into one.
use crate::{ArchiveAddress, ArchiveFormat, ArchiveLocation, NestedArchiveAddress};
use std::path::Path;

/// One step of a saved address below the root of the archive opened from its native
/// container: a folder, or a member opened as an archive inside the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveStep<'a> {
    Folder(&'a [u8]),
    Archive(&'a [u8], ArchiveFormat),
}

impl ArchiveAddress {
    /// The steps from the outermost archive's root to this location, in order.
    pub fn steps(&self) -> Vec<ArchiveStep<'_>> {
        let mut steps: Vec<_> = self
            .components
            .iter()
            .map(|name| ArchiveStep::Folder(name))
            .collect();
        for level in &self.nested {
            steps.push(ArchiveStep::Archive(&level.member, level.format));
            steps.extend(
                level
                    .components
                    .iter()
                    .map(|name| ArchiveStep::Folder(name)),
            );
        }
        steps
    }

    /// How many steps lead from the outermost archive's root to this location.
    pub fn step_count(&self) -> usize {
        self.components.len()
            + self
                .nested
                .iter()
                .map(|level| 1 + level.components.len())
                .sum::<usize>()
    }

    /// Keep the first `count` steps: the address of an enclosing folder, or of the root
    /// of an archive inside the archive when the last kept step opens one.
    pub fn truncate_steps(&mut self, count: usize) {
        if count <= self.components.len() {
            self.components.truncate(count);
            self.nested.clear();
            return;
        }
        let mut remaining = count - self.components.len();
        let mut kept = 0;
        for level in &mut self.nested {
            if remaining == 0 {
                break;
            }
            remaining -= 1;
            kept += 1;
            let folders = remaining.min(level.components.len());
            level.components.truncate(folders);
            remaining -= folders;
        }
        self.nested.truncate(kept);
    }

    /// Whether both addresses read the same archive file with the same decoder. A newer
    /// revision of the file is the same archive.
    pub fn same_archive(&self, other: &Self) -> bool {
        self.provider == other.provider
            && self.container == other.container
            && self
                .format
                .or_else(|| crate::archive_format(&self.container))
                == other
                    .format
                    .or_else(|| crate::archive_format(&other.container))
    }

    /// Lexical location equality without filesystem access: the same archive and the
    /// same folders and archives inside it, whatever its revision.
    pub fn same_location(&self, other: &Self) -> bool {
        self.same_archive(other) && self.steps() == other.steps()
    }
}

impl ArchiveLocation {
    /// The saved form of this location: the address of the archive opened from a native
    /// file, with the members that open each archive inside it and the folders inside
    /// them (`ArchiveAddress::nested`). Saved tabs, Hot Paths and history keep it, and
    /// listing it without a session opens every level again. A level whose member name
    /// or decoder is unknown is not saved: the address then names the folder that holds
    /// that member. A location without a session is already saved.
    pub fn saved_address(&self) -> ArchiveAddress {
        // Innermost first.
        let enclosing = self.enclosing();
        let Some(outermost) = enclosing.last() else {
            return self.address.clone();
        };
        let mut saved = outermost.location.address.clone();
        saved.nested.clear();
        // From the outermost archive inwards: the member that each enclosing folder
        // opens, and the folders of the archive it opens.
        let opened = enclosing
            .iter()
            .rev()
            .skip(1)
            .map(|parent| &parent.location)
            .chain(std::iter::once(self));
        for (parent, level) in enclosing.iter().rev().zip(opened) {
            let format = level
                .address
                .format
                .or_else(|| crate::archive_format(Path::new(&parent.name)));
            let Some(format) = format.filter(|_| !parent.raw_name.is_empty()) else {
                break;
            };
            saved.nested.push(NestedArchiveAddress {
                member: parent.raw_name.clone(),
                format,
                components: level.address.components.clone(),
            });
        }
        if saved.validate().is_err() {
            saved.nested.clear();
        }
        saved
    }
}

#[cfg(test)]
#[path = "archive_saved_tests.rs"]
mod tests;
