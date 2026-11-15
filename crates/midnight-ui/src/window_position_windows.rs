pub(super) fn restore(window: &slint::Window, position: slint::PhysicalPosition) -> bool {
    window.set_position(position);
    true
}
