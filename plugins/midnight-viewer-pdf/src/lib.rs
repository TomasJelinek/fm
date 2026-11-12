//! Bounded PDF parsing and page rasterization, exclusively inside the restricted worker.
#![forbid(unsafe_code)]

use hayro::{
    RenderCache, RenderSettings,
    hayro_interpret::{InterpreterSettings, util::TransformExt},
    hayro_syntax::Pdf,
    kurbo::{Affine, Rect},
    vello_cpu::{
        Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit,
        color::palette::css::{TRANSPARENT, WHITE},
    },
};
use midnight_image_viewer_sdk::{Document, ReadSeek, api};
use std::{
    io::{self, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Maximum encoded source retained by this provider, independent of viewport/output limits.
pub const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;

/// A parsed document retained only in the restricted provider process.
pub struct PdfDocument {
    pdf: Pdf,
    metadata: api::DocumentMetadata,
}

/// Read the captured opaque source once and reject unsupported geometry before rendering.
/// PDFs needing a password fail; no filesystem/font/network lookup callbacks are installed.
pub fn decode(stream: &mut dyn ReadSeek) -> io::Result<PdfDocument> {
    let length = stream.seek(SeekFrom::End(0))?;
    if length == 0 || length > MAX_SOURCE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PDF source exceeds budget or is empty",
        ));
    }
    stream.seek(SeekFrom::Start(0))?;
    let mut bytes = vec![0; length as usize];
    stream.read_exact(&mut bytes)?;
    let pdf = Pdf::new(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Invalid PDF or password required",
        )
    })?;
    if pdf.pages().is_empty() || pdf.pages().len() > api::MAX_DOCUMENT_PAGES {
        return Err(io::Error::other(
            "PDF page count exceeds budget or is empty",
        ));
    }
    let mut pages = Vec::with_capacity(pdf.pages().len());
    for page in pdf.pages().iter() {
        let (width, height) = page.render_dimensions();
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(io::Error::other("Invalid PDF page dimensions"));
        }
        pages.push(
            api::Metadata {
                width: width.ceil() as u32,
                height: height.ceil() as u32,
            }
            .validate()?,
        );
    }
    let metadata = api::DocumentMetadata { pages };
    metadata.validate()?;
    Ok(PdfDocument { pdf, metadata })
}

impl Document for PdfDocument {
    fn metadata(&self) -> api::DocumentMetadata {
        self.metadata.clone()
    }
    fn render(&self, page: u32, viewport: api::Viewport) -> io::Result<api::Frame> {
        viewport.validate()?;
        let page = self.pdf.pages().get(page as usize).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "PDF page outside document")
        })?;
        let mut context = RenderContext::new(viewport.width as u16, viewport.height as u16);
        let mapping = Affine::scale(viewport.scale)
            * Affine::translate((-viewport.origin_x, -viewport.origin_y));
        let (width, height) = page.render_dimensions();
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
        // Per-render caches prevent unbounded retained growth as the user traverses a document.
        // The surrounding worker additionally enforces memory, CPU and operation deadlines.
        hayro::render_into(
            page,
            &RenderCache::new(),
            &settings,
            &RenderSettings::default(),
            &mut context,
            mapping * page.initial_transform(true).to_kurbo(),
        );
        if warning.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "PDF page contains unsupported content",
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
