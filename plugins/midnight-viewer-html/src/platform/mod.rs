#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::*;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::*;

pub(crate) fn key_code(key: &midnight_web_viewer_api::Key) -> i32 {
    match key.key.as_str() {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Escape" => 27,
        " " | "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        _ => {
            if let Some(index) = key
                .key
                .strip_prefix('F')
                .and_then(|s| s.parse::<i32>().ok())
                .filter(|i| (1..=24).contains(i))
            {
                return 111 + index;
            }
            if let Some(letter) = key.code.strip_prefix("Key").filter(|s| s.len() == 1) {
                return letter.as_bytes()[0] as i32;
            }
            if let Some(digit) = key.code.strip_prefix("Digit").filter(|s| s.len() == 1) {
                return digit.as_bytes()[0] as i32;
            }
            key.key
                .chars()
                .next()
                .filter(|c| c.is_ascii())
                .map(|c| c.to_ascii_uppercase() as i32)
                .unwrap_or(0)
        }
    }
}
