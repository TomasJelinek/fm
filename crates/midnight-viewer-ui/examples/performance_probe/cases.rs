//! Read-only real-file cases and deterministic, locally owned probe fixtures.
use std::path::{Path, PathBuf};

pub(super) fn paths(
    directory: &Path,
    extended: bool,
    real_only: bool,
    wrapped_tabs: bool,
) -> std::io::Result<Vec<PathBuf>> {
    if wrapped_tabs {
        return wrapped_tab_paths(directory);
    }
    let mut paths = if real_only {
        Vec::new()
    } else {
        ["plain.txt", "source.rs", "image.bmp"]
            .map(|name| directory.join(name))
            .to_vec()
    };
    if extended {
        let markdown = directory.join("retained-scroll.md");
        if !markdown.exists() {
            let mut text = String::new();
            for index in 0..600 {
                text.push_str(&format!("## Section {index:04}\n\nA varied paragraph {index}: **bold**, *italic*, and `code` with enough words to wrap across the viewport.\n\n```rust\nlet value_{index} = {index} + 73;\n```\n\n"));
            }
            std::fs::write(&markdown, text)?;
        }
        let binary = directory.join("retained-scroll.bin");
        if !binary.exists() {
            std::fs::write(
                &binary,
                (0..262_144)
                    .map(|value| (value % 256) as u8)
                    .collect::<Vec<_>>(),
            )?;
        }
        paths.extend([markdown, binary]);
    }
    if extended || real_only {
        for path in super::wheel::real_paths()? {
            // Never silently replace a user's case with a synthetic file.
            std::fs::metadata(&path)?;
            paths.push(path);
        }
    }
    Ok(paths)
}

fn wrapped_tab_paths(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut text = String::from("short zero\nshort one\n");
    // One logical line exceeds the bounded fragment size. Its tabs must retain
    // their original line context when a refill begins inside a wrapped row.
    for segment in 0..1024 {
        text.push_str(&format!(
            "segment{segment:04} abcdefghijklmno\tPQRSTUVWXYZ\t0123456789 "
        ));
    }
    text.push('\n');
    for row in 0..800 {
        text.push_str(&format!(
            "tail{row:04}\tshort then abcdefghijklmno\tPQRSTUVWXYZ\n"
        ));
    }
    let paths = ["wrapped-tabs.txt", "wrapped-tabs.rs"].map(|name| directory.join(name));
    for path in &paths {
        if !path.exists() {
            std::fs::write(path, &text)?;
        }
    }
    Ok(paths.to_vec())
}
