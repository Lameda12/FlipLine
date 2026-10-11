//! Hold/exposure timing math. Pure functions over a slice of per-frame holds so playback,
//! timecode and exports all agree.

pub const MIN_HOLD: u8 = 1;
pub const MAX_HOLD: u8 = 8;
pub const FPS_PRESETS: [u32; 6] = [6, 8, 12, 15, 24, 30];

/// Total exposure ticks (one tick = 1/fps seconds) in the whole animation.
pub fn total_ticks(holds: &[u8]) -> u32 {
    holds.iter().map(|&h| h as u32).sum()
}

/// First tick at which frame `idx` is shown.
pub fn start_tick(holds: &[u8], idx: usize) -> u32 {
    holds.iter().take(idx).map(|&h| h as u32).sum()
}

/// Which frame is showing at `tick` (0-based). Ticks past the end clamp to the last frame.
pub fn frame_at_tick(holds: &[u8], tick: u32) -> usize {
    let mut acc = 0u32;
    for (i, &h) in holds.iter().enumerate() {
        acc += h as u32;
        if tick < acc {
            return i;
        }
    }
    holds.len().saturating_sub(1)
}

pub fn ticks_to_secs(ticks: u32, fps: u32) -> f64 {
    ticks as f64 / fps.max(1) as f64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Playhead {
    pub frame: usize,
    /// Tick within the animation that is currently showing.
    pub tick: u32,
    /// True when a non-looping animation has run past its end.
    pub finished: bool,
}

/// Maps elapsed wall-clock time since playback started to a frame. Uses elapsed time rather than
/// repaint count, so dropped or extra repaints never change the speed.
pub fn playhead(holds: &[u8], elapsed_secs: f64, fps: u32, looping: bool) -> Playhead {
    let total = total_ticks(holds);
    if total == 0 {
        return Playhead {
            frame: 0,
            tick: 0,
            finished: true,
        };
    }
    let raw = (elapsed_secs.max(0.0) * fps.max(1) as f64).floor() as u64;
    if looping {
        let tick = (raw % total as u64) as u32;
        Playhead {
            frame: frame_at_tick(holds, tick),
            tick,
            finished: false,
        }
    } else if raw >= total as u64 {
        Playhead {
            frame: holds.len() - 1,
            tick: total - 1,
            finished: true,
        }
    } else {
        Playhead {
            frame: frame_at_tick(holds, raw as u32),
            tick: raw as u32,
            finished: false,
        }
    }
}

/// Seconds until the next tick boundary, used to schedule the next repaint.
pub fn secs_until_next_tick(elapsed_secs: f64, fps: u32) -> f64 {
    let fps = fps.max(1) as f64;
    let next = (elapsed_secs.max(0.0) * fps).floor() + 1.0;
    (next / fps - elapsed_secs).max(0.0)
}

/// "frame 03/12 · 0.17s of 1.00s": position of the start of frame `idx` within the animation.
pub fn timecode(holds: &[u8], idx: usize, fps: u32) -> String {
    let count = holds.len();
    let at = ticks_to_secs(start_tick(holds, idx), fps);
    let total = ticks_to_secs(total_ticks(holds), fps);
    format!("frame {:02}/{:02} · {:.2}s of {:.2}s", idx + 1, count, at, total)
}

/// GIF frame delays in centiseconds (`hold / fps` seconds each). Rounding error is carried
/// forward so the total duration stays within half a centisecond of the true length instead of
/// drifting (24 fps is 4.1667 cs per tick; naive rounding would run 4% fast).
pub fn gif_delays_cs(holds: &[u8], fps: u32) -> Vec<u16> {
    let fps = fps.max(1) as f64;
    let mut out = Vec::with_capacity(holds.len());
    let mut ticks = 0u32;
    let mut emitted_cs = 0u64;
    for &h in holds {
        ticks += h as u32;
        let target_cs = (ticks as f64 / fps * 100.0).round() as u64;
        let delay = target_cs.saturating_sub(emitted_cs).max(1);
        emitted_cs += delay;
        out.push(delay.min(u16::MAX as u64) as u16);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_and_lookup() {
        let holds = [1, 3, 2];
        assert_eq!(total_ticks(&holds), 6);
        assert_eq!(start_tick(&holds, 0), 0);
        assert_eq!(start_tick(&holds, 1), 1);
        assert_eq!(start_tick(&holds, 2), 4);
        let by_tick: Vec<usize> = (0..6).map(|t| frame_at_tick(&holds, t)).collect();
        assert_eq!(by_tick, [0, 1, 1, 1, 2, 2]);
        assert_eq!(frame_at_tick(&holds, 99), 2);
    }

    #[test]
    fn playhead_respects_holds_and_loops() {
        let holds = [1, 3, 2];
        let fps = 12;
        let at = |s: f64, l: bool| playhead(&holds, s, fps, l);
        assert_eq!(at(0.0, true).frame, 0);
        assert_eq!(at(1.0 / 12.0 + 1e-6, true).frame, 1);
        assert_eq!(at(3.9 / 12.0, true).frame, 1);
        assert_eq!(at(4.0 / 12.0 + 1e-6, true).frame, 2);
        // 6 ticks per loop: 0.5s wraps back to frame 0.
        assert_eq!(at(0.5 + 1e-6, true).frame, 0);
        assert_eq!(at(5.5 + 1e-6, true).frame, 0);
        let end = at(0.5 + 1e-6, false);
        assert!(end.finished);
        assert_eq!(end.frame, 2);
    }

    #[test]
    fn playhead_is_independent_of_how_often_it_is_sampled() {
        let holds = [2, 1, 4, 1];
        let a = playhead(&holds, 0.75, 24, true);
        let b = playhead(&holds, 0.75, 24, true);
        assert_eq!(a, b);
        // 0.75 s at 24 fps = tick 18; total 8 => tick 2 => frame 1.
        assert_eq!(a.tick, 2);
        assert_eq!(a.frame, 1);
    }

    #[test]
    fn timecode_matches_spec_example() {
        let holds = [1u8; 12];
        assert_eq!(timecode(&holds, 2, 12), "frame 03/12 · 0.17s of 1.00s");
    }

    #[test]
    fn timecode_accounts_for_holds() {
        let holds = [2, 2, 1];
        assert_eq!(timecode(&holds, 2, 12), "frame 03/03 · 0.33s of 0.42s");
    }

    #[test]
    fn gif_delays_follow_hold_over_fps() {
        assert_eq!(gif_delays_cs(&[1, 2, 3], 12), vec![8, 17, 25]);
        // 12 fps ticks are 8.33 cs: carry keeps the 1s total exact.
        let d = gif_delays_cs(&[1; 12], 12);
        assert_eq!(d.iter().map(|&x| x as u32).sum::<u32>(), 100);
        let d = gif_delays_cs(&[1; 24], 24);
        assert_eq!(d.iter().map(|&x| x as u32).sum::<u32>(), 100);
        assert!(d.iter().all(|&x| x == 4 || x == 5));
    }

    #[test]
    fn next_tick_countdown() {
        let s = secs_until_next_tick(0.05, 12);
        assert!((s - (1.0 / 12.0 - 0.05)).abs() < 1e-9);
    }
}
