//! Bounded PDF parsing and page rasterization, exclusively inside the restricted worker.
#![forbid(unsafe_code)]

use hayro::{
    RenderCache, RenderSettings,
    hayro_interpret::{InterpreterSettings, util::TransformExt},
    hayro_syntax::{LoadPdfError, Pdf},
    kurbo::{Affine, Rect},
    vello_cpu::{
        Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit,
        color::palette::css::{TRANSPARENT, WHITE},
    },
};
use midnight_image_viewer_sdk::{Document, ReadSeek, api};
use std::{
    cell::RefCell,
    io::{self, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Maximum encoded source retained by this provider, independent of viewport/output limits.
pub const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;

/// Pages whose Hayro caches stay retained between viewport requests.
const RETAINED_PAGE_CACHES: usize = 2;

/// A parsed document retained only in the restricted provider process.
pub struct PdfDocument {
    /// The worker serves exactly one document for its whole life, so the parsed document
    /// lives until the process exits; its caches can then borrow it for every request.
    pdf: &'static Pdf,
    metadata: api::DocumentMetadata,
    /// Hayro's caches (parsed fonts, glyph outlines and bitmaps, decoded images) for the
    /// pages rendered most recently, keyed by page and scale. The host requests a page in
    /// bands of tiles, so the bands after the first one reuse them. At most
    /// `RETAINED_PAGE_CACHES` pages are kept: retained memory stays a few pages' worth
    /// however far the user travels through the document.
    caches: RefCell<Vec<((u32, u64), RenderCache<'static>)>>,
}

/// Read the captured opaque source once and reject unsupported geometry before rendering.
/// PDFs needing a password fail; no filesystem/font/network lookup callbacks are installed.
/// Error texts are short and path-free: the host shows them as the failure's reason.
pub fn decode(stream: &mut dyn ReadSeek) -> io::Result<PdfDocument> {
    let length = stream.seek(SeekFrom::End(0))?;
    if length == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The file is empty",
        ));
    }
    if length > MAX_SOURCE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "The document is larger than the 128 MiB this viewer reads",
        ));
    }
    stream.seek(SeekFrom::Start(0))?;
    let mut bytes = vec![0; length as usize];
    stream.read_exact(&mut bytes)?;
    let pdf = Pdf::new(bytes).map_err(|error| match error {
        LoadPdfError::Decryption(_) => io::Error::new(
            io::ErrorKind::Unsupported,
            "The document needs a password or uses unsupported encryption",
        ),
        LoadPdfError::Invalid => io::Error::new(
            io::ErrorKind::InvalidData,
            "The PDF structure is invalid or incomplete",
        ),
    })?;
    if pdf.pages().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The document has no pages",
        ));
    }
    if pdf.pages().len() > api::MAX_DOCUMENT_PAGES {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "The document has more than 4096 pages",
        ));
    }
    let mut pages = Vec::with_capacity(pdf.pages().len());
    for page in pdf.pages().iter() {
        let (width, height) = page.render_dimensions();
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "A page has invalid dimensions",
            ));
        }
        pages.push(
            api::Metadata {
                width: width.ceil() as u32,
                height: height.ceil() as u32,
            }
            .validate()
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "A page is larger than this viewer supports",
                )
            })?,
        );
    }
    let metadata = api::DocumentMetadata { pages };
    metadata.validate()?;
    Ok(PdfDocument {
        pdf: Box::leak(Box::new(pdf)),
        metadata,
        caches: RefCell::new(Vec::new()),
    })
}

impl PdfDocument {
    /// Make the retained caches of `page` at `scale` the most recently used entry, replacing
    /// the least recently used one when they are not retained yet.
    fn touch_cache(caches: &mut Vec<((u32, u64), RenderCache<'static>)>, page: u32, scale: f64) {
        let key = (page, scale.to_bits());
        if let Some(index) = caches.iter().position(|(cached, _)| *cached == key) {
            let entry = caches.remove(index);
            caches.push(entry);
            return;
        }
        if caches.len() >= RETAINED_PAGE_CACHES {
            caches.remove(0);
        }
        caches.push((key, RenderCache::new()));
    }

    /// Pages and scales whose caches are retained, least recently used first.
    pub fn retained_caches(&self) -> Vec<(u32, f64)> {
        self.caches
            .borrow()
            .iter()
            .map(|((page, scale), _)| (*page, f64::from_bits(*scale)))
            .collect()
    }
}
impl Document for PdfDocument {
    fn metadata(&self) -> api::DocumentMetadata {
        self.metadata.clone()
    }
    fn render(&self, page: u32, viewport: api::Viewport) -> io::Result<api::Frame> {
        viewport.validate()?;
        let pdf_page = self.pdf.pages().get(page as usize).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "PDF page outside document")
        })?;
        let mut context = RenderContext::new(viewport.width as u16, viewport.height as u16);
        let mapping = Affine::scale(viewport.scale)
            * Affine::translate((-viewport.origin_x, -viewport.origin_y));
        let (width, height) = pdf_page.render_dimensions();
        context.set_transform(mapping);
        context.set_paint(WHITE);
        context.fill_rect(&Rect::new(0.0, 0.0, f64::from(width), f64::from(height)));
        let warning = Arc::new(AtomicBool::new(false));
        let sink = warning.clone();
        let settings = InterpreterSettings {
            warning_sink: Arc::new(move |_| {
                sink.store(true, Ordering::Relaxed);
            }),
            ..Default::default()
        };
        // Glyphs and paths are antialiased (`aliasing_threshold: None`) and horizontal text
        // is hinted at the exact physical size the host requests; the host never scales
        // these pixels up at the zoom it shows them. The surrounding worker additionally
        // enforces memory, CPU and operation deadlines.
        let render_settings = RenderSettings {
            aliasing_threshold: None,
            ..RenderSettings::default()
        };
        let mut caches = self.caches.borrow_mut();
        Self::touch_cache(&mut caches, page, viewport.scale);
        let cache = &caches.last().expect("a retained page cache").1;
        hayro::render_into(
            pdf_page,
            cache,
            &settings,
            &render_settings,
            &mut context,
            mapping * pdf_page.initial_transform(true).to_kurbo(),
        );
        drop(caches);
        if warning.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "The page contains content this viewer cannot draw",
            ));
        }
        context.flush();
        let mut pixmap = Pixmap::new(viewport.width as u16, viewport.height as u16);
        context.render_with(
            &mut pixmap,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(TRANSPARENT),
                ..Default::default()
            },
        );
        let mut rgba = pixmap.data_as_u8_slice().to_vec();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let alpha = u32::from(pixel[3]);
            if alpha > 0 && alpha < 255 {
                for component in &mut pixel[..3] {
                    *component = ((u32::from(*component) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
        Ok(api::Frame {
            width: viewport.width,
            height: viewport.height,
            rgba,
        })
    }
}
