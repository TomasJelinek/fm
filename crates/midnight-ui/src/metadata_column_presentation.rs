//! Bounded, cached metadata samples; scrolling never traverses panel rows.
use crate::*;
use std::hash::{Hash, Hasher};

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let mut key = std::collections::hash_map::DefaultHasher::new();
    (presenter.preferences.values.size_display as u8).hash(&mut key);
    presenter.number_locale.decimal_separator.hash(&mut key);
    presenter.number_locale.group_separator.hash(&mut key);
    presenter.number_locale.grouping.hash(&mut key);
    presenter.number_locale.repeat_last_group.hash(&mut key);
    for state in &presenter.panes {
        state.generation.hash(&mut key);
        let index = state.tree_view().map_or(state.index.as_ref(), |view| {
            view.stamp.revision.0.hash(&mut key);
            view.index.directory.as_ref()
        });
        (index as *const DirectoryIndex as usize).hash(&mut key);
        if let Some(samples) = state.folder_column_samples() {
            for value in samples.values() {
                value.hash(&mut key);
            }
        }
    }
    let revision = key.finish().to_string();
    if window.get_metadata_column_revision().as_str() == revision {
        return;
    }
    window.set_metadata_column_revision(revision.into());
    for pane in [PaneId::Left, PaneId::Right] {
        let state = &presenter.panes[pane.index()];
        let index = state
            .tree_view()
            .map_or(state.index.as_ref(), |view| view.index.directory.as_ref());
        let samples = &index.column_samples;
        let display = presenter.preferences.values.size_display;
        let mut sizes = samples
            .sizes
            .values()
            .map(|value| size_presentation::file_size(value, display, &presenter.number_locale))
            .collect::<Vec<_>>();
        if samples.has_directory {
            sizes.push("<DIR>".into());
        }
        if let Some(folders) = state.folder_column_samples() {
            sizes.extend(folders.values().map(|value| {
                format!(
                    "⚠ ≈ {}",
                    size_presentation::file_size(value, display, &presenter.number_locale)
                )
            }));
        }
        let mut dates = samples.dates().collect::<Vec<_>>();
        let mut times = samples.times().collect::<Vec<_>>();
        if samples.utc_fallback {
            dates.push("2000-12-28");
            times.push("23:58:58 UTC");
        }
        let size = glyph_samples(sizes.iter().map(String::as_str));
        let date = glyph_samples(dates.into_iter());
        let time = glyph_samples(times.into_iter());
        if pane == PaneId::Left {
            window.set_left_size_column_sample(size.into());
            window.set_left_date_column_sample(date.into());
            window.set_left_time_column_sample(time.into());
        } else {
            window.set_right_size_column_sample(size.into());
            window.set_right_date_column_sample(date.into());
            window.set_right_time_column_sample(time.into());
        }
    }
}

/// Ten digit variants let Slint measure proportional numeral glyphs in the actual
/// font. The number of formats and their lengths are bounded by backend samples.
fn glyph_samples<'a>(samples: impl Iterator<Item = &'a str>) -> String {
    let mut result = String::new();
    for sample in samples {
        if !sample.chars().any(|ch| ch.is_ascii_digit()) {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(sample);
            continue;
        }
        for digit in '0'..='9' {
            if !result.is_empty() {
                result.push('\n');
            }
            result.extend(
                sample
                    .chars()
                    .map(|ch| if ch.is_ascii_digit() { digit } else { ch }),
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glyph_samples_keep_locale_separators_units_and_named_months() {
        let sample = glyph_samples(["1 023,9 MiB", "25. listopadu 2026"].into_iter());
        assert_eq!(sample.lines().count(), 20);
        assert!(sample.contains("8 888,8 MiB"));
        assert!(sample.contains("88. listopadu 8888"));
    }
}
