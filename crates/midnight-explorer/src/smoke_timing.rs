//! A bounded dwell for native input qualification in the in-memory UI smoke run.
use std::time::Duration;

pub(crate) fn dwell_duration(requested: Option<&str>) -> Duration {
    let milliseconds = requested
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(350);
    Duration::from_millis(milliseconds.clamp(350, 15_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_smoke_dwell_allows_input_but_stays_below_the_shutdown_deadline() {
        for (input, milliseconds) in [
            (None, 350),
            (Some(""), 350),
            (Some("invalid"), 350),
            (Some("-1"), 350),
            (Some("0"), 350),
            (Some("1"), 350),
            (Some("5000"), 5000),
            (Some("15000"), 15000),
            (Some("15001"), 15000),
            (Some("18446744073709551615"), 15000),
            (Some("18446744073709551616"), 350),
        ] {
            assert_eq!(
                dwell_duration(input),
                Duration::from_millis(milliseconds),
                "{input:?}"
            );
        }
    }
}
