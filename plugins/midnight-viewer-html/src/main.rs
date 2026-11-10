#[cfg(target_os = "linux")]
fn main() {
    std::process::exit(midnight_viewer_html::linux_main());
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Windows HTML viewer must be launched through the official CEF bootstrap executable");
    std::process::exit(70);
}
