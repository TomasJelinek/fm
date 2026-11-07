//! Native containment fixture; accepts only fixed stdin commands, never a shell command.
use std::io::{self, BufRead, Write};

fn main() -> io::Result<()> {
    let mut input = io::BufReader::new(io::stdin().lock());
    let mut output = io::stdout().lock();
    midnight_worker_sdk::enter(&mut input, &mut output)?;
    let mut path = String::new();
    input.read_line(&mut path)?;
    let denied_file = std::fs::read(path.trim_end()).is_err();
    let mut port = String::new();
    input.read_line(&mut port)?;
    let port = port.trim_end().parse().map_err(io::Error::other)?;
    let denied_network = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(100),
    )
    .is_err();
    writeln!(output, "{denied_file} {denied_network}")?;
    output.flush()
}
