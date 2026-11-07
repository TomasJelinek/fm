fn main() -> std::io::Result<()> {
    midnight_image_viewer_sdk::serve(midnight_viewer_image::decode)
}
