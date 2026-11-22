/// This process's resident memory in bytes (`VmRSS`; Linux reports no private commit).
pub fn private_bytes() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let line = status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .expect("a VmRSS line");
    let kib: u64 = line
        .split_whitespace()
        .nth(1)
        .and_then(|kib| kib.parse().ok())
        .expect("a size in KiB");
    kib * 1024
}
