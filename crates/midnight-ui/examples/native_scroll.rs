//! Native wheel-to-render probe. Run explicitly on a desktop.
#[cfg(windows)]
#[path = "native_scroll/windows.rs"]
mod desktop;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    desktop::run()?;
    Ok(())
}
