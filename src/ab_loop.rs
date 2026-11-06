//! A-B loop points. One key walks through: set A, set B, clear.

/// Points closer together than this can't form a useful loop.
const MIN_LOOP_SECS: f64 = 0.05;

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct AbLoop {
    pub a: Option<f64>,
    pub b: Option<f64>,
}

impl AbLoop {
    /// Advances the loop at playback position `pos`. A point set after the
    /// viewer seeked backwards becomes A, so the loop is always forwards.
    pub fn press(&mut self, pos: f64) {
        match (self.a, self.b) {
            (None, _) => self.a = Some(pos),
            (Some(a), None) if (pos - a).abs() < MIN_LOOP_SECS => {}
            (Some(a), None) => {
                self.a = Some(a.min(pos));
                self.b = Some(a.max(pos));
            }
            (Some(_), Some(_)) => *self = Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_through_a_then_b_then_clear() {
        let mut ab = AbLoop::default();
        ab.press(10.0);
        assert_eq!(ab, AbLoop { a: Some(10.0), b: None });
        ab.press(20.0);
        assert_eq!(ab, AbLoop { a: Some(10.0), b: Some(20.0) });
        ab.press(30.0);
        assert_eq!(ab, AbLoop::default());
    }

    #[test]
    fn second_point_before_a_swaps_them() {
        let mut ab = AbLoop::default();
        ab.press(20.0);
        ab.press(10.0);
        assert_eq!(ab, AbLoop { a: Some(10.0), b: Some(20.0) });
    }

    #[test]
    fn second_point_on_top_of_a_is_ignored() {
        let mut ab = AbLoop::default();
        ab.press(10.0);
        ab.press(10.01);
        assert_eq!(ab, AbLoop { a: Some(10.0), b: None });
    }
}
