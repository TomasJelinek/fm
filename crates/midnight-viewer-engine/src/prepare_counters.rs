//! Counts of the text preparation work scrolling causes: layouts and rasterized pixels.
//! Test builds count per thread, so the prepared-window tests can require that scrolling
//! within prepared content prepares nothing again; other builds compile the calls away.

/// Work counted since the previous [`take`].
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Page layouts (shaping and line breaking) and the bytes they laid out.
    pub layouts: u64,
    pub layout_bytes: u64,
    /// Rasterizations of body or gutter pixels and the pixels they painted.
    pub rasters: u64,
    pub raster_pixels: u64,
}

#[cfg(test)]
thread_local! {
    static COUNTS: std::cell::Cell<Counts> = const {
        std::cell::Cell::new(Counts {
            layouts: 0,
            layout_bytes: 0,
            rasters: 0,
            raster_pixels: 0,
        })
    };
}

/// A page of `bytes` was laid out.
pub(crate) fn layout(bytes: usize) {
    #[cfg(test)]
    COUNTS.with(|counts| {
        let mut value = counts.get();
        value.layouts += 1;
        value.layout_bytes += bytes as u64;
        counts.set(value);
    });
    let _ = bytes;
}

/// `pixels` body or gutter pixels were rasterized.
pub(crate) fn raster(pixels: u64) {
    #[cfg(test)]
    COUNTS.with(|counts| {
        let mut value = counts.get();
        value.rasters += 1;
        value.raster_pixels += pixels;
        counts.set(value);
    });
    let _ = pixels;
}

/// The work counted on this thread since the previous call.
#[cfg(test)]
pub(crate) fn take() -> Counts {
    COUNTS.with(|counts| counts.replace(Counts::default()))
}
