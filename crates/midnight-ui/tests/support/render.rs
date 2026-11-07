//! Optional BMP artifacts shared by software-rendered UI checks.
use crate::ui;
#[path = "bitmap.rs"]
mod bitmap;

pub fn snapshot(test: &ui::TestUi, name: &str) {
    bitmap::snapshot(&test.pixels, test.width, test.height, name);
}
