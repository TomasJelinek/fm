//! Read-only cold-start attribution. Run beside the installed plugin catalog.
//! --parallel uses the production bounded discovery workers; --render-first isolates font shaping.
// This diagnostic exercises discovery; UI preparation admission is covered by engine tests.
#[allow(dead_code)]
#[path = "../src/catalog_startup.rs"]
mod catalog_startup;
use midnight_viewer_render::{TextOptions, TextRenderer, Viewport};
use std::{error::Error, time::Instant};

fn timed<T>(pass: &str, phase: &str, run: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = run();
    println!(
        "PHASE pass={pass} name={phase} elapsed_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    value
}

fn render(pass: &str) -> Result<(), Box<dyn Error>> {
    let mut renderer = timed(pass, "renderer_construct", TextRenderer::default);
    let text = "Bounded first page: fn main() { println!(\"Hello, world!\"); }\n".repeat(120);
    let options = TextOptions {
        size: 16.0,
        wrap_width: Some(1000.0),
        ..Default::default()
    };
    for phase in ["first", "second"] {
        let page = timed(pass, &format!("{phase}_layout"), || {
            renderer.layout_with_options(&text, &options, &[])
        })
        .map_err(|error| format!("Layout failed: {error:?}"))?;
        let raster = timed(pass, &format!("{phase}_raster"), || {
            renderer.rasterize(
                &page,
                Viewport {
                    width: 1000,
                    height: 600,
                    origin: [0.0; 2],
                    background: [20, 20, 20, 255],
                    selection_color: [80, 80, 160, 255],
                },
                None,
            )
        })
        .map_err(|error| format!("Raster failed: {error:?}"))?;
        println!(
            "RASTER pass={pass} phase={phase} bytes={}",
            raster.rgba.len()
        );
    }
    Ok(())
}

fn catalogs(
    pass: &str,
    application: Option<&std::path::Path>,
    parallel: bool,
) -> Result<(), Box<dyn Error>> {
    let fonts = timed(pass, "installed_font_families", || {
        midnight_viewer_render::installed_families(|| false)
    });
    println!(
        "CATALOG pass={pass} fonts={}",
        fonts.as_ref().map_or(0, |f| f.len())
    );
    if parallel {
        return concurrent_catalogs(pass, application);
    }
    let executable = timed(pass, "installed_executable_text", || {
        application.map_or_else(midnight_plugin_host::installed_executable, |path| {
            Ok(path.to_owned())
        })
    })?;
    let directory = executable.parent().ok_or("Executable has no directory")?;
    let runtime = timed(pass, "text_catalog", || {
        midnight_plugin_host::Runtime::discover(directory)
    })?;
    println!(
        "CATALOG pass={pass} text={} diagnostics={}",
        runtime.catalog().len(),
        runtime.diagnostics().len()
    );
    let executable = timed(pass, "installed_executable_image", || {
        application.map_or_else(midnight_plugin_host::installed_executable, |path| {
            Ok(path.to_owned())
        })
    })?;
    let images = timed(pass, "image_catalog", || {
        midnight_image_viewer_host::discover(&executable)
    })?;
    println!("CATALOG pass={pass} image={}", images.len());
    let executable = timed(pass, "installed_executable_web", || {
        application.map_or_else(midnight_plugin_host::installed_executable, |path| {
            Ok(path.to_owned())
        })
    })?;
    let web = timed(pass, "web_catalog", || {
        midnight_web_viewer_host::discover_with_diagnostics(&executable)
    })?;
    println!("CATALOG pass={pass} web={}", web.providers.len());
    Ok(())
}

fn concurrent_catalogs(
    pass: &str,
    application: Option<&std::path::Path>,
) -> Result<(), Box<dyn Error>> {
    let executable = application
        .map_or_else(midnight_plugin_host::installed_executable, |path| {
            Ok(path.to_owned())
        })?;
    let directory = executable.parent().ok_or("Executable has no directory")?;
    let (native, images, web) = timed(pass, "parallel_catalogs", || {
        catalog_startup::parallel(
            || {
                timed(pass, "text_catalog", || {
                    midnight_plugin_host::Runtime::discover(directory)
                })
                .map(|catalog| catalog.catalog().len())
                .map_err(|error| error.to_string())
            },
            || {
                timed(pass, "image_catalog", || {
                    midnight_image_viewer_host::discover(&executable)
                })
                .map(|catalog| catalog.len())
                .map_err(|error| error.to_string())
            },
            || {
                timed(pass, "web_catalog", || {
                    midnight_web_viewer_host::discover_with_diagnostics(&executable)
                })
                .map(|catalog| catalog.providers.len())
                .map_err(|error| error.to_string())
            },
        )
    })?;
    println!("CATALOG pass={pass} text={native} image={images} web={web}");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut render_first = false;
    let mut parallel = false;
    let mut application = None;
    for argument in std::env::args_os().skip(1) {
        if argument == "--render-first" {
            render_first = true;
        } else if argument == "--parallel" {
            parallel = true;
        } else if application.is_none() {
            application = Some(std::path::PathBuf::from(argument).canonicalize()?);
        } else {
            return Err(
                "Usage: startup_phases [--render-first] [--parallel] [application-executable]"
                    .into(),
            );
        }
    }
    println!("STARTUP render_first={render_first} parallel={parallel}");
    for pass in ["cold", "warm"] {
        let started = Instant::now();
        if render_first {
            render(pass)?;
            catalogs(pass, application.as_deref(), parallel)?;
        } else {
            catalogs(pass, application.as_deref(), parallel)?;
            render(pass)?;
        }
        println!(
            "TOTAL pass={pass} elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
