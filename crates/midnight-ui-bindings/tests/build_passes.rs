//! Unit tests of the token passes that the build script applies to the generated
//! Rust. Cargo does not run tests inside a build script, so the passes are compiled
//! here a second time; they need nothing but `proc-macro2`.
#[path = "../build/arms.rs"]
mod arms;
#[path = "../build/partition.rs"]
mod partition;

#[test]
fn the_passes_combine_on_generated_shaped_code() {
    let generated: proc_macro2::TokenStream = "mod slint_generatedX { \
         struct InnerA { a: u8 } \
         impl InnerA { fn accessible_role(self: Pin<&Self>, index: u32) -> Role { \
         match index { 1u32 => Role::Button, 1u32 => self.a.accessible_role(0), _ => Role::None, } } } } \
         pub use slint_generatedX::{InnerA};"
        .parse()
        .unwrap();
    let output = partition::spread(arms::prune(generated), partition::PARTS)
        .unwrap()
        .to_string()
        .replace(char::is_whitespace, "");
    assert_eq!(output.matches("1u32").count(), 1);
    assert_eq!(output.matches("modslint_part_").count(), 1);
    assert!(output.contains("pub(super)structInnerA"));
}
