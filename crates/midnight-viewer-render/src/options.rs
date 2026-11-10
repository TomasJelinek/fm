/// Logical font settings; layout and raster coordinates are physical pixels.
pub struct TextOptions<'a> {
    pub family: &'a str,
    /// Treat installed family names literally, without interpreting CSS punctuation.
    pub literal_family: bool,
    pub size: f32,
    pub scale: f32,
    pub weight: u16,
    pub italic: bool,
    /// Equal-width digits for numeric tables, using the font's OpenType feature.
    pub tabular_numbers: bool,
    pub tab_width: u8,
    pub foreground: [u8; 4],
    /// Optional physical width for word wrapping. Source text remains unchanged.
    pub wrap_width: Option<f32>,
    /// Optional line height as a multiple of the selected font size.
    pub line_height: Option<f32>,
}

impl Default for TextOptions<'_> {
    fn default() -> Self {
        Self {
            family: "monospace",
            literal_family: false,
            size: 18.0,
            scale: 1.0,
            weight: 400,
            italic: false,
            tabular_numbers: false,
            tab_width: 4,
            foreground: [220, 220, 220, 255],
            wrap_width: None,
            line_height: None,
        }
    }
}
