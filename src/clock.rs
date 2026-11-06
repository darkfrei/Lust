// the clock of a game loop: how many ticks to run in a frame, and with which dt.
//   variable  one tick per frame with the time since the last one (dt floats). smooth, and
//             what most games that need not replay exactly want. a frame that took too
//             long (the window was dragged, a breakpoint) gives at most max_dt, so the game
//             does not jump
//   fixed     the world moves in equal steps; a frame runs as many as have piled up, at
//             most max_steps. for physics that must stay stable and for exact replays.
//             after a slow frame the rest is dropped, so one slow frame does not make the
//             next one slower too (it would otherwise catch up for ever)

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Variable { max_dt: f64 },
    Fixed { step: f64, max_steps: u32 },
}

#[derive(Clone, Debug)]
pub struct Clock {
    mode: Mode,
    acc: f64,
    // the time the clock gave up on: too long frames, steps not run
    pub dropped: f64,
}

impl Clock {
    pub fn variable(max_dt: f64) -> Clock {
        Clock { mode: Mode::Variable { max_dt: max_dt.max(1e-6) }, acc: 0.0, dropped: 0.0 }
    }

    pub fn fixed(step: f64, max_steps: u32) -> Clock {
        Clock { mode: Mode::Fixed { step: step.max(1e-6), max_steps: max_steps.max(1) }, acc: 0.0, dropped: 0.0 }
    }

    // a frame took `elapsed` seconds: run this many ticks, each with this dt
    pub fn advance(&mut self, elapsed: f64) -> (u32, f64) {
        let elapsed = if elapsed.is_finite() { elapsed.max(0.0) } else { 0.0 };
        match self.mode {
            Mode::Variable { max_dt } => {
                if elapsed <= 0.0 {
                    return (0, 0.0);
                }
                if elapsed > max_dt {
                    self.dropped += elapsed - max_dt;
                }
                (1, elapsed.min(max_dt))
            }
            Mode::Fixed { step, max_steps } => {
                self.acc += elapsed;
                let mut n = (self.acc / step).floor() as u64;
                if n > max_steps as u64 {
                    let over = (n - max_steps as u64) as f64 * step;
                    self.dropped += over;
                    self.acc -= over;
                    n = max_steps as u64;
                }
                self.acc -= n as f64 * step;
                (n as u32, step)
            }
        }
    }

    // with a fixed step: how far the frame is between two steps (0..1), to draw between the
    // last two states. with a variable one there is nothing between: 0
    pub fn alpha(&self) -> f64 {
        match self.mode {
            Mode::Fixed { step, .. } => self.acc / step,
            Mode::Variable { .. } => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variable_clock_gives_the_frame_time_but_not_a_jump() {
        let mut c = Clock::variable(0.1);
        assert_eq!(c.advance(0.016), (1, 0.016));
        assert_eq!(c.advance(0.0), (0, 0.0));
        assert_eq!(c.advance(2.0), (1, 0.1));
        assert!((c.dropped - 1.9).abs() < 1e-9);
    }

    #[test]
    fn a_fixed_clock_drops_what_it_cannot_catch_up() {
        let mut c = Clock::fixed(0.01, 5);
        assert_eq!(c.advance(0.025).0, 2);
        assert!((c.alpha() - 0.5).abs() < 1e-9);
        assert_eq!(c.advance(1.0).0, 5);
        assert!(c.dropped > 0.9);
        assert_eq!(c.advance(0.0).0, 0);
    }
}
