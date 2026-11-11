fn main() {
    if midnight_elevation::serve(std::env::args_os().skip(1).collect()).is_err() {
        // The authenticated channel carries bounded errors; never log privileged paths.
        std::process::exit(1);
    }
}
