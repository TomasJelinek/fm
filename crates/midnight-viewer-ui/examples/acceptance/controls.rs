//! Native widgets must change the rendered content, not just stored properties.
use super::*;

#[derive(Default)]
pub struct Controls {
    source_stage: usize,
    markdown_stage: usize,
    previous: Vec<u8>,
    text: String,
    source_family: String,
    markdown_family: String,
    markdown_scroll_stage: usize,
    markdown_initial: String,
    markdown_initial_pixels: Vec<u8>,
}

impl Controls {
    fn capture(&mut self, window: &ViewerWindow) -> Result<()> {
        self.previous = pixels(window)?.as_bytes().to_vec();
        self.text = window.get_accessible_text().to_string();
        Ok(())
    }

    fn changed(&self, window: &ViewerWindow) -> Result<bool> {
        require(
            window.get_accessible_text() == self.text,
            "Presentation must preserve source text",
        )?;
        Ok(pixels(window)?.as_bytes() != self.previous)
    }

    pub fn source(&mut self, window: &ViewerWindow) -> Result<bool> {
        match self.source_stage {
            0 => {
                require(window.get_line_numbers(), "Source numbering defaults on")?;
                require(
                    window.get_status().starts_with("Ln 1, Col 1"),
                    "Source status needs exact coordinates",
                )?;
                self.capture(window)?;
                click(window, "Source line numbers")?;
            }
            1 if !window.get_line_numbers() && self.changed(window)? => {
                self.capture(window)?;
                click(window, "Source line numbers")?;
            }
            2 if window.get_line_numbers() && self.changed(window)? => {
                self.capture(window)?;
                self.source_family = select_installed_family(window)?;
            }
            3 if window.get_viewer_font_family() == self.source_family
                && self.changed(window)? =>
            {
                self.capture(window)?;
                increment_size(window)?;
            }
            4 if window.get_viewer_font_size() != 14 && self.changed(window)? => return Ok(true),
            _ => return Ok(false),
        }
        self.source_stage += 1;
        Ok(false)
    }

    pub fn markdown(&mut self, window: &ViewerWindow) -> Result<bool> {
        // Exercise the full plugin -> semantic layout -> native presentation path
        // before changing the GitHub preset. The fixture has hundreds of later blocks.
        match self.markdown_scroll_stage {
            0 => {
                self.markdown_initial = window.get_accessible_text().to_string();
                self.markdown_initial_pixels = pixels(window)?.as_bytes().to_vec();
                require(
                    !self.markdown_initial.contains("# Heading"),
                    "First page must remain rich despite later blocks",
                )?;
                window.invoke_seek(0.12);
                self.markdown_scroll_stage = 1;
                return Ok(false);
            }
            1 if !window.get_accessible_text().starts_with("Heading") => {
                window.invoke_seek(0.0);
                self.markdown_scroll_stage = 2;
                return Ok(false);
            }
            2 if window.get_accessible_text().starts_with("Heading") => {
                require(
                    window.get_accessible_text() == self.markdown_initial,
                    "Returning to the top must restore the identical parsed page",
                )?;
                require(
                    pixels(window)?.as_bytes() == self.markdown_initial_pixels,
                    "Returning to the top must restore identical Markdown pixels",
                )?;
                self.markdown_scroll_stage = 3;
            }
            1 | 2 => return Ok(false),
            _ => {}
        }
        match self.markdown_stage {
            0 => {
                require(
                    window.get_markdown_preset() == 0,
                    "Markdown defaults to GitHub",
                )?;
                require(
                    window.get_viewer_font_family() == "system-ui"
                        && window.get_viewer_font_size() == 16,
                    "Source font changes cannot overwrite Markdown defaults",
                )?;
                self.capture(window)?;
                select_next_preset(window)?;
            }
            1 if window.get_markdown_preset() == 1 && self.changed(window)? => {
                self.capture(window)?;
                select_next_preset(window)?;
            }
            2 if window.get_markdown_preset() == 2 && self.changed(window)? => {
                self.capture(window)?;
                self.markdown_family = select_installed_family(window)?;
            }
            3 if window.get_viewer_font_family() == self.markdown_family
                && self.changed(window)? =>
            {
                self.capture(window)?;
                increment_size(window)?;
            }
            4 if window.get_viewer_font_size() != 16 && self.changed(window)? => return Ok(true),
            _ => return Ok(false),
        }
        self.markdown_stage += 1;
        eprintln!(
            "Markdown control stage {}: preset={} font={} size={} content-focused={}",
            self.markdown_stage,
            window.get_markdown_preset(),
            window.get_viewer_font_family(),
            window.get_viewer_font_size(),
            window.get_content_focused()
        );
        Ok(false)
    }
}

fn select_next_preset(window: &ViewerWindow) -> Result<()> {
    click(window, "Markdown presentation")?;
    key(window, Key::DownArrow);
    key(window, Key::Return);
    Ok(())
}

fn select_installed_family(window: &ViewerWindow) -> Result<String> {
    let families = window.get_font_families();
    let target = ["Calibri", "DejaVu Serif", "Liberation Serif"]
        .iter()
        .find_map(|preferred| families.iter().position(|name| name == *preferred))
        .unwrap_or(3);
    let name = families
        .row_data(target)
        .ok_or("Installed font catalog was not published")?;
    let current = window.get_font_family_index();
    let target = target as i32;
    click(window, "Viewer font family")?;
    for _ in current..target {
        key(window, Key::DownArrow);
    }
    for _ in target..current {
        key(window, Key::UpArrow);
    }
    key(window, Key::Return);
    Ok(name.to_string())
}

fn increment_size(window: &ViewerWindow) -> Result<()> {
    let _ = window.window().take_snapshot()?;
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    let item = find_control(root, "Viewer font size").ok_or("Missing font size widget")?;
    // Use the real SpinBox's screen-reader input action. Its whole bounding box
    // includes the editable value and two buttons, so its center is not a
    // portable increment target across native font metrics and density modes.
    item.accessible_action(&i_slint_core::accessibility::AccessibilityAction::Increment);
    Ok(())
}
