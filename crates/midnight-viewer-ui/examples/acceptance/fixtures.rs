use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub fn fixtures(output: &Path, existing: Option<&Path>) -> io::Result<Vec<PathBuf>> {
    // Each invocation receives its own output directory; fixtures never replace user content.
    std::fs::create_dir_all(output)?;
    if let Some(existing) = existing {
        // Explicit reuse lets desktop scanners finish touching authored fixture metadata.
        // Reused content is read only; native capture still rejects any subsequent revision.
        let paths: Vec<_> = [
            "text.txt",
            "binary.bin",
            "source.rs",
            "markdown.md",
            "image.bmp",
            "document.pdf",
        ]
        .into_iter()
        .map(|name| existing.join(name))
        .collect();
        settle(&paths)?;
        return Ok(paths);
    }
    let directory = output.join("fixtures");
    std::fs::create_dir(&directory)?;
    let text = format!(
        "FIRST Žluťoučký 世界 😀\n{}\n{}\nLAST needle target\n",
        "long word ".repeat(300),
        "ordinary line café\n".repeat(4000)
    );
    let content = [
        ("text.txt", text.into_bytes()),
        ("binary.bin", vec![0, 1, 2, 255, b'A']),
        ("source.rs", b"// Source color acceptance\nfn main() {\n    let number = 42;\n    println!(\"Hello source\");\n}\n".to_vec()),
        ("markdown.md", format!("{}{}", concat!(
            "# Heading\n\nHello **world**!\n\n",
            "| Name | Count | Status |\n| :--- | ---: | :---: |\n",
            "| **Café 世界** | 42 | `ready` |\n| Empty | | ~~old~~ |\n\n",
            "- [x] Done\n- [ ] Pending\n\n",
            "A [link](https://example.invalid/) stays inert.\n"
        ), (0..500).map(|index| format!("\n\n## Scroll section {index}\n\nParagraph **bold** with `code` café.\n\n> A quotation.\n\n```rust\nlet value = {index};\n```\n")).collect::<String>()).into_bytes()),
        ("image.bmp", bitmap()),
        ("document.pdf", pdf()),
    ];
    let paths: Vec<PathBuf> = content
        .into_iter()
        .map(|(name, bytes)| {
            let path = directory.join(name);
            std::fs::write(&path, bytes)?;
            Ok(path)
        })
        .collect::<io::Result<_>>()?;
    settle(&paths)?;
    Ok(paths)
}

fn settle(paths: &[PathBuf]) -> io::Result<()> {
    // Native metadata can advance shortly after fixture creation. Capture only after a
    // stable interval, exactly as the engine acceptance tests do; production stays strict.
    let versions = || {
        paths
            .iter()
            .map(|path| {
                midnight_plugin_host::CapturedFile::capture(path)
                    .map(|file| file.stable_version())
                    .map_err(io::Error::other)
            })
            .collect::<io::Result<Vec<_>>>()
    };
    let started = Instant::now();
    let mut quiet = Instant::now();
    let mut previous = versions()?;
    while quiet.elapsed() < Duration::from_millis(500) {
        if started.elapsed() > Duration::from_secs(5) {
            return Err(io::Error::other(
                "Generated fixture revisions did not settle",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
        let current = versions()?;
        if current != previous {
            previous = current;
            quiet = Instant::now();
        }
    }
    Ok(())
}

fn bitmap() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.extend_from_slice(&60054u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&200i32.to_le_bytes());
    bytes.extend_from_slice(&100i32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for y in 0..100u8 {
        for x in 0..200u8 {
            bytes.extend_from_slice(&[x, y, 255]);
        }
    }
    bytes
}

fn pdf() -> Vec<u8> {
    // Authored two-page document with unambiguous render colors; no external assets.
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Contents 4 0 R >>".to_owned(),
        stream("1 0 0 rg 0 0 400 300 re f\n"),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Contents 6 0 R >>".to_owned(),
        stream("0 0 1 rg 0 0 400 300 re f\n"),
    ];
    let mut pdf = "%PDF-1.7\n".to_owned();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    pdf.into_bytes()
}

fn stream(commands: &str) -> String {
    format!(
        "<< /Length {} >>\nstream\n{commands}endstream",
        commands.len()
    )
}
