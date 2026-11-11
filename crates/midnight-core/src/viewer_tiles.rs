//! App-owned raster tiles. These types are not part of a plugin ABI.
use std::{fmt, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewerTileId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewerSurfaceId(pub u64);

/// Backend RAII byte reservation, retained until the last pixel owner is dropped.
pub trait ViewerPixelOwner: fmt::Debug + Send + Sync {}

pub struct ViewerPixels {
    // Drop pixel memory before releasing its reservation.
    rgba: Box<[u8]>,
    known_transparent: bool,
    _owner: Arc<dyn ViewerPixelOwner>,
}

impl ViewerPixels {
    /// The backend reserves the allocation before asking a decoder for it.
    pub fn new(rgba: Vec<u8>, owner: Arc<dyn ViewerPixelOwner>) -> Result<Self, &'static str> {
        Self::validate_length(rgba.len())?;
        Ok(Self {
            rgba: rgba.into_boxed_slice(),
            known_transparent: false,
            _owner: owner,
        })
    }

    /// Allocate guaranteed zero pixels after the backend reserves their byte budget.
    pub fn transparent(
        bytes: usize,
        owner: Arc<dyn ViewerPixelOwner>,
    ) -> Result<Self, &'static str> {
        Self::validate_length(bytes)?;
        Ok(Self {
            rgba: vec![0; bytes].into_boxed_slice(),
            known_transparent: true,
            _owner: owner,
        })
    }

    /// True only for immutable pixels created by the guaranteed-zero constructor.
    /// Ordinary decoded pixels are not scanned or classified as transparent.
    pub fn is_known_transparent(&self) -> bool {
        self.known_transparent
    }

    fn validate_length(bytes: usize) -> Result<(), &'static str> {
        if bytes == 0 || bytes > 16 * 1024 * 1024 || !bytes.is_multiple_of(4) {
            return Err("Viewer tile exceeds pixel bounds.");
        }
        Ok(())
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

impl fmt::Debug for ViewerPixels {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ViewerPixels")
            .field("bytes", &self.rgba.len())
            .field("known_transparent", &self.known_transparent)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct ViewerTile {
    pub id: ViewerTileId,
    /// Raster pixels relative to the frame's visible camera, including bounded guards.
    pub position: [f32; 2],
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<ViewerPixels>,
}

impl ViewerTile {
    pub fn validate(&self) -> Result<(), &'static str> {
        let bytes = usize::try_from(self.width)
            .ok()
            .and_then(|width| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4));
        if self.id.0 == 0
            || self.width == 0
            || self.height == 0
            || self.width > 4096
            || self.height > 4096
            || self
                .position
                .iter()
                .any(|value| !value.is_finite() || value.abs() > 8192.0)
            || bytes != Some(self.pixels.rgba().len())
        {
            return Err("Invalid viewer raster tile.");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Charge(Arc<AtomicUsize>);
    impl ViewerPixelOwner for Charge {}
    impl Drop for Charge {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn known_transparent_pixels_are_zero_and_retain_their_reservation() {
        let released = Arc::new(AtomicUsize::new(0));
        let pixels =
            Arc::new(ViewerPixels::transparent(8, Arc::new(Charge(released.clone()))).unwrap());
        assert!(pixels.is_known_transparent());
        assert_eq!(pixels.rgba(), &[0; 8]);
        let retained = pixels.clone();
        drop(pixels);
        assert_eq!(released.load(Ordering::Relaxed), 0);
        drop(retained);
        assert_eq!(released.load(Ordering::Relaxed), 1);

        let decoded = ViewerPixels::new(vec![0; 8], Arc::new(Charge(released.clone()))).unwrap();
        assert!(!decoded.is_known_transparent());
        drop(decoded);
        assert_eq!(released.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn transparent_pixel_bounds_are_checked_before_allocating() {
        let released = Arc::new(AtomicUsize::new(0));
        let invalid = [0, 1, 3, 5, 16 * 1024 * 1024 + 4, usize::MAX];
        for bytes in invalid {
            assert!(ViewerPixels::transparent(bytes, Arc::new(Charge(released.clone()))).is_err());
        }
        assert_eq!(released.load(Ordering::Relaxed), invalid.len());
        let maximum =
            ViewerPixels::transparent(16 * 1024 * 1024, Arc::new(Charge(released))).unwrap();
        assert!(maximum.is_known_transparent());
        assert_eq!(maximum.rgba().len(), 16 * 1024 * 1024);
    }
    #[test]
    fn reservation_lives_through_every_retained_frame_and_pixels_are_validated() {
        let released = Arc::new(AtomicUsize::new(0));
        let pixels = Arc::new(
            ViewerPixels::new(vec![10, 20, 30, 255], Arc::new(Charge(released.clone()))).unwrap(),
        );
        let tile = ViewerTile {
            id: ViewerTileId(1),
            position: [-2.5, 3.25],
            width: 1,
            height: 1,
            pixels: pixels.clone(),
        };
        tile.validate().unwrap();
        drop(pixels);
        assert_eq!(released.load(Ordering::Relaxed), 0);
        let retained = tile.clone();
        drop(tile);
        assert_eq!(released.load(Ordering::Relaxed), 0);
        let bad = ViewerTile {
            width: 2,
            ..retained.clone()
        };
        assert!(bad.validate().is_err());
        drop(bad);
        drop(retained);
        assert_eq!(released.load(Ordering::Relaxed), 1);
    }
}
