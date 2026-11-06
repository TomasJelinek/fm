//! GetAsyncKeyState reports physical buttons, unlike logical mouse messages.
pub(super) fn primary_pressed(swapped: bool, physical_pressed: impl Fn(i32) -> bool) -> bool {
    // VK_LBUTTON=1 / VK_RBUTTON=2 are physical keys for GetAsyncKeyState.
    physical_pressed(if swapped { 2 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_start_checks_the_configured_primary_physical_button() {
        assert!(primary_pressed(false, |key| key == 1));
        assert!(!primary_pressed(false, |key| key == 2));
        assert!(primary_pressed(true, |key| key == 2));
        assert!(!primary_pressed(true, |key| key == 1));
    }
}
