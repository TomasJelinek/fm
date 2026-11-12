//! Opt-in aggregate glyph timings, without per-pixel clocks or content/identity output.
#[cfg(debug_assertions)]
use std::time::{Duration, Instant};

pub(super) struct Trace {
    #[cfg(debug_assertions)]
    state: Option<State>,
}
#[cfg(debug_assertions)]
struct State {
    start: Instant,
    generation: Duration,
    composition: Duration,
    generation_calls: usize,
    composition_calls: usize,
}
impl Trace {
    pub fn new() -> Self {
        Self {
            #[cfg(debug_assertions)]
            state: {
                static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                ENABLED
                    .get_or_init(|| std::env::var_os("MIDNIGHT_VIEWER_GLYPH_TRACE").is_some())
                    .then(|| State {
                        start: Instant::now(),
                        generation: Duration::ZERO,
                        composition: Duration::ZERO,
                        generation_calls: 0,
                        composition_calls: 0,
                    })
            },
        }
    }
    #[inline]
    pub fn generate<T>(&mut self, operation: impl FnOnce() -> T) -> T {
        self.measure(true, operation)
    }
    #[inline]
    pub fn compose<T>(&mut self, operation: impl FnOnce() -> T) -> T {
        self.measure(false, operation)
    }
    #[inline]
    fn measure<T>(&mut self, generation: bool, operation: impl FnOnce() -> T) -> T {
        #[cfg(debug_assertions)]
        let start = self.state.as_ref().map(|_| Instant::now());
        #[cfg(not(debug_assertions))]
        let _ = generation;
        let result = operation();
        #[cfg(debug_assertions)]
        if let Some(start) = start
            && let Some(state) = &mut self.state
        {
            if generation {
                state.generation += start.elapsed();
                state.generation_calls += 1;
            } else {
                state.composition += start.elapsed();
                state.composition_calls += 1;
            }
        }
        result
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if let Some(state) = &self.state {
            use std::io::Write as _;
            let total = state.start.elapsed().as_secs_f64() * 1000.0;
            let _ = writeln!(
                std::io::stderr().lock(),
                "glyph_phase total_ms={total:.3} generation_ms={:.3} composition_ms={:.3} \
                 generation_calls={} composition_calls={}",
                state.generation.as_secs_f64() * 1000.0,
                state.composition.as_secs_f64() * 1000.0,
                state.generation_calls,
                state.composition_calls,
            );
        }
    }
}
