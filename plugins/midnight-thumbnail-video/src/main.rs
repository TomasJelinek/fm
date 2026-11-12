fn main() -> std::io::Result<()> {
    midnight_thumbnail_sdk::serve(midnight_thumbnail_video::decode)
}
