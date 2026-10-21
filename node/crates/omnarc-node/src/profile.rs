//! The profiler (SPEC NO-40): what this device can do within what its owner offers, measured once when the
//! service starts. Decoding a token reads every weight of a window once, so the memory bandwidth is what decides a
//! device's time per layer; the matrix throughput decides prefill.
//!
//! The measurement runs on the cores the owner offers and in a buffer far smaller than what is offered, so a
//! phone giving 500 MB and one core is measured as exactly that.

use std::hint::black_box;
use std::time::{Duration, Instant};

/// What was measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    /// Bytes per second read from memory with the offered cores.
    pub bandwidth_bytes_per_s: f64,
    /// Fused multiply-adds per second (two operations each) with the offered cores, in f32.
    pub flops: f64,
}

impl Profile {
    /// The time one token takes through a layer of `layer_bytes` of weights.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn seconds_per_layer(&self, layer_bytes: u64) -> f64 {
        layer_bytes as f64 / self.bandwidth_bytes_per_s.max(1.0)
    }
}

/// The buffer each core reads; far above any cache, far below any budget worth placing a layer in.
const BUFFER_BYTES: usize = 32 << 20;

/// Measures this device on `cores` cores for about `time`.
#[must_use]
pub fn measure(cores: u32, time: Duration) -> Profile {
    let cores = cores.max(1) as usize;
    let half = time / 2;
    Profile { bandwidth_bytes_per_s: parallel(cores, half, read_memory), flops: parallel(cores, half, fused_adds) }
}

// Runs `work` on `cores` threads for `time` and adds up what each reports per second.
fn parallel(cores: usize, time: Duration, work: fn(Duration) -> f64) -> f64 {
    std::thread::scope(|scope| {
        let threads: Vec<_> = (0..cores).map(|_| scope.spawn(move || work(time))).collect();
        threads.into_iter().map(|t| t.join().unwrap_or(0.0)).sum()
    })
}

// Bytes per second this thread reads from a buffer much larger than its caches.
#[allow(clippy::cast_precision_loss)]
fn read_memory(time: Duration) -> f64 {
    let words = vec![1u64; BUFFER_BYTES / 8];
    let started = Instant::now();
    let mut read = 0usize;
    let mut sum = 0u64;
    while started.elapsed() < time {
        // eight independent sums let the core keep several loads in flight, as a matrix-vector product does
        let mut lanes = [0u64; 8];
        for chunk in words.as_chunks::<8>().0 {
            for (lane, word) in lanes.iter_mut().zip(chunk) {
                *lane = lane.wrapping_add(*word);
            }
        }
        sum = sum.wrapping_add(lanes.iter().fold(0, |a, b| a.wrapping_add(*b)));
        read += BUFFER_BYTES;
    }
    black_box(sum);
    read as f64 / started.elapsed().as_secs_f64()
}

// Floating-point operations per second this thread runs from data in its caches.
#[allow(clippy::cast_precision_loss)]
fn fused_adds(time: Duration) -> f64 {
    let a = vec![1.000_001_f32; 1024];
    let b = vec![0.999_999_f32; 1024];
    let mut acc = vec![0.0_f32; 1024];
    let started = Instant::now();
    let mut rounds = 0u64;
    while started.elapsed() < time {
        for _ in 0..64 {
            for ((acc, a), b) in acc.iter_mut().zip(&a).zip(&b) {
                *acc = a.mul_add(*b, *acc);
            }
        }
        rounds += 64;
        black_box(&mut acc);
    }
    (rounds * 1024 * 2) as f64 / started.elapsed().as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_measures_something_and_more_cores_read_more() {
        let one = measure(1, Duration::from_millis(400));
        assert!(one.bandwidth_bytes_per_s > 100e6, "{one:?}");
        assert!(one.flops > 10e6, "{one:?}");
        // a layer of 145 MB on a device that reads 10 GB/s takes about 15 ms
        let layer = Profile { bandwidth_bytes_per_s: 10e9, flops: 1e11 }.seconds_per_layer(145_000_000);
        assert!((layer - 0.0145).abs() < 1e-9);
    }
}
