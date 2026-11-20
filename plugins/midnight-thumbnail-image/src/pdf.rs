//! First-page PDF previews, drawn the way the PDF viewer draws a page.
use hayro::{
    RenderCache, RenderSettings,
    hayro_interpret::{InterpreterSettings, util::TransformExt},
    hayro_syntax::{LoadPdfError, Pdf},
    kurbo::{Affine, Rect},
    vello_cpu::{
        Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit,
        color::palette::css::WHITE,
    },
};
use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Readers accept a PDF header after up to this many bytes of other data.
pub(crate) const SNIFF_BYTES: usize = 1024;
/// Largest encoded document a preview reads. It is read whole, once.
pub const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// Page edges outside this range in points are rejected, like the viewer's page bound.
const PAGE_EDGES: std::ops::RangeInclusive<f32> = 1.0..=32768.0;

/// Whether the content is a PDF document: `%PDF-` within its first kilobyte, in
/// content that no image signature or SVG markup start gives to the image decoder
/// (image metadata, SVG text and comments can mention a PDF header).
pub(crate) fn sniff(prefix: &[u8]) -> bool {
    !crate::signature::image(prefix)
        && prefix[..prefix.len().min(SNIFF_BYTES)]
            .windows(5)
            .any(|window| window == b"%PDF-")
}

fn error(kind: io::ErrorKind, message: &'static str) -> io::Error {
    io::Error::new(kind, message)
}

/// Draw page 1 only, fitted inside `width` by `height` with its aspect ratio, as an
/// opaque white page. Password-protected, damaged and unsupported documents fail;
/// the host then keeps the placeholder.
pub(crate) fn first_page(
    source: &mut (impl Read + Seek + ?Sized),
    width: u32,
    height: u32,
) -> io::Result<(u32, u32, Vec<u8>)> {
    let length = source.seek(SeekFrom::End(0))?;
    if length == 0 || length > MAX_SOURCE_BYTES {
        return Err(error(
            io::ErrorKind::InvalidData,
            "PDF source exceeds the preview budget or is empty",
        ));
    }
    source.seek(SeekFrom::Start(0))?;
    let length = usize::try_from(length)
        .map_err(|_| error(io::ErrorKind::InvalidData, "PDF source exceeds memory"))?;
    let mut bytes = vec![0; length];
    source.read_exact(&mut bytes)?;
    // The PDF viewer's words for the same refusals.
    let pdf = Pdf::new(bytes).map_err(|failure| match failure {
        LoadPdfError::Decryption(_) => error(
            io::ErrorKind::Unsupported,
            "The document needs a password or uses unsupported encryption",
        ),
        LoadPdfError::Invalid => error(
            io::ErrorKind::InvalidData,
            "The PDF structure is invalid or incomplete",
        ),
    })?;
    let page = pdf
        .pages()
        .first()
        .ok_or_else(|| error(io::ErrorKind::InvalidData, "PDF has no pages"))?;
    let (page_width, page_height) = page.render_dimensions();
    if !PAGE_EDGES.contains(&page_width) || !PAGE_EDGES.contains(&page_height) {
        return Err(error(
            io::ErrorKind::InvalidData,
            "Invalid PDF page dimensions",
        ));
    }
    let (page_width, page_height) = (f64::from(page_width), f64::from(page_height));
    let scale = (f64::from(width) / page_width).min(f64::from(height) / page_height);
    // At most 512 by the protocol, so both fit the renderer's u16 surfaces.
    let output_width = ((page_width * scale).round() as u32).clamp(1, width);
    let output_height = ((page_height * scale).round() as u32).clamp(1, height);
    // Map the page exactly onto the output so the paper covers every pixel.
    let mapping = Affine::scale_non_uniform(
        f64::from(output_width) / page_width,
        f64::from(output_height) / page_height,
    );
    let mut context = RenderContext::new(output_width as u16, output_height as u16);
    context.set_transform(mapping);
    context.set_paint(WHITE);
    context.fill_rect(&Rect::new(0.0, 0.0, page_width, page_height));
    let warning = Arc::new(AtomicBool::new(false));
    let sink = warning.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |_| {
            sink.store(true, Ordering::Relaxed);
        }),
        ..Default::default()
    };
    // The cache lives for this one render; the worker's memory, CPU and wall-clock
    // limits bound the rest.
    hayro::render_into(
        page,
        &RenderCache::new(),
        &settings,
        &RenderSettings::default(),
        &mut context,
        mapping * page.initial_transform(true).to_kurbo(),
    );
    // The viewer refuses such a page too; a preview must not differ from it.
    if warning.load(Ordering::Relaxed) {
        return Err(error(
            io::ErrorKind::Unsupported,
            "PDF page contains unsupported content",
        ));
    }
    context.flush();
    let mut pixmap = Pixmap::new(output_width as u16, output_height as u16);
    context.render_with(
        &mut pixmap,
        &mut Resources::default(),
        RasterizerSettings {
            target_init: TargetInit::Clear(WHITE),
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
    Ok((output_width, output_height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pdf_header_counts_only_within_the_first_kilobyte() {
        assert!(sniff(b"%PDF-1.7\n"));
        let mut shifted = vec![0; SNIFF_BYTES - 5];
        shifted.extend_from_slice(b"%PDF-");
        assert!(sniff(&shifted));
        shifted.insert(0, 0);
        assert!(!sniff(&shifted));
        for other in [&b""[..], b"%PDF", b"%pdf-1.7", b"\x89PNG\r\n\x1a\n"] {
            assert!(!sniff(other), "{other:?}");
        }
    }

    /// Leading bytes that are no image signature and no markup still lead to a PDF
    /// header; an image signature or markup leaves the content to the image decoder.
    #[test]
    fn a_pdf_header_after_an_image_signature_or_markup_is_not_a_pdf() {
        for lead in [
            &b"\xef\xbb\xbf"[..],
            b" \r\n\t",
            b"\0\0\0\0",
            b"#!junk\n",
            b"Content-Type: application/pdf\r\n\r\n",
        ] {
            assert!(sniff(&[lead, b"%PDF-1.7\n"].concat()), "{lead:?}");
        }
        for lead in [
            &b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"[..],
            b"\xff\xd8\xff\xfe\0\x10",
            b"GIF89a",
            b"RIFF\x24\0\0\0WEBPVP8L",
            b"II*\0",
            b"BM",
            b"\0\0\x01\0",
            b"\0\0\0\x1cftypavif",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><!-- ",
            b"\xef\xbb\xbf\n <?xml version=\"1.0\"?>",
        ] {
            assert!(!sniff(&[lead, b"%PDF-1.7\n"].concat()), "{lead:?}");
        }
    }
}
