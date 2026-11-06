use midnight_core::Cancellation;

#[test]
fn parent_cancellation_reaches_a_child_immediately_without_a_polling_task() {
    let parent = Cancellation::default();
    let child = parent.child();
    let grandchild = child.child();
    parent.cancel();
    assert!(child.is_cancelled());
    assert!(grandchild.is_cancelled());
}

#[test]
fn child_cancel_does_not_cancel_its_parent_or_sibling() {
    let parent = Cancellation::default();
    let child = parent.child();
    let sibling = parent.child();
    child.cancel();
    assert!(child.is_cancelled());
    assert!(!parent.is_cancelled());
    assert!(!sibling.is_cancelled());
}
