use std::time::Duration;

use tracing::info;

const SKIP_FRAMES: usize = 10;

#[derive(Debug, Default)]
pub struct MeasureSeries {
    stats: Stats,
    frame: usize,
}

impl MeasureSeries {
    pub fn add_sample(&mut self, duration: Duration) {
        self.frame += 1;
        if self.frame <= SKIP_FRAMES {
            return;
        }

        self.stats.min = if self.stats.count == 0 {
            duration
        } else {
            self.stats.min.min(duration)
        };
        self.stats.max = self.stats.max.max(duration);
        self.stats.sum += duration;
        self.stats.count += 1;
    }
}

impl Drop for MeasureSeries {
    fn drop(&mut self) {
        if self.stats.count == 0 {
            return;
        }

        info!(
            "Measure series: mean: {:?} ({:?}-{:?}, {} samples, {} skipped)",
            self.stats.mean().unwrap(),
            self.stats.min,
            self.stats.max,
            self.stats.count,
            SKIP_FRAMES
        )
    }
}

#[allow(unused)]
#[derive(Debug, Default)]
struct Stats {
    min: Duration,
    sum: Duration,
    max: Duration,
    count: usize,
}

impl Stats {
    fn mean(&self) -> Option<Duration> {
        if self.count == 0 {
            return None;
        }
        Some(self.sum / self.count as u32)
    }
}
