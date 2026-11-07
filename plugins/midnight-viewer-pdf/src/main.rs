fn main() -> std::io::Result<()> {
    midnight_image_viewer_sdk::serve_document(midnight_viewer_pdf::decode)
}
