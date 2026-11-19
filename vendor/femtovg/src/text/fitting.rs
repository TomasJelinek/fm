//! Opt-in fitting of filled text to the pixel grid. This module is a local
//! change to the vendored crate, described in `vendor/README.md`; with the
//! default `GlyphFitting::Subpixel` nothing in it takes part in rendering.

use std::sync::atomic::{AtomicU8, Ordering};

/// How the glyphs of filled text are fitted to the pixel grid.
///
/// The choice is process-wide. A canvas reads it once, when it is created, and
/// keeps it: its glyph atlas never mixes bitmaps of different fittings.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum GlyphFitting {
    /// Glyphs keep their layout position to a tenth of a pixel and are fitted
    /// to the grid vertically only. This is the unchanged upstream rendering.
    #[default]
    Subpixel,
    /// Every glyph starts on a whole pixel, so a letter looks the same wherever
    /// it stands, and its coverage gets a mild contrast curve.
    Snapped,
    /// Outlines are also fitted to the grid horizontally, which puts vertical
    /// stems on whole pixel columns. Inside a word glyphs follow each other at
    /// their fitted, whole-pixel advances; the pen returns to the layout
    /// position after every blank and never strays more than a pixel from it.
    /// A word is followed from the first glyph of a glyph run call, so a caller
    /// that omits glyphs outside its clip can move the rest of that word by a
    /// pixel between two scroll positions.
    Fitted,
}

static FITTING: AtomicU8 = AtomicU8::new(GlyphFitting::Subpixel as u8);

/// Selects the fitting for every canvas created from now on. Call it before
/// the first canvas exists; canvases created earlier keep their fitting.
pub fn set_glyph_fitting(fitting: GlyphFitting) {
    FITTING.store(fitting as u8, Ordering::Relaxed);
}

/// The fitting that a canvas created now would use.
pub fn glyph_fitting() -> GlyphFitting {
    match FITTING.load(Ordering::Relaxed) {
        value if value == GlyphFitting::Snapped as u8 => GlyphFitting::Snapped,
        value if value == GlyphFitting::Fitted as u8 => GlyphFitting::Fitted,
        _ => GlyphFitting::Subpixel,
    }
}

/// The largest distance, in pixels, between a fitted pen and its layout position.
const MAX_DRIFT: f32 = 1.0;

/// Places the glyphs of one glyph run call for `GlyphFitting::Fitted`.
///
/// A fitted outline is drawn for a whole-pixel advance. Following the linear
/// layout instead would round every pen separately and open or close gaps
/// between neighbors by a pixel. The pen therefore moves by the fitted advance
/// until the next blank, another baseline, a glyph that was not fitted, or a
/// drift of more than `MAX_DRIFT` from the layout, where it returns to the
/// rounded layout position. Carets and selections, which follow the layout,
/// stay within that distance of the ink.
#[derive(Default)]
pub(crate) struct WordPen {
    /// Where the next glyph goes if it continues the word: x and baseline.
    next: Option<(f32, f32)>,
}

impl WordPen {
    /// The whole-pixel x for a glyph that layout placed at `x` on `baseline`.
    pub(crate) fn place(&mut self, x: f32, baseline: f32) -> f32 {
        match self.next.take() {
            Some((pen, same)) if same == baseline && (pen - x).abs() <= MAX_DRIFT => pen,
            _ => x.round(),
        }
    }

    /// Records the glyph drawn at `pen`. `advance` is its fitted advance, or
    /// `None` for a blank and for a glyph that was not fitted.
    pub(crate) fn advance(&mut self, pen: f32, baseline: f32, advance: Option<f32>) {
        self.next = advance.map(|advance| (pen + advance, baseline));
    }
}

/// The fitting of one glyph run: stroked text keeps the upstream placement,
/// because its outlines are never fitted.
pub(crate) fn of_run(fitting: GlyphFitting, filled: bool) -> GlyphFitting {
    if filled {
        fitting
    } else {
        GlyphFitting::Subpixel
    }
}

/// Where a glyph that layout put at `x` on `baseline` is drawn: the whole-pixel
/// x of its origin, and how many tenths of a pixel beyond it the bitmap is
/// rasterized at. `Subpixel` is the upstream arithmetic, unchanged.
pub(crate) fn place(fitting: GlyphFitting, word: &mut WordPen, x: f32, baseline: f32) -> (f32, f32) {
    match fitting {
        GlyphFitting::Subpixel => (x.trunc(), crate::geometry::quantize(x.fract(), 0.1) * 10.0),
        GlyphFitting::Snapped => (x.round(), 0.0),
        GlyphFitting::Fitted => (word.place(x, baseline), 0.0),
    }
}

/// Coverage under a mild contrast curve around one half. It turns the faint
/// fringe beside a stem towards empty and the stem towards full, the same for
/// light text on dark and dark text on light.
#[cfg(feature = "swash")]
pub(crate) fn contrast(coverage: u8) -> u8 {
    const GAIN: f32 = 1.3;
    let value = (f32::from(coverage) / 255.0 - 0.5) * GAIN + 0.5;
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(feature = "swash")]
pub(crate) use fitted::FittedRasterizer;

#[cfg(feature = "swash")]
pub(crate) mod fitted {
    use fnv::FnvHashMap;
    use skrifa::{
        instance::{LocationRef, NormalizedCoord, Size},
        outline::{
            DrawSettings, Engine, GlyphStyles, HintingInstance, HintingOptions, OutlineGlyph, OutlineGlyphCollection,
            OutlinePen, SmoothMode, Target,
        },
        FontRef, GlyphId, MetadataProvider, Tag,
    };
    use swash::zeno::{Command, Format, Mask, Origin, Placement, Point, Scratch};

    use crate::text::FontId;

    /// Which hinter fits a font's outlines horizontally.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    enum Fit {
        /// The font's own TrueType program in full hinting mode. Its stems land
        /// on whole pixel columns, but only in fonts whose program moves x.
        Program,
        /// The automatic hinter with stem adjustment in both directions, for
        /// every other font. It leaves a faint fringe that the contrast curve
        /// removes.
        Automatic,
    }

    impl Fit {
        fn options(self, styles: Option<GlyphStyles>) -> HintingOptions {
            match self {
                Self::Program => HintingOptions {
                    engine: Engine::Interpreter,
                    target: Target::Mono,
                },
                Self::Automatic => HintingOptions {
                    engine: Engine::Auto(styles),
                    target: Target::Smooth {
                        mode: SmoothMode::Normal,
                        symmetric_rendering: true,
                        preserve_linear_metrics: false,
                    },
                },
            }
        }
    }

    struct Instance {
        font: FontId,
        size_bits: u32,
        coords: Vec<NormalizedCoord>,
        fit: Fit,
        hinting: HintingInstance,
        used: u64,
    }

    /// Like swash's scaler cache: few instances, searched linearly, the least
    /// recently used one is replaced.
    const MAX_INSTANCES: usize = 8;

    /// A glyph fitted to the pixel grid in both directions.
    pub(crate) struct FittedGlyph {
        /// `placement.width * placement.height` coverage bytes, top row first.
        pub data: Vec<u8>,
        /// As swash's image placement: `left` counts columns from the pen to
        /// the bitmap's left edge, `top` rows from the baseline up to its top.
        pub placement: Placement,
        /// The fitted advance in whole pixels.
        pub advance: Option<f32>,
    }

    /// Draws outlines hinted by skrifa and rasterized by zeno, the two crates
    /// swash itself renders with. Only the hinting options differ from swash's.
    #[derive(Default)]
    pub(crate) struct FittedRasterizer {
        instances: Vec<Instance>,
        /// Fonts with color glyph tables keep the unfitted path, which knows them.
        color: FnvHashMap<FontId, bool>,
        /// The automatic hinter's glyph classification, one per font.
        styles: FnvHashMap<FontId, GlyphStyles>,
        clock: u64,
        scratch: Scratch,
        path: Vec<Command>,
        coords: Vec<NormalizedCoord>,
    }

    impl FittedRasterizer {
        /// The fitted bitmap of a glyph, or `None` where the caller's unfitted
        /// path must draw it: glyphs without an outline, fonts with color
        /// glyphs, and any font or glyph the hinter rejects.
        pub(crate) fn render(
            &mut self,
            font_id: FontId,
            data: &[u8],
            face_index: u32,
            font_size: f32,
            glyph_id: u16,
            normalized_coords: &[i16],
        ) -> Option<FittedGlyph> {
            let font = FontRef::from_index(data, face_index).ok()?;
            let has_color = *self.color.entry(font_id).or_insert_with(|| {
                [b"COLR", b"CBDT", b"sbix", b"SVG "]
                    .into_iter()
                    .any(|tag| font.table_data(Tag::new(tag)).is_some())
            });
            if has_color {
                return None;
            }
            let outlines = font.outline_glyphs();
            let glyph = outlines.get(GlyphId::from(glyph_id))?;
            self.coords.clear();
            self.coords
                .extend(normalized_coords.iter().map(|coord| NormalizedCoord::from_bits(*coord)));
            let size_bits = font_size.to_bits();
            self.clock += 1;
            let found = self.instances.iter().position(|instance| {
                instance.font == font_id && instance.size_bits == size_bits && instance.coords == self.coords
            });
            let index = match found {
                Some(index) => index,
                None => {
                    let (fit, hinting) = self.fit(font_id, &font, &outlines, font_size)?;
                    let instance = Instance {
                        font: font_id,
                        size_bits,
                        coords: self.coords.clone(),
                        fit,
                        hinting,
                        used: 0,
                    };
                    if self.instances.len() < MAX_INSTANCES {
                        self.instances.push(instance);
                        self.instances.len() - 1
                    } else {
                        let index = (0..self.instances.len()).min_by_key(|index| self.instances[*index].used)?;
                        self.instances[index] = instance;
                        index
                    }
                }
            };
            self.instances[index].used = self.clock;
            let instance = &self.instances[index];

            let advance = draw(&glyph, &instance.hinting, &mut self.path)?;
            let mut data = Vec::new();
            let placement = Mask::with_scratch(&self.path[..], &mut self.scratch)
                .format(Format::Alpha)
                .origin(Origin::BottomLeft)
                // Sizing the mask here also gives `placement.top` its meaning
                // with a bottom-left origin.
                .inspect(|format, width, height| data.resize(format.buffer_size(width, height), 0))
                .render_into(&mut data[..], None);
            if placement.width == 0 || placement.height == 0 {
                return None;
            }
            if instance.fit == Fit::Automatic {
                for coverage in &mut data {
                    *coverage = super::contrast(*coverage);
                }
            }
            Some(FittedGlyph {
                data,
                placement,
                advance,
            })
        }

        /// The hinter for a font at one size: its own program if that puts a
        /// plain stem on whole pixels, the automatic hinter otherwise.
        fn fit(
            &mut self,
            font_id: FontId,
            font: &FontRef<'_>,
            outlines: &OutlineGlyphCollection<'_>,
            font_size: f32,
        ) -> Option<(Fit, HintingInstance)> {
            let size = Size::new(font_size);
            let location = LocationRef::new(&self.coords);
            let program = HintingInstance::new(outlines, size, location, Fit::Program.options(None)).ok();
            if let Some(program) = program {
                // Letters whose left and right edges are the outer edges of
                // vertical stems in an upright Latin face. A program that fits
                // x puts all of them on whole pixels; one stem can land there
                // by chance of the size, three letters do not.
                let charmap = font.charmap();
                let mut fitted = ['l', 'n', 'H'].into_iter().map(|letter| {
                    let glyph = outlines.get(charmap.map(letter)?)?;
                    draw(&glyph, &program, &mut self.path)?;
                    let (left, right) = horizontal_bounds(&self.path)?;
                    let whole = |edge: f32| (edge - edge.round()).abs() < 0.02;
                    Some(whole(left) && whole(right) && right - left >= 0.5)
                });
                if fitted.all(|letter| letter == Some(true)) {
                    return Some((Fit::Program, program));
                }
            }
            let styles = self
                .styles
                .entry(font_id)
                .or_insert_with(|| GlyphStyles::new(outlines))
                .clone();
            let automatic =
                HintingInstance::new(outlines, size, location, Fit::Automatic.options(Some(styles))).ok()?;
            Some((Fit::Automatic, automatic))
        }
    }

    /// Draws the hinted outline into `path` and returns its fitted advance.
    /// `None` for an empty outline and when the glyph cannot be drawn.
    fn draw(glyph: &OutlineGlyph<'_>, hinting: &HintingInstance, path: &mut Vec<Command>) -> Option<Option<f32>> {
        path.clear();
        let mut pen = PathPen(path);
        // `false`: a failing glyph program yields the unhinted outline, not an error.
        let adjusted = glyph.draw(DrawSettings::hinted(hinting, false), &mut pen).ok()?;
        pen.close_open_subpath();
        (!path.is_empty()).then_some(adjusted.advance_width)
    }

    /// The smallest and largest x of a path's points.
    fn horizontal_bounds(path: &[Command]) -> Option<(f32, f32)> {
        let mut bounds: Option<(f32, f32)> = None;
        let mut include = |point: &Point| {
            let (left, right) = bounds.unwrap_or((point.x, point.x));
            bounds = Some((left.min(point.x), right.max(point.x)));
        };
        for command in path {
            match command {
                Command::MoveTo(point) | Command::LineTo(point) => include(point),
                Command::QuadTo(control, point) => {
                    include(control);
                    include(point);
                }
                Command::CurveTo(first, second, point) => {
                    include(first);
                    include(second);
                    include(point);
                }
                Command::Close => {}
            }
        }
        bounds
    }

    /// Collects skrifa's outline, in pixels with y up from the baseline, as a
    /// zeno path. Like swash's own pen it closes a subpath left open.
    struct PathPen<'a>(&'a mut Vec<Command>);

    impl PathPen<'_> {
        fn close_open_subpath(&mut self) {
            if !self.0.is_empty() && self.0.last() != Some(&Command::Close) {
                self.0.push(Command::Close);
            }
        }
    }

    impl OutlinePen for PathPen<'_> {
        fn move_to(&mut self, x: f32, y: f32) {
            self.close_open_subpath();
            self.0.push(Command::MoveTo(Point::new(x, y)));
        }

        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push(Command::LineTo(Point::new(x, y)));
        }

        fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
            self.0.push(Command::QuadTo(Point::new(cx0, cy0), Point::new(x, y)));
        }

        fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
            self.0.push(Command::CurveTo(
                Point::new(cx0, cy0),
                Point::new(cx1, cy1),
                Point::new(x, y),
            ));
        }

        fn close(&mut self) {
            self.0.push(Command::Close);
        }
    }
}

/// The vendored tree ships no font, so these read one from the machine and are
/// ignored by default. `vendor/README.md` has the command that runs them.
#[cfg(all(test, feature = "swash"))]
mod font_tests {
    use super::fitted::FittedRasterizer;
    use crate::text::FontId;
    use skrifa::MetadataProvider;

    const DEJAVU_SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

    struct Face {
        data: Vec<u8>,
        id: FontId,
        rasterizer: FittedRasterizer,
    }

    impl Face {
        fn dejavu_sans() -> Self {
            let mut keys = slotmap::SlotMap::<slotmap::DefaultKey, ()>::new();
            Self {
                data: std::fs::read(DEJAVU_SANS).expect("DejaVu Sans is installed"),
                id: FontId(keys.insert(())),
                rasterizer: FittedRasterizer::default(),
            }
        }

        /// The bitmap rows of a character and its fitted advance.
        fn render(&mut self, character: char, size: f32) -> (Vec<Vec<u8>>, f32) {
            let font = skrifa::FontRef::from_index(&self.data, 0).unwrap();
            let glyph = font.charmap().map(character).unwrap();
            let fitted = self
                .rasterizer
                .render(self.id, &self.data, 0, size, glyph.to_u32() as u16, &[])
                .unwrap_or_else(|| panic!("{character:?} at {size} px is fitted"));
            let width = fitted.placement.width as usize;
            assert_eq!(fitted.data.len(), width * fitted.placement.height as usize);
            let rows = fitted.data.chunks(width).map(<[u8]>::to_vec).collect();
            (rows, fitted.advance.expect("a fitted advance"))
        }
    }

    /// Columns that are fully covered over at least half of the glyph's height:
    /// stems, but not the arches that join them.
    fn solid_columns(rows: &[Vec<u8>]) -> Vec<usize> {
        (0..rows[0].len())
            .filter(|column| {
                let solid = rows.iter().filter(|row| row[*column] >= 250).count();
                solid * 2 >= rows.len()
            })
            .collect()
    }

    #[test]
    #[ignore = "reads DejaVu Sans from /usr/share/fonts/truetype/dejavu"]
    fn stems_of_a_font_with_a_fitting_program_are_whole_pixel_columns() {
        let mut face = Face::dejavu_sans();
        for size in [11.0, 12.0, 13.0, 14.0, 16.0] {
            let (l, advance) = face.render('l', size);
            assert_eq!(advance, advance.round(), "l at {size} px");
            let stem = solid_columns(&l);
            assert!(!stem.is_empty(), "l at {size} px has a full column");
            // Nothing is left beside the stem: no half-covered fringe column.
            for row in &l {
                for (column, coverage) in row.iter().enumerate() {
                    assert!(
                        stem.contains(&column) || *coverage == 0,
                        "l at {size} px: column {column} has coverage {coverage}"
                    );
                }
            }
            // The three stems of m, each on full columns with gaps between them.
            let (m, advance) = face.render('m', size);
            assert_eq!(advance, advance.round(), "m at {size} px");
            let stems = solid_columns(&m);
            let groups = 1 + stems.windows(2).filter(|pair| pair[1] != pair[0] + 1).count();
            let picture: Vec<String> = m
                .iter()
                .map(|row| row.iter().map(|coverage| format!("{coverage:4}")).collect())
                .collect();
            assert_eq!(
                groups,
                3,
                "m at {size} px: solid columns {stems:?}\n{}",
                picture.join("\n")
            );
        }
    }

    #[test]
    #[ignore = "reads DejaVu Sans from /usr/share/fonts/truetype/dejavu"]
    fn blanks_are_left_to_the_caller_and_bitmaps_repeat_exactly() {
        let mut face = Face::dejavu_sans();
        let font = skrifa::FontRef::from_index(&face.data, 0).unwrap();
        let space = font.charmap().map(' ').unwrap().to_u32() as u16;
        assert!(face
            .rasterizer
            .render(face.id, &face.data, 0, 12.0, space, &[])
            .is_none());
        // More sizes than cached hinting instances, then the first one again.
        let first = face.render('g', 12.0);
        for size in 13..=24 {
            face.render('g', size as f32);
        }
        assert_eq!(face.render('g', 12.0), first);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_fitting_is_the_upstream_one() {
        assert_eq!(GlyphFitting::default(), GlyphFitting::Subpixel);
    }

    #[test]
    fn a_word_keeps_its_fitted_advances_and_returns_to_the_layout_at_blanks() {
        let mut pen = WordPen::default();
        // "mi m": layout advances 11.69, 3.33 and 3.81, fitted advances 11 and 3.
        let m = pen.place(3.9, 20.0);
        assert_eq!(m, 4.0, "a word starts on the rounded layout position");
        pen.advance(m, 20.0, Some(11.0));
        let i = pen.place(15.59, 20.0);
        assert_eq!(
            i, 15.0,
            "the next glyph follows at the fitted advance, not at round(15.59)"
        );
        pen.advance(i, 20.0, Some(3.0));
        // The blank is not drawn, so the pen forgets the word.
        let blank = pen.place(18.92, 20.0);
        pen.advance(blank, 20.0, None);
        assert_eq!(pen.place(22.73, 20.0), 23.0);
    }

    #[test]
    fn the_pen_never_strays_more_than_a_pixel_from_the_layout() {
        let mut pen = WordPen::default();
        let mut layout = 0.3_f32;
        let mut placed = Vec::new();
        // Twenty glyphs whose fitted advance is 0.4 px short of the layout.
        for _ in 0..20 {
            let x = pen.place(layout, 9.5);
            assert_eq!(x, x.round());
            assert!((x - layout).abs() <= MAX_DRIFT, "{x} against {layout}");
            placed.push(x);
            pen.advance(x, 9.5, Some(6.0));
            layout += 6.4;
        }
        let gaps: Vec<f32> = placed.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(gaps.iter().all(|gap| *gap == 6.0 || *gap == 7.0), "{gaps:?}");
        assert!(gaps.contains(&7.0), "the drift is corrected inside the word");
        assert!(
            gaps.windows(2).all(|pair| pair != [7.0, 7.0]),
            "corrections are single pixels: {gaps:?}"
        );
    }

    #[test]
    fn another_baseline_or_a_pen_behind_the_layout_starts_anew() {
        let mut pen = WordPen::default();
        pen.advance(10.0, 5.0, Some(7.0));
        assert_eq!(pen.place(17.6, 6.0), 18.0, "the next line");
        pen.advance(10.0, 5.0, Some(7.0));
        assert_eq!(pen.place(3.6, 5.0), 4.0, "a run that goes backwards");
        pen.advance(10.0, 5.0, Some(7.0));
        assert_eq!(pen.place(18.01, 5.0), 18.0, "more than a pixel ahead");
        pen.advance(10.0, 5.0, Some(7.0));
        assert_eq!(pen.place(17.9, 5.0), 17.0, "within a pixel the word continues");
    }

    #[test]
    fn subpixel_placement_is_the_upstream_arithmetic() {
        let mut word = WordPen::default();
        for step in -300..=300 {
            let x = step as f32 * 0.037;
            // The two expressions this replaced in `render_atlas`.
            let upstream = (x.trunc(), crate::geometry::quantize(x.fract(), 0.1) * 10.0);
            assert_eq!(place(GlyphFitting::Subpixel, &mut word, x, 5.0), upstream, "{x}");
        }
        // The pen truncates and the bitmap carries the rest, in tenths: the
        // key of the glyph cache, where a fraction near one becomes ten.
        let tenths = |x| {
            let (pen, phase) = place(GlyphFitting::Subpixel, &mut WordPen::default(), x, 5.0);
            (pen, phase as u8)
        };
        assert_eq!(tenths(7.26), (7.0, 3));
        assert_eq!(tenths(7.96), (7.0, 10));
        // The word pen was never consulted: the next fitted glyph starts a word.
        assert_eq!(place(GlyphFitting::Fitted, &mut word, 7.6, 5.0), (8.0, 0.0));
    }

    #[test]
    fn snapped_and_fitted_glyphs_have_one_bitmap_on_a_whole_pixel() {
        let mut word = WordPen::default();
        for (x, pen) in [(7.26, 7.0), (7.5, 8.0), (7.96, 8.0), (-0.4, 0.0), (-0.6, -1.0)] {
            assert_eq!(place(GlyphFitting::Snapped, &mut word, x, 5.0), (pen, 0.0), "{x}");
        }
        assert_eq!(place(GlyphFitting::Fitted, &mut word, 3.9, 5.0), (4.0, 0.0));
        word.advance(4.0, 5.0, Some(11.0));
        assert_eq!(
            place(GlyphFitting::Fitted, &mut word, 15.59, 5.0),
            (15.0, 0.0),
            "inside a word the fitted advance decides"
        );
    }

    #[test]
    fn stroked_runs_keep_the_upstream_placement_in_every_fitting() {
        for fitting in [GlyphFitting::Subpixel, GlyphFitting::Snapped, GlyphFitting::Fitted] {
            assert_eq!(of_run(fitting, true), fitting);
            assert_eq!(of_run(fitting, false), GlyphFitting::Subpixel);
        }
    }

    #[cfg(feature = "swash")]
    #[test]
    fn contrast_is_monotonic_keeps_the_ends_and_is_symmetric() {
        assert_eq!((contrast(0), contrast(255)), (0, 255));
        assert_eq!(contrast(128), 128);
        assert!(contrast(40) < 40 && contrast(215) > 215);
        for coverage in 0..255u8 {
            assert!(contrast(coverage) <= contrast(coverage + 1));
            let mirrored = 255 - contrast(255 - coverage);
            assert!(mirrored.abs_diff(contrast(coverage)) <= 1, "{coverage}");
        }
    }
}
