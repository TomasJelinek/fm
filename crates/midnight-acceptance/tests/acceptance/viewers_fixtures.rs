//! Content the viewer scenarios generate in their own work folders (the shared fixture
//! stays unchanged): long CSV and TSV tables with rows of 100 to 260 bytes, like the
//! inventory whose scrollbar drags once failed, long Markdown with tables, long source,
//! a multi-page PDF, a long HTML page, a sample for every extension the Source viewer
//! claims, damaged files, and archives of every supported format. Formats that need a
//! compressor this suite does not implement (xz, bzip2, Zstandard, 7-Zip, real deflate)
//! are made with the host's own programs; without one, the check that needs it is
//! skipped with that reason.
use crate::encode;
use std::{
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Words for generated prose; every generated line also carries its own number, so no
/// two lines render alike.
const WORDS: &[&str] = &[
    "midnight",
    "explorer",
    "panel",
    "folder",
    "archive",
    "viewer",
    "scroll",
    "status",
    "kinetic",
    "render",
    "theme",
    "column",
    "inventory",
    "license",
    "package",
    "source",
    "binary",
    "column",
    "window",
    "cursor",
    "select",
    "copy",
    "verify",
    "unicode",
];

const LICENSES: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "BSD-3-Clause",
    "MIT OR Apache-2.0",
    "Zlib",
    "ISC",
    "MPL-2.0",
];

/// A small deterministic generator (xorshift64*).
pub struct Random(u64);

impl Random {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }

    pub fn word(&mut self) -> &'static str {
        WORDS[self.below(WORDS.len())]
    }
}

/// Write `bytes` to `root/relative` (creating folders) and return the path.
pub fn write(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) -> Result<PathBuf, String> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

/// The byte length of row `row` of [`table`] without its line break: 100 to 260 bytes.
pub fn table_row_length(row: usize) -> usize {
    100 + row * 37 % 161
}

/// A table of `rows` data rows after a header, separated by `delimiter` (`,` or `\t`),
/// seven columns, every row 100 to 260 bytes long and starting with its number
/// (`r00042`), like `docs/third-party-inventory.csv`.
pub fn table(rows: usize, delimiter: char) -> String {
    let mut random = Random::new(0x5eed_0001);
    let mut text = format!(
        "id{d}component{d}license{d}version{d}size{d}notes{d}flag\n",
        d = delimiter
    );
    for row in 0..rows {
        let mut line = format!(
            "r{row:05}{d}{}-{}{d}{}{d}{}.{}.{}{d}{}{d}",
            random.word(),
            row % 89,
            LICENSES[row % LICENSES.len()],
            row % 4,
            row % 31,
            row % 9,
            row * 7919 % 100_000,
            d = delimiter
        );
        let flag = if row % 3 == 0 { "yes" } else { "no" };
        let target = table_row_length(row) - flag.len() - 1;
        while line.len() < target {
            let word = random.word();
            let room = target - line.len();
            if word.len() < room {
                line.push_str(word);
                line.push(' ');
            } else {
                line.push_str(&"x".repeat(room));
            }
        }
        line.push(delimiter);
        line.push_str(flag);
        text.push_str(&line);
        text.push('\n');
    }
    text
}

/// Long Markdown: `sections` sections, each a heading, a paragraph, a 20-row table with
/// left, centred and right columns, a list, a code block and a quote; every element
/// names its section (`s012`).
pub fn markdown(sections: usize) -> String {
    let mut random = Random::new(0x5eed_0002);
    let mut text = String::from("# Long document\n\nGenerated for the viewer scenarios.\n\n");
    for section in 0..sections {
        let _ = write!(
            text,
            "## Section s{section:03}\n\nParagraph s{section:03}: "
        );
        for _ in 0..40 {
            text.push_str(random.word());
            text.push(' ');
        }
        text.push_str("\n\n| Row | Name | Count |\n| --- | :---: | ---: |\n");
        for row in 0..20 {
            let _ = writeln!(
                text,
                "| s{section:03}-r{row:02} | {} {} | {} |",
                random.word(),
                random.word(),
                section * 100 + row
            );
        }
        text.push('\n');
        for item in 0..4 {
            let _ = writeln!(text, "- item s{section:03}-{item}: {}", random.word());
        }
        let _ = write!(
            text,
            "\n```rust\nfn section_{section}() -> u32 {{\n    {section}\n}}\n```\n\n> Quote s{section:03}: {}\n\n",
            random.word()
        );
    }
    text
}

/// Long Rust source: `lines` numbered lines of functions, strings, numbers and comments.
pub fn rust(lines: usize) -> String {
    let mut text = String::from("// Generated for the viewer scenarios.\n");
    let mut line = 1;
    let mut function = 0;
    while line < lines {
        let _ = writeln!(
            text,
            "/// Function {function}.\nfn function_{function}() -> u64 {{"
        );
        line += 2;
        for step in 0..12 {
            let _ = writeln!(
                text,
                "    let value_{step} = compute(\"item {line}\", {line} * {step}); // line {line}"
            );
            line += 1;
        }
        text.push_str("    0\n}\n");
        line += 2;
        function += 1;
    }
    text
}

/// A PDF of `pages` US-letter portrait pages; each says "Page N of M" in large type,
/// followed by 30 lines of text naming the page.
pub fn pdf(pages: usize) -> Vec<u8> {
    let mut output = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut object = |output: &mut Vec<u8>, body: &[u8]| {
        offsets.push(output.len());
        output.extend(format!("{} 0 obj\n", offsets.len()).as_bytes());
        output.extend(body);
        output.extend(b"\nendobj\n");
    };
    let kids: Vec<String> = (0..pages)
        .map(|page| format!("{} 0 R", 4 + 2 * page))
        .collect();
    object(&mut output, b"<< /Type /Catalog /Pages 2 0 R >>");
    object(
        &mut output,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            kids.join(" ")
        )
        .as_bytes(),
    );
    object(
        &mut output,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    );
    for page in 0..pages {
        object(
            &mut output,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << \
                 /F1 3 0 R >> >> /Contents {} 0 R >>",
                5 + 2 * page
            )
            .as_bytes(),
        );
        let mut content = format!(
            "BT /F1 40 Tf 72 700 Td (Page {} of {pages}) Tj ET\n",
            page + 1
        );
        for line in 0..30 {
            let _ = writeln!(
                content,
                "BT /F1 12 Tf 72 {} Td (Line {} of page {}: viewer acceptance text) Tj ET",
                650 - line * 18,
                line + 1,
                page + 1
            );
        }
        // `content` ends with its own line break; the one before `endstream` is the
        // stream's end of line, not counted in its length.
        object(
            &mut output,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .as_bytes(),
        );
    }
    let xref = output.len();
    output.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for offset in &offsets {
        output.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    output
}

/// A long HTML page: a heading and `paragraphs` numbered paragraphs of differing length.
pub fn html(paragraphs: usize) -> String {
    let mut random = Random::new(0x5eed_0003);
    let mut text = String::from(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>Long page</title>\
         <style>body{font:16px sans-serif;margin:16px;color:#111;background:#fff}\
         p{margin:0 0 8px}</style></head><body>\n<h1>Long page</h1>\n",
    );
    for paragraph in 0..paragraphs {
        let _ = write!(text, "<p>Paragraph {paragraph}:");
        for _ in 0..(3 + paragraph * 7 % 23) {
            text.push(' ');
            text.push_str(random.word());
        }
        text.push_str("</p>\n");
    }
    text.push_str("</body></html>\n");
    text
}

/// One sample per lexical profile of the Source viewer, and for every further
/// extension its manifest claims: (file name, content, a fragment the view shows).
pub fn source_samples() -> Vec<(String, String, &'static str)> {
    let rust =
        "// comment\nfn main() {\n    let answer: u32 = 42;\n    println!(\"{answer}\");\n}\n";
    let c = "/* comment */\n#include <stdio.h>\nint main(void) {\n    return printf(\"%d\\n\", 42);\n}\n";
    let header = "#ifndef SAMPLE_H\n#define SAMPLE_H 1\nint answer(void); /* 42 */\n#endif\n";
    let cpp = "// comment\n#include <string>\nauto main() -> int {\n    std::string text = R\"x(raw)x\";\n    return static_cast<int>(text.size()) + 42;\n}\n";
    let hpp = "#pragma once\nnamespace sample {\nconstexpr int answer = 42; // comment\n}\n";
    let js = "// comment\nexport function sum(a, b) {\n  const text = \"total\";\n  return a + b + 42;\n}\n";
    let jsx = "// comment\nexport const View = () => <div className=\"box\">{42}</div>;\n";
    let ts = "// comment\nexport interface Item { name: string; count: number }\nexport const answer: number = 42;\nconst label = \"items\";\n";
    let python = "# comment\ndef greet(name: str) -> str:\n    return f\"hello {name}\" * 42\n";
    let shell = "#!/bin/sh\n# comment\nset -eu\nanswer=42\necho \"answer: $answer\"\n";
    let batch = "@echo off\nREM comment\nset ANSWER=42\necho \"%ANSWER%\"\nif exist \"file.txt\" goto :eof\n";
    let json = "{\n  \"theme\": \"dark\",\n  \"size\": 42,\n  \"enabled\": true\n}\n";
    let toml = "# comment\n[window]\nwidth = 1340\ntitle = \"Midnight\"\n";
    let java = "// comment\npublic class Sample {\n    public static void main(String[] args) {\n        System.out.println(\"answer \" + 42);\n    }\n}\n";
    let csharp = "// comment\nusing System;\npublic static class Sample {\n    public static int Answer() => 42;\n    public static string Name = \"sample\";\n}\n";
    let go = "// comment\npackage main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"answer\", 42)\n}\n";
    let swift = "// comment\nimport Foundation\nlet answer: Int = 42\nfunc greet() -> String { return \"hello\" }\n";
    let kotlin =
        "// comment\nfun main() {\n    val answer = 42\n    println(\"answer $answer\")\n}\n";
    let ruby = "# comment\ndef greet(name)\n  \"hello #{name}\" * 42\nend\n";
    let sql = "-- comment\nSELECT name, COUNT(*) AS total\nFROM items\nWHERE size > 42 AND kind = 'file';\n";
    let yaml = "# comment\nfixture:\n  name: \"sample\"\n  count: 42\n  enabled: true\n";
    let lua =
        "-- comment\nlocal answer = 42\nfunction greet(name)\n  return \"hello \" .. name\nend\n";
    let xml = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- comment -->\n<root kind=\"sample\">\n  <item count=\"42\">text</item>\n</root>\n";
    let usda = "#usda 1.0\n(\n    doc = \"sample\"\n)\ndef Xform \"World\"\n{\n    double3 xformOp:translate = (0, 42, 0)\n}\n";
    let css = "/* comment */\nbody {\n  background: #101820;\n  margin: 42px;\n}\n";
    let make = "# comment\nCC := cc\nall: build\n\t@echo done 42\n";
    let mut samples: Vec<(String, String, &'static str)> = Vec::new();
    let mut add = |names: &[&str], content: &str, fragment: &'static str| {
        for name in names {
            samples.push(((*name).to_owned(), content.to_owned(), fragment));
        }
    };
    add(&["sample.rs"], rust, "let answer: u32 = 42;");
    add(&["sample.c"], c, "#include <stdio.h>");
    add(&["sample.h"], header, "#define SAMPLE_H 1");
    add(
        &["sample.cpp", "sample.cc", "sample.cxx"],
        cpp,
        "std::string text",
    );
    add(
        &["sample.hpp", "sample.hh", "sample.hxx"],
        hpp,
        "namespace sample",
    );
    add(
        &["sample.js", "sample.mjs", "sample.cjs"],
        js,
        "export function sum",
    );
    add(&["sample.jsx"], jsx, "className=\"box\"");
    add(
        &["sample.ts", "sample.tsx", "sample.mts", "sample.cts"],
        ts,
        "export interface Item",
    );
    add(&["sample.py", "sample.pyw"], python, "def greet");
    add(
        &["sample.sh", "sample.bash", "sample.zsh"],
        shell,
        "set -eu",
    );
    add(&["sample.bat", "sample.cmd"], batch, "set ANSWER=42");
    add(&["sample.json"], json, "\"theme\"");
    add(&["sample.toml"], toml, "[window]");
    add(&["sample.java"], java, "public class Sample");
    add(&["sample.cs"], csharp, "using System;");
    add(&["sample.go"], go, "package main");
    add(&["sample.swift"], swift, "import Foundation");
    add(&["sample.kt", "sample.kts"], kotlin, "val answer = 42");
    add(&["sample.rb"], ruby, "def greet(name)");
    add(&["sample.sql"], sql, "SELECT name");
    add(&["sample.yaml", "sample.yml"], yaml, "fixture:");
    add(&["sample.lua"], lua, "local answer = 42");
    add(
        &[
            "sample.xml",
            "sample.xsd",
            "sample.xsl",
            "sample.xslt",
            "sample.xaml",
            "sample.csproj",
            "sample.fsproj",
            "sample.vbproj",
            "sample.vcxproj",
            "sample.props",
            "sample.targets",
            "sample.resx",
            "app.manifest",
        ],
        xml,
        "<root kind=\"sample\">",
    );
    add(&["sample.usda"], usda, "def Xform");
    add(&["sample.css"], css, "background: #101820;");
    add(&["rules.mk", "rules.mak", "GNUmakefile"], make, "CC := cc");
    samples.push(("table.csv".into(), table(40, ','), "r00001,"));
    samples.push(("table.tsv".into(), table(40, '\t'), "r00001\t"));
    samples
}

/// A 24-bit uncompressed Windows bitmap (BITMAPINFOHEADER, rows bottom-up).
pub fn bmp(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let row = (width * 3).next_multiple_of(4);
    let size = 54 + row * height;
    let mut output = Vec::with_capacity(size as usize);
    output.extend_from_slice(b"BM");
    output.extend_from_slice(&size.to_le_bytes());
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(&54_u32.to_le_bytes());
    output.extend_from_slice(&40_u32.to_le_bytes());
    output.extend_from_slice(&(width as i32).to_le_bytes());
    output.extend_from_slice(&(height as i32).to_le_bytes());
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&24_u16.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(&(row * height).to_le_bytes());
    output.extend_from_slice(&2835_i32.to_le_bytes());
    output.extend_from_slice(&2835_i32.to_le_bytes());
    output.extend_from_slice(&[0; 8]);
    for y in (0..height).rev() {
        let start = output.len();
        for x in 0..width {
            let [red, green, blue] = pixel(x, y);
            output.extend_from_slice(&[blue, green, red]);
        }
        output.resize(start + row as usize, 0);
    }
    output
}

/// An icon file holding one 32-bit image of `edge` x `edge` pixels as a device-
/// independent bitmap (the classic icon payload: BGRA rows bottom-up at twice the
/// height, then an all-zero AND mask).
pub fn ico(edge: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mask_row = edge.div_ceil(32) * 4;
    let image_size = 40 + edge * edge * 4 + mask_row * edge;
    let mut output = Vec::with_capacity(22 + image_size as usize);
    output.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    let byte_edge = if edge >= 256 { 0 } else { edge as u8 };
    output.extend_from_slice(&[byte_edge, byte_edge, 0, 0]);
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&32_u16.to_le_bytes());
    output.extend_from_slice(&image_size.to_le_bytes());
    output.extend_from_slice(&22_u32.to_le_bytes());
    output.extend_from_slice(&40_u32.to_le_bytes());
    output.extend_from_slice(&(edge as i32).to_le_bytes());
    output.extend_from_slice(&((edge * 2) as i32).to_le_bytes());
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&32_u16.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(&(edge * edge * 4 + mask_row * edge).to_le_bytes());
    output.extend_from_slice(&[0; 16]);
    for y in (0..edge).rev() {
        for x in 0..edge {
            let [red, green, blue] = pixel(x, y);
            output.extend_from_slice(&[blue, green, red, 255]);
        }
    }
    output.resize(output.len() + (mask_row * edge) as usize, 0);
    output
}

/// A little-endian baseline TIFF: uncompressed 8-bit RGB in one strip.
pub fn tiff(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    const ENTRIES: u16 = 10;
    let ifd = 8_u32;
    let bits = ifd + 2 + u32::from(ENTRIES) * 12 + 4;
    let data = bits + 6;
    let length = width * height * 3;
    let mut output = b"II*\0".to_vec();
    output.extend_from_slice(&ifd.to_le_bytes());
    output.extend_from_slice(&ENTRIES.to_le_bytes());
    let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
        output.extend_from_slice(&tag.to_le_bytes());
        output.extend_from_slice(&kind.to_le_bytes());
        output.extend_from_slice(&count.to_le_bytes());
        output.extend_from_slice(&value.to_le_bytes());
    };
    const SHORT: u16 = 3;
    const LONG: u16 = 4;
    entry(256, LONG, 1, width);
    entry(257, LONG, 1, height);
    entry(258, SHORT, 3, bits);
    entry(259, SHORT, 1, 1);
    entry(262, SHORT, 1, 2);
    entry(273, LONG, 1, data);
    entry(277, SHORT, 1, 3);
    entry(278, LONG, 1, height);
    entry(279, LONG, 1, length);
    entry(284, SHORT, 1, 1);
    output.extend_from_slice(&0_u32.to_le_bytes());
    for _ in 0..3 {
        output.extend_from_slice(&8_u16.to_le_bytes());
    }
    for y in 0..height {
        for x in 0..width {
            output.extend_from_slice(&pixel(x, y));
        }
    }
    output
}

/// A colourful test picture: a gradient with a dark diagonal band.
pub fn picture(x: u32, y: u32) -> [u8; 3] {
    if (x + y) % 40 < 6 {
        [20, 24, 30]
    } else {
        [
            (x * 4 % 256) as u8,
            (y * 5 % 256) as u8,
            ((x + y) * 3 % 256) as u8,
        ]
    }
}

/// Standard base64 with padding (RFC 4648), for `data:` URIs.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = chunk
            .iter()
            .enumerate()
            .fold(0_u32, |value, (index, byte)| {
                value | u32::from(*byte) << (16 - 8 * index)
            });
        for index in 0..4 {
            text.push(if index <= chunk.len() {
                TABLE[(value >> (18 - 6 * index) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    text
}

/// A colour a pixel count must reach: (colour, at least, at most) pixels within six
/// steps per channel.
pub type Count = ([u8; 3], usize, usize);

/// An SVG that uses one feature the image decoder draws, on a white 200 x 120 page
/// (shown at 1:1: an image that fits the window is not enlarged): its name, its text and
/// the pixel counts that prove the feature was drawn (or, for a script and links,
/// that nothing else was).
pub struct SvgFeature {
    pub name: &'static str,
    pub text: String,
    pub counts: &'static [Count],
}

const SVG_OPEN: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120" viewBox="0 0 200 120"><rect width="200" height="120" fill="#ffffff"/>"##;
const MAGENTA: [u8; 3] = [0xff, 0x00, 0xff];

/// A marker, a pattern, a mask, a clip path, a blur filter, an embedded `data:` PNG and
/// a `switch` fallback for `foreignObject`, each drawn; a script and images linked to a
/// local file and a URL, none of which is run or shown. `linked.png` (magenta) must lie
/// beside them.
pub fn svg_features() -> Vec<SvgFeature> {
    let embedded = crate::encode::png(8, 8, |_, _| [0x30, 0xc0, 0x60]);
    let feature = |name: &'static str, body: String, counts: &'static [Count]| SvgFeature {
        name,
        text: format!("{SVG_OPEN}{body}</svg>"),
        counts,
    };
    vec![
        // Two 12 x 12 squares at the line's ends (markerUnits in user space).
        feature(
            "svg-marker.svg",
            r##"<defs><marker id="m" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="12" markerHeight="12" markerUnits="userSpaceOnUse"><rect width="10" height="10" fill="#ff0080"/></marker></defs><path d="M20 60 L180 60" stroke="#202020" stroke-width="4" marker-start="url(#m)" marker-end="url(#m)"/>"##.into(),
            &[([0xff, 0x00, 0x80], 200, 320)],
        ),
        // Half of a 160 x 80 rectangle: a checkerboard of 10-pixel squares.
        feature(
            "svg-pattern.svg",
            r##"<defs><pattern id="p" width="20" height="20" patternUnits="userSpaceOnUse"><rect width="10" height="10" fill="#00a0ff"/><rect x="10" y="10" width="10" height="10" fill="#00a0ff"/></pattern></defs><rect x="20" y="20" width="160" height="80" fill="url(#p)"/>"##.into(),
            &[([0x00, 0xa0, 0xff], 5800, 6600)],
        ),
        // The whole page in red, masked to a circle of radius 40 (5,027 pixels).
        feature(
            "svg-mask.svg",
            r##"<defs><mask id="k"><rect width="200" height="120" fill="#000000"/><circle cx="100" cy="60" r="40" fill="#ffffff"/></mask></defs><rect width="200" height="120" fill="#e01010" mask="url(#k)"/>"##.into(),
            &[([0xe0, 0x10, 0x10], 4500, 5300)],
        ),
        // The whole page in blue, clipped to the same circle.
        feature(
            "svg-clip.svg",
            r##"<defs><clipPath id="c"><circle cx="100" cy="60" r="40"/></clipPath></defs><rect width="200" height="120" fill="#1040e0" clip-path="url(#c)"/>"##.into(),
            &[([0x10, 0x40, 0xe0], 4500, 5300)],
        ),
        // A blurred black rectangle: its solid middle stays, its edges become greys.
        feature(
            "svg-blur.svg",
            r##"<defs><filter id="f" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="6"/></filter></defs><rect x="60" y="30" width="80" height="60" fill="#000000" filter="url(#f)"/>"##.into(),
            &[([0x00, 0x00, 0x00], 100, 3000), ([0x80, 0x80, 0x80], 40, 2000)],
        ),
        // An 8 x 8 PNG in a data: URI, drawn 40 x 40.
        feature(
            "svg-data-image.svg",
            format!(
                r##"<image x="20" y="20" width="40" height="40" href="data:image/png;base64,{}"/>"##,
                base64(&embedded)
            ),
            &[([0x30, 0xc0, 0x60], 1300, 1700)],
        ),
        // The fallback of a switch whose foreignObject is not drawn.
        feature(
            "svg-switch.svg",
            r##"<switch><foreignObject requiredFeatures="http://www.w3.org/TR/SVG11/feature#Extensibility" width="200" height="120"><div xmlns="http://www.w3.org/1999/xhtml">HTML</div></foreignObject><rect width="200" height="120" fill="#c08000"/></switch>"##.into(),
            &[([0xc0, 0x80, 0x00], 23000, 24000)],
        ),
        // A script that would paint the page magenta and two linked images (a local
        // magenta file and a URL) that would cover a third of it: none runs or shows.
        feature(
            "svg-script-links.svg",
            r##"<script>document.getElementById('r').setAttribute('fill', '#ff00ff');</script><rect id="r" width="200" height="120" fill="#2040a0"/><image x="20" y="20" width="60" height="60" href="linked.png"/><image x="110" y="20" width="60" height="60" href="https://example.invalid/linked.png"/>"##.into(),
            &[([0x20, 0x40, 0xa0], 23500, 24000), (MAGENTA, 0, 0)],
        ),
    ]
}

/// The magenta picture `svg-script-links.svg` links to.
pub fn linked_png() -> Vec<u8> {
    crate::encode::png(60, 60, |_, _| MAGENTA)
}

/// SVGs the decoder refuses before drawing, each with the start of the sentence it
/// reports (a complete sentence starting with "This SVG", shown as it is).
pub fn hostile_svgs() -> Vec<(&'static str, String, &'static str)> {
    let mut deep =
        String::from(r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"##);
    for _ in 0..70 {
        deep.push_str("<g>");
    }
    deep.push_str(r##"<rect width="10" height="10"/>"##);
    for _ in 0..70 {
        deep.push_str("</g>");
    }
    deep.push_str("</svg>");
    vec![
        (
            "svg-no-namespace.svg",
            r##"<svg width="10" height="10"><rect width="10" height="10"/></svg>"##.into(),
            "This SVG does not declare the SVG namespace on its root element, so it is not a \
             standalone SVG image.",
        ),
        (
            "svg-entities.svg",
            r##"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text y="8">&b;</text></svg>"##.into(),
            "This SVG declares XML entities (a DOCTYPE internal subset), which are not \
             supported.",
        ),
        (
            "svg-deep.svg",
            deep,
            "This SVG nests elements and references more than 64 levels deep (line",
        ),
        (
            "svg-cycle.svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
<use id="a" href="#a"/>
</svg>"##.into(),
            "This SVG is invalid: the reference on line 2 leads back to itself.",
        ),
    ]
}

/// A GIF89a of two 48 x 48 frames, `first` and then `second` all over, 100 ms apart:
/// two single-frame GIFs of one palette (`encode::gif`) spliced into one stream, each
/// frame after a graphic control extension.
pub fn two_frame_gif(first: [u8; 3], second: [u8; 3]) -> Vec<u8> {
    let mut palette = [[0_u8; 3]; 256];
    palette[1] = first;
    palette[2] = second;
    let frames = [
        crate::encode::gif(48, 48, &palette, |_, _| 1),
        crate::encode::gif(48, 48, &palette, |_, _| 2),
    ];
    // Header, logical screen descriptor and the 256-entry global colour table.
    const HEAD: usize = 6 + 7 + 256 * 3;
    let mut output = frames[0][..HEAD].to_vec();
    // Loop forever (NETSCAPE2.0), as animated GIFs do.
    output.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00");
    for frame in &frames {
        // Graphic control extension: no disposal, 10 hundredths of a second.
        output.extend_from_slice(&[0x21, 0xf9, 0x04, 0x00, 10, 0, 0, 0]);
        // The image descriptor and its data, without the stream's trailer.
        output.extend_from_slice(&frame[HEAD..frame.len() - 1]);
    }
    output.push(0x3b);
    output
}

/// An animated WebP of two 48 x 48 lossless frames, `first` and then `second` all over,
/// 100 ms each: the VP8L chunks of two still WebPs (`encode::webp`) in ANMF frames after
/// the VP8X and ANIM chunks.
pub fn two_frame_webp(first: [u8; 3], second: [u8; 3]) -> Vec<u8> {
    let still =
        |colour: [u8; 3]| crate::encode::webp(48, 48, colour, [255, 255, 255], |_, _| false);
    let u24 = |value: u32| value.to_le_bytes()[..3].to_vec();
    let chunk = |tag: &[u8], payload: &[u8]| {
        let mut chunk = tag.to_vec();
        chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        chunk.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            chunk.push(0);
        }
        chunk
    };
    let mut body = b"WEBP".to_vec();
    // VP8X: the animation flag, then the canvas size minus one.
    let mut vp8x = vec![0x02, 0, 0, 0];
    vp8x.extend(u24(47));
    vp8x.extend(u24(47));
    body.extend(chunk(b"VP8X", &vp8x));
    // ANIM: a white background (BGRA), looping forever.
    body.extend(chunk(b"ANIM", &[255, 255, 255, 255, 0, 0]));
    for colour in [first, second] {
        // A still WebP is RIFF, its size, WEBP, then the VP8L chunk.
        let frame = still(colour);
        let mut anmf = Vec::new();
        anmf.extend(u24(0));
        anmf.extend(u24(0));
        anmf.extend(u24(47));
        anmf.extend(u24(47));
        anmf.extend(u24(100));
        // Do not blend, no disposal.
        anmf.push(0x02);
        anmf.extend_from_slice(&frame[12..]);
        body.extend(chunk(b"ANMF", &anmf));
    }
    let mut output = b"RIFF".to_vec();
    output.extend_from_slice(&(body.len() as u32).to_le_bytes());
    output.extend(body);
    output
}

/// The licensed AVIF fixtures of the image decoder (libavif 1.3.0 test data under its
/// BSD-2-Clause license, provenance in `plugins/image-decoder/tests/fixtures/README.md`):
/// an opaque white pixel, and the quality-100 encoding of `draw_points.png` with an
/// alpha item, with that PNG as the independent reference.
pub const WHITE_AVIF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../plugins/image-decoder/tests/fixtures/white_1x1.avif"
));
pub const POINTS_AVIF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../plugins/image-decoder/tests/fixtures/draw_points_idat.avif"
));
pub const POINTS_PNG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../plugins/image-decoder/tests/fixtures/draw_points.png"
));

/// A damaged or unusual file and how its viewer must answer it.
pub struct Damaged {
    pub name: &'static str,
    pub bytes: Vec<u8>,
    pub kind: &'static str,
}

/// Damaged files of every viewer kind: truncated, empty, garbage after a valid
/// signature, invalid text encodings, NUL bytes, broken markup.
pub fn damaged() -> Vec<Damaged> {
    let mut random = Random::new(0x5eed_0004);
    let noise = |random: &mut Random, length: usize| -> Vec<u8> {
        (0..length).map(|_| random.next() as u8).collect()
    };
    let pdf = pdf(3);
    let jpeg = encode::jpeg(16, 8, |x, y| ((x * 15 + y * 8) % 256) as u8);
    let mut palette = [[0_u8; 3]; 256];
    for (index, colour) in palette.iter_mut().enumerate() {
        *colour = [index as u8, (255 - index) as u8, (index * 3 % 256) as u8];
    }
    let gif = encode::gif(48, 48, &palette, |x, y| ((x / 6 + y / 6) * 32 % 256) as u8);
    let webp = encode::webp(64, 64, [40, 60, 90], [230, 200, 120], |x, y| {
        (x + y) % 16 < 8
    });
    let bmp = bmp(64, 48, picture);
    let tiff = tiff(40, 30, picture);
    let half = |bytes: &[u8]| bytes[..bytes.len() * 3 / 5].to_vec();
    let mut garbage_pdf = b"%PDF-1.7\n".to_vec();
    garbage_pdf.extend(noise(&mut random, 4096));
    let mut invalid = b"valid start\n".to_vec();
    invalid.extend_from_slice(&[0xC3, 0x28, b'\n', 0xF0, 0x28, 0x8C, 0xBC, b'\n', 0xFF]);
    invalid.extend_from_slice(b"\nvalid end\n");
    let mut odd = vec![0xFF, 0xFE];
    odd.extend_from_slice(&[b'h', 0, b'i', 0, b'\n', 0, 0x41]);
    let nul: Vec<u8> = b"text\0with\0nul\0bytes\n".repeat(200);
    vec![
        Damaged {
            name: "truncated.pdf",
            bytes: half(&pdf),
            kind: "PDF cut at 60 %",
        },
        Damaged {
            name: "garbage.pdf",
            bytes: garbage_pdf,
            kind: "PDF signature followed by noise",
        },
        Damaged {
            name: "empty.pdf",
            bytes: Vec::new(),
            kind: "empty PDF",
        },
        Damaged {
            name: "truncated.jpg",
            bytes: half(&jpeg),
            kind: "JPEG cut at 60 %",
        },
        Damaged {
            name: "truncated.gif",
            bytes: half(&gif),
            kind: "GIF cut at 60 %",
        },
        Damaged {
            name: "truncated.webp",
            bytes: half(&webp),
            kind: "WebP cut at 60 %",
        },
        Damaged {
            name: "truncated.bmp",
            bytes: half(&bmp),
            kind: "BMP cut at 60 %",
        },
        Damaged {
            name: "truncated.tif",
            bytes: half(&tiff),
            kind: "TIFF cut at 60 %",
        },
        Damaged {
            name: "broken.svg",
            bytes:
                b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\"><rect x=\""
                    .to_vec(),
            kind: "unterminated SVG",
        },
        Damaged {
            name: "invalid-utf8.txt",
            bytes: invalid,
            kind: "text with invalid UTF-8",
        },
        Damaged {
            name: "odd-utf16.txt",
            bytes: odd,
            kind: "UTF-16 with an odd byte count",
        },
        Damaged {
            name: "nul.txt",
            bytes: nul,
            kind: "text with NUL bytes",
        },
        Damaged {
            name: "broken.md",
            bytes: b"# Broken heading\n\n```rust\nunterminated fence\n| a | b |\n|---|\n| 1 |\n"
                .to_vec(),
            kind: "Markdown with an open fence and a short table",
        },
        Damaged {
            name: "broken.html",
            bytes: b"<html><body><h1>Broken page<p>unclosed <b>tags<table><tr><td>cell".to_vec(),
            kind: "HTML with unclosed tags",
        },
        Damaged {
            name: "broken.json",
            bytes: b"{\"a\": [1, 2,, \"b\": }\n".to_vec(),
            kind: "invalid JSON",
        },
    ]
}

/// A program on the `PATH`, if there is one.
pub fn program(name: &str) -> Option<PathBuf> {
    // Windows programs carry their extension (Git for Windows ships xz.exe, bzip2.exe).
    let names = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_owned()]
    } else {
        vec![name.to_owned()]
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|folder| {
            names
                .iter()
                .map(|name| folder.join(name))
                .find(|candidate| candidate.is_file())
        })
    })
}

/// Run `program` with `arguments` in `folder`, `input` on its standard input; its
/// standard output, or why it failed.
pub fn run(
    program: &Path,
    arguments: &[&str],
    folder: &Path,
    input: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let mut child = Command::new(program)
        .args(arguments)
        .current_dir(folder)
        // Unicode member names need a UTF-8 locale in the host's archivers.
        .env("LC_ALL", "C.UTF-8")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{}: {error}", program.display()))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let data = input.to_vec();
        // Feed from a thread so a full output pipe cannot block the writer.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("{}: {error}", program.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} {arguments:?} ended {}: {}",
            program.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// `data` compressed by a host program reading standard input (`xz -c`, `bzip2 -c`,
/// `zstd -c`); Err names the missing program.
pub fn compress(name: &str, data: &[u8], folder: &Path) -> Result<Vec<u8>, String> {
    let path = program(name).ok_or_else(|| format!("no {name} program on this machine"))?;
    let arguments: &[&str] = match name {
        "zstd" => &["-q", "-c", "-"],
        _ => &["-c"],
    };
    run(&path, arguments, folder, Some(data))
}

/// Archive `entry` (a file or folder in `folder`) with a host program: `7z` (7-Zip,
/// also `7za` or `7zr`) or `zip` (Info-ZIP, deflated); `output` must not exist yet.
pub fn archive_folder(kind: &str, folder: &Path, entry: &str, output: &Path) -> Result<(), String> {
    let output = output
        .to_str()
        .ok_or_else(|| format!("{} is not UTF-8", output.display()))?;
    match kind {
        "7z" => {
            let program = ["7z", "7za", "7zr"]
                .iter()
                .find_map(|name| program(name))
                .ok_or("no 7-Zip program (7z, 7za or 7zr) on this machine")?;
            run(
                &program,
                &["a", "-t7z", "-bd", "-y", output, entry],
                folder,
                None,
            )
            .map(|_| ())
        }
        "zip" => {
            let program = program("zip").ok_or("no zip program on this machine")?;
            run(&program, &["-r", "-q", "-X", output, entry], folder, None).map(|_| ())
        }
        other => Err(format!("unknown archiver {other}")),
    }
}

/// The members every generated archive holds, like the fixture's `bundle.zip`.
pub fn bundle_members() -> Vec<encode::Member> {
    vec![
        encode::Member::directory("bundle"),
        encode::Member::file("bundle/readme.txt", "Inside the bundle.\n"),
        encode::Member::file("bundle/ünïcödé 🦀.txt", "Unicode member.\n"),
        encode::Member::directory("bundle/empty"),
        encode::Member::directory("bundle/nested"),
        encode::Member::file("bundle/nested/notes.md", "# Notes\n\nNested member.\n"),
        encode::Member::directory("bundle/nested/deeper"),
        encode::Member::file(
            "bundle/nested/deeper/data.bin",
            (0..=255_u8).cycle().take(1000).collect::<Vec<u8>>(),
        ),
    ]
}

/// Write `members` as a folder tree below `root` (for the host's archivers).
pub fn materialize(root: &Path, members: &[encode::Member]) -> Result<(), String> {
    for member in members {
        let path = root.join(&member.path);
        match &member.data {
            Some(data) => {
                write(root, &member.path, data)?;
            }
            None => {
                fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?
            }
        }
    }
    Ok(())
}
