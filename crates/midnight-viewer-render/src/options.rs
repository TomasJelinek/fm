/// Logical font settings; layout and raster coordinates are physical pixels.
pub struct TextOptions<'a> {
    pub family: &'a str,
    pub size: f32,
    pub scale: f32,
    pub weight: u16,
    pub italic: bool,
    pub tab_width: u8,
    pub foreground: [u8; 4],
}

impl Default for TextOptions<'_> {
    fn default() -> Self {
        Self {
            family: "monospace",
            size: 18.0,
            scale: 1.0,
            weight: 400,
            italic: false,
            tab_width: 4,
            foreground: [220, 220, 220, 255],
        }
    }
}
