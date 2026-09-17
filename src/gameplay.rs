use crate::judge::{is_passed, judge_hit, Judge, WINDOW_KA};
use crate::models::{Branch, BranchPoint, Chart, NoteKind, Roll, ScoreState};

pub const ROLL_HIT_SCORE: i32 = 30;

/// Judgement counts since the last branch point.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct SectionStats {
    pub perfect: i32,
    pub good: i32,
    pub miss: i32,
    pub rolls: i32,
}

impl SectionStats {
    /// Accuracy in percent (良 = 1, 可 = 0.5). Returns 0 when nothing was judged.
    pub fn accuracy(&self) -> f64 {
        let total = self.perfect + self.good + self.miss;
        if total == 0 {
            return 0.0;
        }
        (self.perfect as f64 + self.good as f64 * 0.5) / total as f64 * 100.0
    }
}

/// Result of a single drum input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HitOutcome {
    /// A note was hit.
    Note { judge: Judge, big: bool },
    /// The input counted as a roll/balloon hit.
    Roll,
    /// No note or roll accepted the input.
    Ignored,
}

/// Gameplay state for one play of a chart, independent of rendering, audio and input.
///
/// All `now` arguments are game time in seconds since the song started, and
/// `timing_offset_sec` is the user's timing correction.
pub struct PlaySession {
    pub chart: Chart,
    pub score: ScoreState,
    pub current_branch: Branch,
    pub section: SectionStats,
    pub roll_hit_count: i32,
    branch_points_handled: usize,
}

impl PlaySession {
    pub fn new(chart: Chart) -> Self {
        Self {
            chart,
            score: ScoreState::default(),
            current_branch: Branch::Normal,
            section: SectionStats::default(),
            roll_hit_count: 0,
            branch_points_handled: 0,
        }
    }

    /// Convert a chart time into game time.
    pub fn game_time(&self, chart_sec: f64, timing_offset_sec: f64) -> f64 {
        chart_sec - self.chart.offset_sec + timing_offset_sec
    }

    /// Whether an object on `branch` is playable on the current branch.
    pub fn is_on_current_branch(&self, branch: Option<Branch>) -> bool {
        branch.map_or(true, |b| b == self.current_branch)
    }

    pub fn is_finished(&self, now: f64) -> bool {
        now > self.chart.duration_sec + 1.0
    }

    /// Evaluate every branch point reached by `now`.
    pub fn check_branch_points(&mut self, now: f64, timing_offset_sec: f64) {
        while let Some(bp) = self.chart.branch_points.get(self.branch_points_handled) {
            if now < self.game_time(bp.time_sec, timing_offset_sec) {
                return;
            }
            let bp = bp.clone();
            self.evaluate_branch(&bp);
            self.branch_points_handled += 1;
        }
    }

    fn evaluate_branch(&mut self, bp: &BranchPoint) {
        let value = match bp.requirement_type {
            'r' => Some(self.section.rolls as f64),
            'p' => Some(self.section.accuracy()),
            // 's' (section reset) and unknown types only reset the stats.
            _ => None,
        };
        if let Some(v) = value {
            self.current_branch = if v >= bp.master_threshold {
                Branch::Master
            } else if v >= bp.expert_threshold {
                Branch::Expert
            } else {
                Branch::Normal
            };
        }
        self.section = SectionStats::default();
    }

    /// Handle a drum input. Rolls take priority over notes; wrong-color or
    /// out-of-window inputs are ignored.
    pub fn hit(&mut self, now: f64, timing_offset_sec: f64, kind: NoteKind) -> HitOutcome {
        if self.hit_roll_if_active(now, timing_offset_sec, kind) {
            return HitOutcome::Roll;
        }

        let Some((idx, delta)) = self.find_nearest_note(now, timing_offset_sec, kind) else {
            return HitOutcome::Ignored;
        };
        let Some(judge) = judge_hit(delta) else {
            return HitOutcome::Ignored;
        };
        let note = &mut self.chart.notes[idx];
        note.judged = true;
        note.hit = true;
        let big = note.big;
        self.register_judge(judge);
        HitOutcome::Note { judge, big }
    }

    /// Judge notes that passed the hit window as 不可 and reset the roll counter
    /// once no roll is active. Returns the number of newly missed notes.
    pub fn update(&mut self, now: f64, timing_offset_sec: f64) -> usize {
        if self.active_roll(now, timing_offset_sec).is_none() {
            self.roll_hit_count = 0;
        }

        let offset = self.chart.offset_sec;
        let cur_branch = self.current_branch;
        let mut missed = 0;
        for note in &mut self.chart.notes {
            if note.judged || note.branch.is_some_and(|b| b != cur_branch) {
                continue;
            }
            let note_time = note.time_sec - offset + timing_offset_sec;
            if is_passed(now - note_time) {
                note.judged = true;
                note.hit = false;
                missed += 1;
            }
        }
        for _ in 0..missed {
            self.register_judge(Judge::Fuka);
        }
        missed
    }

    pub fn active_roll(&self, now: f64, timing_offset_sec: f64) -> Option<&Roll> {
        self.chart.rolls.iter().find(|r| {
            self.is_on_current_branch(r.branch)
                && now >= self.game_time(r.start_sec, timing_offset_sec)
                && now <= self.game_time(r.end_sec, timing_offset_sec)
        })
    }

    fn find_nearest_note(&self, now: f64, timing_offset_sec: f64, kind: NoteKind) -> Option<(usize, f64)> {
        let mut best = None;
        let mut best_abs = f64::MAX;
        for (idx, note) in self.chart.notes.iter().enumerate() {
            if note.judged || note.kind != kind || !self.is_on_current_branch(note.branch) {
                continue;
            }
            let abs_delta = (now - self.game_time(note.time_sec, timing_offset_sec)).abs();
            if abs_delta <= WINDOW_KA && abs_delta < best_abs {
                best_abs = abs_delta;
                best = Some(idx);
            }
        }
        best.map(|i| (i, best_abs))
    }

    fn hit_roll_if_active(&mut self, now: f64, timing_offset_sec: f64, kind: NoteKind) -> bool {
        // Balloons only accept Don; a Kat input falls through to the next roll or notes.
        let accepted = self.chart.rolls.iter().any(|roll| {
            self.is_on_current_branch(roll.branch)
                && now >= self.game_time(roll.start_sec, timing_offset_sec)
                && now <= self.game_time(roll.end_sec, timing_offset_sec)
                && (!roll.balloon || kind == NoteKind::Don)
        });
        if accepted {
            self.score.score += ROLL_HIT_SCORE;
            self.roll_hit_count += 1;
            self.section.rolls += 1;
        }
        accepted
    }

    fn register_judge(&mut self, judge: Judge) {
        self.score.score += judge.score();
        match judge {
            Judge::Ryo => { self.score.perfect += 1; self.section.perfect += 1; }
            Judge::Ka => { self.score.good += 1; self.section.good += 1; }
            Judge::Fuka => { self.score.miss += 1; self.section.miss += 1; }
        }
        if judge.keeps_combo() { self.score.combo += 1; } else { self.score.combo = 0; }
        self.score.max_combo = self.score.max_combo.max(self.score.combo);
    }
}
