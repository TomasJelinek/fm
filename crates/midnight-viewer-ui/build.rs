fn main() {
    slint_build::compile_with_config(
        "ui/viewer.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .unwrap();
}
