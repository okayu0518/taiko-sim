/// Hit window for 良 (seconds, absolute delta). Approximates Oni difficulty.
pub const WINDOW_RYO: f64 = 0.025;
/// Hit window for 可 (seconds, absolute delta). Notes past this are judged 不可.
pub const WINDOW_KA: f64 = 0.075;

pub const SCORE_RYO: i32 = 300;
pub const SCORE_KA: i32 = 150;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Judge {
    Ryo,
    Ka,
    Fuka,
}

impl Judge {
    pub fn label(self) -> &'static str {
        match self {
            Judge::Ryo => "良",
            Judge::Ka => "可",
            Judge::Fuka => "不可",
        }
    }

    pub fn score(self) -> i32 {
        match self {
            Judge::Ryo => SCORE_RYO,
            Judge::Ka => SCORE_KA,
            Judge::Fuka => 0,
        }
    }

    pub fn keeps_combo(self) -> bool {
        !matches!(self, Judge::Fuka)
    }
}

/// Judge a hit by its timing delta (seconds). Returns `None` when the input
/// is outside the hit window and should be ignored.
pub fn judge_hit(delta: f64) -> Option<Judge> {
    let abs = delta.abs();
    if abs <= WINDOW_RYO {
        Some(Judge::Ryo)
    } else if abs <= WINDOW_KA {
        Some(Judge::Ka)
    } else {
        None
    }
}

/// Whether an unhit note has passed the hit window and must be judged 不可.
/// `elapsed` is `now - note_time` in seconds.
pub fn is_passed(elapsed: f64) -> bool {
    elapsed > WINDOW_KA
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ryo_boundary() {
        assert_eq!(judge_hit(0.0), Some(Judge::Ryo));
        assert_eq!(judge_hit(0.025), Some(Judge::Ryo));
        assert_eq!(judge_hit(-0.025), Some(Judge::Ryo));
    }

    #[test]
    fn ka_boundary() {
        assert_eq!(judge_hit(0.0251), Some(Judge::Ka));
        assert_eq!(judge_hit(-0.075), Some(Judge::Ka));
        assert_eq!(judge_hit(0.075), Some(Judge::Ka));
    }

    #[test]
    fn outside_window_is_ignored() {
        assert_eq!(judge_hit(0.0751), None);
        assert_eq!(judge_hit(-0.2), None);
    }

    #[test]
    fn passed_after_ka_window() {
        assert!(!is_passed(0.075));
        assert!(is_passed(0.0751));
        assert!(!is_passed(-1.0));
    }

    #[test]
    fn scoring_and_combo() {
        assert_eq!(Judge::Ryo.score(), 300);
        assert_eq!(Judge::Ka.score(), 150);
        assert_eq!(Judge::Fuka.score(), 0);
        assert!(Judge::Ka.keeps_combo());
        assert!(!Judge::Fuka.keeps_combo());
    }
}
