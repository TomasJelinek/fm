use midnight_core::Cancellation;

#[test]
fn linked_lifetime_observes_both_ancestors_without_cancelling_siblings() {
    let first = Cancellation::default();
    let second = Cancellation::default();
    let first_child = first.child();
    let second_child = second.child();
    let linked = first_child.linked(&second_child);
    let sibling = first_child.linked(&second_child);
    linked.cancel();
    assert!(!first.is_cancelled());
    assert!(!second.is_cancelled());
    assert!(!sibling.is_cancelled());
    second.cancel();
    assert!(sibling.is_cancelled());

    let third = Cancellation::default();
    let linked = first_child.linked(&third);
    first.cancel();
    assert!(linked.is_cancelled());
    assert!(!third.is_cancelled());
}
