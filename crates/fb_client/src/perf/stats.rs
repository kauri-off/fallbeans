//! A run of frames in numbers: averages and percentiles, the 1% low, stutters, and what held the frames back.
use serde::Serialize;

/// One frame, ms (NaN: not measured).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Real time at its end, s.
    pub t: f32,
    /// Since the frame before.
    pub frame: f32,
    /// Of it, the frame limit's sleep.
    pub sleep: f32,
    /// The main world's schedules, `First` to `Last`.
    pub main: f32,
    /// The render thread's last finished frame (the main frame before this one): its `Render` schedule,
    /// `ExtractCommands` to `PostCleanup`, the swapchain waits included. Extract itself (on the main thread,
    /// after `Last`) and the main thread's wait for the render thread are in neither `main` nor `render`:
    /// only in `frame`.
    pub render: f32,
    /// Of it, waiting for the swapchain: acquire (`prepare_windows`) and present (the `present_frames` span,
    /// so only with the profiler built in; NaN without).
    pub wait: f32,
    /// The GPU's timed passes, shadows not among them (a few frames late; NaN with `--no-gpu-timers`).
    pub gpu: f32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Stat {
    pub avg: f32,
    pub p50: f32,
    pub p95: f32,
    pub p99: f32,
    pub max: f32,
}

impl Stat {
    pub fn of(values: impl Iterator<Item = f32>) -> Option<Stat> {
        // (NaN: not measured; a negative time is a driver's wrong GPU timestamp.)
        let mut v: Vec<f32> = values.filter(|x| x.is_finite() && *x >= 0.0).collect();
        if v.is_empty() {
            return None;
        }
        v.sort_by(f32::total_cmp);
        let at = |q: f32| v[((v.len() - 1) as f32 * q).round() as usize];
        Some(Stat {
            avg: v.iter().sum::<f32>() / v.len() as f32,
            p50: at(0.5),
            p95: at(0.95),
            p99: at(0.99),
            max: v[v.len() - 1],
        })
    }
}

/// What the frames waited on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Bound {
    /// The frame limit.
    Limit,
    /// The display's refresh (vsync), with time to spare.
    Vsync,
    Gpu,
    /// The game's logic, animation, UI: the main world.
    CpuMain,
    /// Preparing and submitting the draws: the render world.
    CpuRender,
    Mixed,
}

impl Bound {
    pub fn label(self) -> &'static str {
        match self {
            Bound::Limit => "fps limit",
            Bound::Vsync => "vsync",
            Bound::Gpu => "GPU-bound",
            Bound::CpuMain => "CPU-bound (main)",
            Bound::CpuRender => "CPU-bound (render)",
            Bound::Mixed => "mixed",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    pub frames: usize,
    pub secs: f32,
    pub fps: f32,
    /// The frame rate of the slowest 1% of the frames.
    pub fps_low1: f32,
    /// Frames over twice the median (and 4 ms more than it).
    pub stutters: usize,
    pub frame: Stat,
    pub sleep: Stat,
    pub main: Stat,
    pub render: Stat,
    pub wait: Option<Stat>,
    pub gpu: Option<Stat>,
    pub bound: Option<Bound>,
}

impl Summary {
    pub fn of(frames: &[Frame], vsync: bool) -> Summary {
        let Some(frame) = Stat::of(frames.iter().map(|f| f.frame)) else {
            return Summary::default();
        };
        let stat = |f: fn(&Frame) -> f32| Stat::of(frames.iter().map(f));
        let mut sorted: Vec<f32> = frames.iter().map(|f| f.frame).collect();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let worst = &sorted[..sorted.len().div_ceil(100)];
        let low1 = worst.iter().sum::<f32>() / worst.len() as f32;
        let mut s = Summary {
            frames: frames.len(),
            secs: frames.iter().map(|f| f.frame).sum::<f32>() / 1000.0,
            fps: 1000.0 / frame.avg,
            fps_low1: 1000.0 / low1,
            stutters: frames
                .iter()
                .filter(|f| f.frame > 2.0 * frame.p50 && f.frame > frame.p50 + 4.0)
                .count(),
            frame,
            sleep: stat(|f| f.sleep).unwrap_or_default(),
            main: stat(|f| f.main).unwrap_or_default(),
            render: stat(|f| f.render).unwrap_or_default(),
            wait: stat(|f| f.wait),
            gpu: stat(|f| f.gpu),
            bound: None,
        };
        s.bound = Some(s.bound_by(vsync));
        s
    }

    fn bound_by(&self, vsync: bool) -> Bound {
        let frame = self.frame.avg;
        if self.sleep.avg > 0.2 * frame {
            return Bound::Limit;
        }
        let work = frame - self.sleep.avg;
        let wait = self.wait.map_or(0.0, |w| w.avg);
        let gpu = self.gpu.map(|g| g.avg);
        if gpu.is_some_and(|g| g > 0.8 * work) {
            return Bound::Gpu;
        }
        if self.main.avg > 0.8 * work {
            return Bound::CpuMain;
        }
        if self.render.avg - wait > 0.8 * work {
            return Bound::CpuRender;
        }
        if wait > 0.3 * work {
            return match gpu {
                Some(_) if vsync => Bound::Vsync,
                Some(_) => Bound::Mixed,
                None => Bound::Gpu,
            };
        }
        Bound::Mixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ms: f32) -> Frame {
        Frame {
            t: 0.0,
            frame: ms,
            sleep: 0.0,
            main: 1.0,
            render: 1.0,
            wait: 0.0,
            gpu: ms * 0.9,
        }
    }

    #[test]
    fn percentiles_and_lows() {
        let s = Stat::of((1..=100).map(|x| x as f32)).unwrap();
        assert_eq!((s.p50, s.p99, s.max), (51.0, 99.0, 100.0));
        assert!(Stat::of([f32::NAN].into_iter()).is_none());
        let mut frames: Vec<Frame> = (0..99).map(|_| frame(10.0)).collect();
        frames.push(frame(50.0));
        let s = Summary::of(&frames, false);
        assert_eq!(s.stutters, 1);
        assert!((s.fps_low1 - 20.0).abs() < 0.01);
        assert_eq!(s.bound, Some(Bound::Gpu));
    }
}
