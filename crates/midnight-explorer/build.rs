#[path = "build/windows.rs"]
mod windows;

fn main() {
    println!("cargo:rerun-if-changed=build/windows.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows::configure();
    }
}
