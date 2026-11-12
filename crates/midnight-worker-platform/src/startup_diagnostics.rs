//! Bounded opt-in timing without source paths, environment values, or protocol data.
use std::{fmt::Write as _, io::Write as _, time::Instant};

/// Internal host diagnostics. Call sites supply fixed stage labels only.
pub struct Trace {
    name: &'static str,
    start: Instant,
    marks: [(&'static str, f64); 16],
    used: usize,
}

impl Trace {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            start: Instant::now(),
            marks: [("", 0.0); 16],
            used: 0,
        }
    }

    pub fn mark(&mut self, name: &'static str) {
        if let Some(mark) = self.marks.get_mut(self.used) {
            *mark = (name, self.start.elapsed().as_secs_f64() * 1000.0);
            self.used += 1;
        }
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        // Emit once after the measured operation, avoiding synchronous writes between stages.
        let mut line = String::with_capacity(768);
        let _ = write!(line, "startup_trace={}", self.name);
        for (name, elapsed) in &self.marks[..self.used] {
            let _ = write!(line, " {name}_ms={elapsed:.3}");
        }
        let _ = writeln!(std::io::stderr().lock(), "{line}");
    }
}
