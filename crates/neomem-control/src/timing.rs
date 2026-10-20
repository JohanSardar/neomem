use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Percentiles {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

pub fn measure(
    repetitions: usize,
    mut action: impl FnMut() -> anyhow::Result<()>,
) -> anyhow::Result<Percentiles> {
    let mut samples = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let started = Instant::now();
        action()?;
        samples.push(started.elapsed());
    }
    Ok(percentiles(&mut samples))
}

pub fn percentiles(samples: &mut [Duration]) -> Percentiles {
    samples.sort_unstable();
    let at = |per_mille: usize| {
        let last = samples.len().saturating_sub(1);
        let index = (last * per_mille + 500) / 1000;
        samples.get(index).map_or(0.0, |sample| sample.as_secs_f64() * 1000.0)
    };
    Percentiles { p50_ms: at(500), p95_ms: at(950), max_ms: at(1000) }
}

impl std::fmt::Display for Percentiles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms", self.p50_ms, self.p95_ms, self.max_ms)
    }
}
