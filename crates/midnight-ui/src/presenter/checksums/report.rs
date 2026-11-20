//! What a checksum window says beyond its result rows: the issue line, the save line
//! and the details behind both. Paths keep their own lines (see `append_path`).
use super::*;
use crate::error_presentation::{append_path, describe};

impl ChecksumSession {
    /// The attention line: the window's error, else the first backend diagnostic of the
    /// finished run (a refused manifest, an unreadable source), else the limits it reached.
    pub(crate) fn issue(&self) -> Option<String> {
        if let Some(line) = self.error.lines().find(|line| !line.trim().is_empty()) {
            return Some(line.to_owned());
        }
        let summary = &self.run.as_ref()?.summary;
        if let Some(first) = summary.diagnostics.first() {
            let line = first.message.lines().next().unwrap_or_default();
            return Some(match summary.diagnostics.len() - 1 {
                0 => line.to_owned(),
                more => format!("{line} (and {more} more)"),
            });
        }
        (!summary.reached_budgets.is_empty()).then(|| {
            format!(
                "Stopped at the limit for {}",
                limits(&summary.reached_budgets)
            )
        })
    }

    /// The save line: what a running save does, or where the last one went.
    pub(crate) fn save_status(&self, locale: &NumberLocale) -> String {
        if self.save.is_some() {
            return self.save_message.clone();
        }
        let Some(saved) = &self.saved else {
            return String::new();
        };
        let name = saved
            .path
            .file_name()
            .unwrap_or(saved.path.as_os_str())
            .to_string_lossy();
        let mut line = format!(
            "Saved {name} · {} included",
            locale.format_u64(saved.included)
        );
        if saved.excluded > 0 {
            line.push_str(&format!(
                " · {} excluded",
                locale.format_u64(saved.excluded)
            ));
        }
        if !saved.recovery.is_empty() {
            line.push_str(" · the replaced file is kept for recovery");
        }
        line
    }

    /// Everything the two lines summarize, for the Details view and Copy details.
    pub(crate) fn details(&self, locale: &NumberLocale) -> String {
        fn section(text: &mut String, heading: &str) {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(heading);
        }
        let mut text = String::new();
        if !self.error.is_empty() {
            section(&mut text, &self.error);
        }
        if let Some(saved) = &self.saved {
            section(
                &mut text,
                &format!(
                    "Saved and verified {} bytes\n",
                    locale.format_u64(saved.verified_bytes)
                ),
            );
            append_path(&mut text, &saved.path);
            text.push_str(&format!(
                "\n{} included · {} excluded",
                locale.format_u64(saved.included),
                locale.format_u64(saved.excluded)
            ));
            if !saved.recovery.is_empty() {
                text.push_str("\n\nThe replaced file is kept for recovery at");
                for path in &saved.recovery {
                    text.push('\n');
                    append_path(&mut text, path);
                }
                text.push_str("\nDelete it when you no longer need it.");
            }
        }
        if let Some(manifest) = &self.verification {
            section(&mut text, "Checksum file\n");
            append_path(&mut text, manifest);
        }
        section(
            &mut text,
            &format!(
                "{} found · {} skipped · {} errors",
                locale.format_u64(self.progress.discovered),
                locale.format_u64(self.progress.skipped),
                locale.format_u64(self.progress.errors)
            ),
        );
        if let Some(summary) = self.run.as_ref().map(|run| &run.summary) {
            if !summary.reached_budgets.is_empty() {
                section(
                    &mut text,
                    &format!(
                        "Reached resource limits: {}",
                        limits(&summary.reached_budgets)
                    ),
                );
            }
            for diagnostic in &summary.diagnostics {
                section(&mut text, &describe(diagnostic));
            }
        }
        text
    }
}

fn limits(budgets: &[ChecksumBudget]) -> String {
    budgets
        .iter()
        .map(|budget| match budget {
            ChecksumBudget::Rows => "result rows",
            ChecksumBudget::Depth => "folder depth",
            ChecksumBudget::Nodes => "visited entries",
            ChecksumBudget::RetainedBytes => "retained memory",
        })
        .collect::<Vec<_>>()
        .join(", ")
}
