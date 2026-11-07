//! Backend-only ordered filename matching; undecodable native names never match.
use crate::filter::CompiledFilter;
use midnight_core::{
    EntryKind, FilenameColorRule, FilenameColorTarget, FilenameStyle, validate_filename_colors,
};
use std::ffi::OsStr;

pub struct CompiledFilenameColors(Vec<(FilenameColorRule, CompiledFilter)>);

impl CompiledFilenameColors {
    pub fn new(rules: &[FilenameColorRule]) -> Result<Self, String> {
        validate_filename_colors(rules).map_err(str::to_owned)?;
        let mut compiled = Vec::with_capacity(rules.len());
        for (index, rule) in rules.iter().enumerate() {
            let filter = CompiledFilter::new(&rule.filter())
                .map_err(|error| format!("Rule {}: {error}", index + 1))?;
            compiled.push((rule.clone(), filter));
        }
        Ok(Self(compiled))
    }

    pub fn style(&self, name: &OsStr, kind: EntryKind) -> Option<FilenameStyle> {
        name.to_str()?;
        let folder = matches!(kind, EntryKind::Directory | EntryKind::DirectoryLink);
        self.0.iter().find_map(|(rule, filter)| {
            (rule.enabled
                && match rule.target {
                    FilenameColorTarget::All => true,
                    FilenameColorTarget::Files => !folder,
                    FilenameColorTarget::Folders => folder,
                }
                && filter.matches(name))
            .then_some(rule.style)
        })
    }
}
