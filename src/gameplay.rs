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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Note;

    fn note(time_sec: f64, kind: NoteKind) -> Note {
        Note { time_sec, kind, judged: false, hit: false, big: false, branch: None, scroll_speed: 1.0 }
    }

    fn roll(start_sec: f64, end_sec: f64, balloon: bool) -> Roll {
        Roll { start_sec, end_sec, big: false, balloon, branch: None, scroll_speed: 1.0 }
    }

    fn branch_point(time_sec: f64, requirement_type: char, expert: f64, master: f64) -> BranchPoint {
        BranchPoint { time_sec, requirement_type, expert_threshold: expert, master_threshold: master }
    }

    fn chart(notes: Vec<Note>, rolls: Vec<Roll>, branch_points: Vec<BranchPoint>) -> Chart {
        Chart {
            title: "test".to_string(),
            course: "Oni".to_string(),
            level: 1,
            offset_sec: 0.0,
            has_branch: !branch_points.is_empty(),
            notes,
            rolls,
            branch_points,
            duration_sec: 5.0,
        }
    }

    fn session_with_notes(notes: Vec<Note>) -> PlaySession {
        PlaySession::new(chart(notes, vec![], vec![]))
    }

    // --- hits ---

    #[test]
    fn ryo_hit_scores_and_extends_combo() {
        let mut s = session_with_notes(vec![note(1.0, NoteKind::Don)]);
        assert_eq!(s.hit(1.01, 0.0, NoteKind::Don), HitOutcome::Note { judge: Judge::Ryo, big: false });
        assert!(s.chart.notes[0].judged && s.chart.notes[0].hit);
        assert_eq!((s.score.score, s.score.combo, s.score.perfect), (300, 1, 1));
        assert_eq!(s.section.perfect, 1);
    }

    #[test]
    fn ka_hit_scores_half() {
        let mut s = session_with_notes(vec![note(1.0, NoteKind::Kat)]);
        assert_eq!(s.hit(0.95, 0.0, NoteKind::Kat), HitOutcome::Note { judge: Judge::Ka, big: false });
        assert_eq!((s.score.score, s.score.combo, s.score.good), (150, 1, 1));
    }

    #[test]
    fn big_note_hit_reports_big() {
        let mut big = note(1.0, NoteKind::Don);
        big.big = true;
        let mut s = session_with_notes(vec![big]);
        assert_eq!(s.hit(1.0, 0.0, NoteKind::Don), HitOutcome::Note { judge: Judge::Ryo, big: true });
    }

    #[test]
    fn wrong_color_input_is_ignored() {
        let mut s = session_with_notes(vec![note(1.0, NoteKind::Don)]);
        assert_eq!(s.hit(1.0, 0.0, NoteKind::Kat), HitOutcome::Ignored);
        assert!(!s.chart.notes[0].judged);
        assert_eq!(s.score, ScoreState::default());
    }

    #[test]
    fn out_of_window_input_is_ignored() {
        let mut s = session_with_notes(vec![note(1.0, NoteKind::Don)]);
        assert_eq!(s.hit(0.9, 0.0, NoteKind::Don), HitOutcome::Ignored);
        assert_eq!(s.hit(1.1, 0.0, NoteKind::Don), HitOutcome::Ignored);
        assert!(!s.chart.notes[0].judged);
    }

    #[test]
    fn nearest_matching_note_is_hit() {
        let mut s = session_with_notes(vec![
            note(1.0, NoteKind::Don),
            note(1.05, NoteKind::Kat),
            note(1.06, NoteKind::Don),
        ]);
        assert!(matches!(s.hit(1.04, 0.0, NoteKind::Don), HitOutcome::Note { .. }));
        let judged: Vec<_> = s.chart.notes.iter().map(|n| n.judged).collect();
        assert_eq!(judged, vec![false, false, true]);
    }

    #[test]
    fn note_cannot_be_hit_twice() {
        let mut s = session_with_notes(vec![note(1.0, NoteKind::Don)]);
        assert!(matches!(s.hit(1.0, 0.0, NoteKind::Don), HitOutcome::Note { .. }));
        assert_eq!(s.hit(1.0, 0.0, NoteKind::Don), HitOutcome::Ignored);
        assert_eq!(s.score.score, 300);
    }

    #[test]
    fn timing_offsets_shift_note_times() {
        let mut s = session_with_notes(vec![note(1.0, NoteKind::Don), note(2.0, NoteKind::Don)]);
        // User timing offset delays notes.
        assert_eq!(s.hit(1.0, 0.1, NoteKind::Don), HitOutcome::Ignored);
        assert!(matches!(s.hit(1.1, 0.1, NoteKind::Don), HitOutcome::Note { judge: Judge::Ryo, .. }));

        // Negative chart OFFSET delays notes as well.
        s.chart.offset_sec = -0.5;
        assert_close(s.game_time(2.0, 0.0), 2.5);
        assert!(matches!(s.hit(2.5, 0.0, NoteKind::Don), HitOutcome::Note { judge: Judge::Ryo, .. }));
    }

    // --- misses ---

    #[test]
    fn passed_notes_become_fuka_once() {
        let mut s = session_with_notes(vec![note(1.0, NoteKind::Don), note(2.0, NoteKind::Don)]);
        s.hit(1.0, 0.0, NoteKind::Don);
        assert_eq!(s.score.combo, 1);

        assert_eq!(s.update(2.07, 0.0), 0);
        assert_eq!(s.update(2.1, 0.0), 1);
        assert!(s.chart.notes[1].judged && !s.chart.notes[1].hit);
        assert_eq!((s.score.miss, s.score.combo, s.score.max_combo), (1, 0, 1));
        assert_eq!(s.section.miss, 1);

        assert_eq!(s.update(3.0, 0.0), 0);
        assert_eq!(s.score.miss, 1);
    }

    #[test]
    fn notes_on_other_branches_are_ignored() {
        let mut expert = note(1.0, NoteKind::Don);
        expert.branch = Some(Branch::Expert);
        let mut normal = note(1.0, NoteKind::Kat);
        normal.branch = Some(Branch::Normal);
        let mut s = session_with_notes(vec![expert, normal]);

        assert_eq!(s.hit(1.0, 0.0, NoteKind::Don), HitOutcome::Ignored);
        assert_eq!(s.update(5.0, 0.0), 1);
        assert!(!s.chart.notes[0].judged);
    }

    #[test]
    fn session_finishes_one_second_after_duration() {
        let s = session_with_notes(vec![]);
        assert!(!s.is_finished(6.0));
        assert!(s.is_finished(6.01));
    }

    // --- rolls ---

    #[test]
    fn roll_hits_take_priority_over_notes() {
        let mut s = PlaySession::new(chart(vec![note(1.5, NoteKind::Don)], vec![roll(1.0, 2.0, false)], vec![]));
        assert_eq!(s.hit(1.5, 0.0, NoteKind::Kat), HitOutcome::Roll);
        assert_eq!(s.hit(1.5, 0.0, NoteKind::Don), HitOutcome::Roll);
        assert!(!s.chart.notes[0].judged);
        assert_eq!((s.score.score, s.roll_hit_count, s.section.rolls), (2 * ROLL_HIT_SCORE, 2, 2));
        assert_eq!(s.score.combo, 0);
    }

    #[test]
    fn roll_counter_resets_after_roll_ends() {
        let mut s = PlaySession::new(chart(vec![], vec![roll(1.0, 2.0, false)], vec![]));
        s.hit(1.5, 0.0, NoteKind::Don);
        s.update(2.0, 0.0);
        assert_eq!(s.roll_hit_count, 1);
        assert!(s.active_roll(2.0, 0.0).is_some());

        s.update(2.01, 0.0);
        assert_eq!(s.roll_hit_count, 0);
        assert!(s.active_roll(2.01, 0.0).is_none());
    }

    #[test]
    fn balloon_accepts_only_don() {
        let mut s = PlaySession::new(chart(vec![note(1.5, NoteKind::Kat)], vec![roll(1.0, 2.0, true)], vec![]));
        assert_eq!(s.hit(1.5, 0.0, NoteKind::Don), HitOutcome::Roll);
        // Kat falls through to notes.
        assert!(matches!(s.hit(1.5, 0.0, NoteKind::Kat), HitOutcome::Note { judge: Judge::Ryo, .. }));
        assert_eq!(s.roll_hit_count, 1);
    }

    #[test]
    fn rolls_on_other_branches_are_ignored() {
        let mut r = roll(1.0, 2.0, false);
        r.branch = Some(Branch::Master);
        let mut s = PlaySession::new(chart(vec![], vec![r], vec![]));
        assert_eq!(s.hit(1.5, 0.0, NoteKind::Don), HitOutcome::Ignored);
        assert!(s.active_roll(1.5, 0.0).is_none());
    }

    // --- branches ---

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "expected {expected}, got {actual}");
    }

    #[test]
    fn section_accuracy_weights_ka_half() {
        assert_close(SectionStats::default().accuracy(), 0.0);
        let stats = SectionStats { perfect: 2, good: 1, miss: 1, rolls: 0 };
        assert_close(stats.accuracy(), 62.5);
    }

    fn session_with_section(bp: BranchPoint, section: SectionStats) -> PlaySession {
        let mut s = PlaySession::new(chart(vec![], vec![], vec![bp]));
        s.section = section;
        s
    }

    #[test]
    fn precision_branch_uses_thresholds_inclusively() {
        let cases = [(0, Branch::Normal), (1, Branch::Expert), (3, Branch::Master)];
        // perfect / (perfect + 1 miss): 0% / 50% / 75%
        for (perfect, expected) in cases {
            let section = SectionStats { perfect, miss: 1, ..Default::default() };
            let mut s = session_with_section(branch_point(2.0, 'p', 50.0, 75.0), section);
            s.check_branch_points(2.0, 0.0);
            assert_eq!(s.current_branch, expected, "perfect = {perfect}");
            assert_eq!(s.section, SectionStats::default());
        }
    }

    #[test]
    fn roll_branch_uses_roll_count() {
        let cases = [(4, Branch::Normal), (5, Branch::Expert), (10, Branch::Master)];
        for (rolls, expected) in cases {
            let section = SectionStats { rolls, ..Default::default() };
            let mut s = session_with_section(branch_point(2.0, 'r', 5.0, 10.0), section);
            s.check_branch_points(2.0, 0.0);
            assert_eq!(s.current_branch, expected, "rolls = {rolls}");
        }
    }

    #[test]
    fn branch_points_wait_for_their_time_and_offset() {
        let section = SectionStats { rolls: 10, ..Default::default() };
        let mut s = session_with_section(branch_point(2.0, 'r', 5.0, 10.0), section);
        s.check_branch_points(2.05, 0.1);
        assert_eq!(s.current_branch, Branch::Normal);
        assert_eq!(s.section.rolls, 10);

        s.check_branch_points(2.1, 0.1);
        assert_eq!(s.current_branch, Branch::Master);
    }

    #[test]
    fn section_point_resets_stats_without_changing_branch() {
        let mut s = PlaySession::new(chart(vec![], vec![], vec![
            branch_point(1.0, 'r', 1.0, 2.0),
            branch_point(2.0, 's', 0.0, 0.0),
            branch_point(3.0, 'r', 1.0, 2.0),
        ]));
        s.section.rolls = 5;
        s.check_branch_points(1.0, 0.0);
        assert_eq!(s.current_branch, Branch::Master);

        s.section.rolls = 5;
        s.check_branch_points(2.5, 0.0);
        assert_eq!(s.current_branch, Branch::Master);
        assert_eq!(s.section.rolls, 0);

        // Nothing hit since the section reset.
        s.check_branch_points(3.0, 0.0);
        assert_eq!(s.current_branch, Branch::Normal);
    }

    #[test]
    fn multiple_passed_branch_points_are_evaluated_in_order() {
        let mut s = PlaySession::new(chart(vec![], vec![], vec![
            branch_point(1.0, 'r', 1.0, 2.0),
            branch_point(2.0, 'r', 1.0, 2.0),
        ]));
        s.section.rolls = 5;
        s.check_branch_points(10.0, 0.0);
        // The second point sees the stats reset by the first.
        assert_eq!(s.current_branch, Branch::Normal);
    }

    #[test]
    fn judgements_feed_branch_evaluation() {
        let mut s = PlaySession::new(chart(
            vec![note(0.5, NoteKind::Don), note(1.0, NoteKind::Don)],
            vec![],
            vec![branch_point(2.0, 'p', 50.0, 90.0)],
        ));
        s.hit(0.5, 0.0, NoteKind::Don);
        s.update(1.5, 0.0);
        s.check_branch_points(2.0, 0.0);
        assert_eq!(s.current_branch, Branch::Expert);
    }
}
