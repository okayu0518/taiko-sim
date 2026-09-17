use crate::judge::{is_passed, judge_hit, Judge, WINDOW_KA};
use crate::models::{Branch, BranchCondition, BranchPoint, Chart, NoteKind, Roll, ScoreState};

pub const ROLL_HIT_SCORE: i32 = 30;

/// Tolerance when comparing chart times produced by the parser.
const TIME_EPS: f64 = 1e-9;

/// Judgement counts since the last branch judgement or `#SECTION`.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct SectionStats {
    pub perfect: i32,
    pub good: i32,
    pub miss: i32,
    pub rolls: i32,
    pub score: i32,
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

/// Branch-related events in chart time order.
#[derive(Clone, Copy, PartialEq, Debug)]
enum BranchEvent {
    /// Decide the branch for `chart.branch_points[i]`.
    Judge(usize),
    /// `#SECTION`: reset the section stats.
    SectionReset,
}

/// Gameplay state for one play of a chart, independent of rendering, audio and input.
///
/// All `now` arguments are game time in seconds since the song started, and
/// `timing_offset_sec` is the user's timing correction.
pub struct PlaySession {
    pub chart: Chart,
    pub score: ScoreState,
    pub section: SectionStats,
    pub roll_hit_count: i32,
    /// Branch chosen for each branch point decided so far (same order as `chart.branch_points`).
    decided_branches: Vec<Branch>,
    /// Branch events sorted by chart time.
    branch_events: Vec<(f64, BranchEvent)>,
    next_branch_event: usize,
}

impl PlaySession {
    pub fn new(chart: Chart) -> Self {
        let mut branch_events: Vec<_> = chart.branch_points.iter().enumerate()
            .map(|(i, bp)| (bp.judge_time_sec, BranchEvent::Judge(i)))
            .chain(chart.section_resets.iter().map(|&t| (t, BranchEvent::SectionReset)))
            .collect();
        // Stable sort keeps judgements before resets at the same time.
        branch_events.sort_by(|a, b| a.0.total_cmp(&b.0));

        Self {
            chart,
            score: ScoreState::default(),
            section: SectionStats::default(),
            roll_hit_count: 0,
            decided_branches: Vec::new(),
            branch_events,
            next_branch_event: 0,
        }
    }

    /// Convert a chart time into game time.
    pub fn game_time(&self, chart_sec: f64, timing_offset_sec: f64) -> f64 {
        chart_sec - self.chart.offset_sec + timing_offset_sec
    }

    /// The most recently decided branch (shown in the HUD).
    pub fn current_branch(&self) -> Branch {
        self.decided_branches.last().copied().unwrap_or(Branch::Normal)
    }

    /// Branch played at `chart_sec`. Sections not decided yet follow the current branch.
    pub fn branch_at(&self, chart_sec: f64) -> Branch {
        self.chart.branch_points.iter()
            .rposition(|bp| bp.start_time_sec <= chart_sec + TIME_EPS)
            .map_or(Branch::Normal, |i| self.decided_branches.get(i).copied().unwrap_or(self.current_branch()))
    }

    /// Whether an object on `branch` at `chart_sec` belongs to the branch being played.
    pub fn is_active(&self, branch: Option<Branch>, chart_sec: f64) -> bool {
        branch.map_or(true, |b| b == self.branch_at(chart_sec))
    }

    pub fn is_finished(&self, now: f64) -> bool {
        now > self.chart.duration_sec + 1.0
    }

    /// Process branch judgements and section resets reached by `now`.
    pub fn check_branch_points(&mut self, now: f64, timing_offset_sec: f64) {
        while let Some(&(time, event)) = self.branch_events.get(self.next_branch_event) {
            if now < self.game_time(time, timing_offset_sec) {
                return;
            }
            match event {
                BranchEvent::Judge(i) => {
                    let branch = self.evaluate_branch(&self.chart.branch_points[i]);
                    self.decided_branches.push(branch);
                    self.section = SectionStats::default();
                }
                BranchEvent::SectionReset => self.section = SectionStats::default(),
            }
            self.next_branch_event += 1;
        }
    }

    fn evaluate_branch(&self, bp: &BranchPoint) -> Branch {
        let value = match bp.condition {
            Some(BranchCondition::Roll) => self.section.rolls as f64,
            Some(BranchCondition::Precision) => self.section.accuracy(),
            Some(BranchCondition::Score) => self.section.score as f64,
            None => return Branch::Normal,
        };
        if value >= bp.master_threshold {
            Branch::Master
        } else if value >= bp.expert_threshold {
            Branch::Expert
        } else {
            Branch::Normal
        }
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

        let mut missed = 0;
        for idx in 0..self.chart.notes.len() {
            let note = &self.chart.notes[idx];
            if note.judged || !self.is_active(note.branch, note.time_sec) {
                continue;
            }
            if is_passed(now - self.game_time(note.time_sec, timing_offset_sec)) {
                let note = &mut self.chart.notes[idx];
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
        self.chart.rolls.iter().find(|r| self.is_roll_active_at(r, now, timing_offset_sec))
    }

    fn is_roll_active_at(&self, roll: &Roll, now: f64, timing_offset_sec: f64) -> bool {
        self.is_active(roll.branch, roll.start_sec)
            && now >= self.game_time(roll.start_sec, timing_offset_sec)
            && now <= self.game_time(roll.end_sec, timing_offset_sec)
    }

    fn find_nearest_note(&self, now: f64, timing_offset_sec: f64, kind: NoteKind) -> Option<(usize, f64)> {
        let mut best = None;
        let mut best_abs = f64::MAX;
        for (idx, note) in self.chart.notes.iter().enumerate() {
            if note.judged || note.kind != kind || !self.is_active(note.branch, note.time_sec) {
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
            self.is_roll_active_at(roll, now, timing_offset_sec) && (!roll.balloon || kind == NoteKind::Don)
        });
        if accepted {
            self.score.score += ROLL_HIT_SCORE;
            self.roll_hit_count += 1;
            self.section.rolls += 1;
            self.section.score += ROLL_HIT_SCORE;
        }
        accepted
    }

    fn register_judge(&mut self, judge: Judge) {
        self.score.score += judge.score();
        self.section.score += judge.score();
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
    use crate::models::{Note, ScrollSpeed};

    const SPEED: ScrollSpeed = ScrollSpeed { bpm: 120.0, scroll: 1.0 };

    fn note(time_sec: f64, kind: NoteKind) -> Note {
        Note { time_sec, kind, judged: false, hit: false, big: false, branch: None, speed: SPEED }
    }

    fn roll(start_sec: f64, end_sec: f64, balloon: bool) -> Roll {
        Roll { start_sec, end_sec, big: false, balloon, branch: None, head_speed: SPEED, tail_speed: SPEED }
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
            section_resets: vec![],
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

    fn branch_point(judge_time_sec: f64, start_time_sec: f64, condition: BranchCondition, expert: f64, master: f64) -> BranchPoint {
        BranchPoint { judge_time_sec, start_time_sec, condition: Some(condition), expert_threshold: expert, master_threshold: master }
    }

    fn branched_note(time_sec: f64, branch: Branch) -> Note {
        Note { branch: Some(branch), ..note(time_sec, NoteKind::Don) }
    }

    fn session_with_branches(notes: Vec<Note>, branch_points: Vec<BranchPoint>, section_resets: Vec<f64>) -> PlaySession {
        let mut c = chart(notes, vec![], branch_points);
        c.section_resets = section_resets;
        PlaySession::new(c)
    }

    #[test]
    fn section_accuracy_weights_ka_half() {
        assert_close(SectionStats::default().accuracy(), 0.0);
        let stats = SectionStats { perfect: 2, good: 1, miss: 1, ..Default::default() };
        assert_close(stats.accuracy(), 62.5);
    }

    #[test]
    fn precision_branch_uses_thresholds_inclusively() {
        let cases = [(0, Branch::Normal), (1, Branch::Expert), (3, Branch::Master)];
        // perfect / (perfect + 1 miss): 0% / 50% / 75%
        for (perfect, expected) in cases {
            let mut s = session_with_branches(vec![], vec![branch_point(2.0, 4.0, BranchCondition::Precision, 50.0, 75.0)], vec![]);
            s.section = SectionStats { perfect, miss: 1, ..Default::default() };
            s.check_branch_points(2.0, 0.0);
            assert_eq!(s.current_branch(), expected, "perfect = {perfect}");
            assert_eq!(s.section, SectionStats::default());
        }
    }

    #[test]
    fn roll_branch_uses_roll_count() {
        let cases = [(4, Branch::Normal), (5, Branch::Expert), (10, Branch::Master)];
        for (rolls, expected) in cases {
            let mut s = session_with_branches(vec![], vec![branch_point(2.0, 4.0, BranchCondition::Roll, 5.0, 10.0)], vec![]);
            s.section.rolls = rolls;
            s.check_branch_points(2.0, 0.0);
            assert_eq!(s.current_branch(), expected, "rolls = {rolls}");
        }
    }

    #[test]
    fn score_branch_uses_section_score() {
        let mut s = session_with_branches(
            vec![note(0.5, NoteKind::Don), note(1.0, NoteKind::Don)],
            vec![branch_point(2.0, 4.0, BranchCondition::Score, 500.0, 600.0)],
            vec![],
        );
        s.hit(0.5, 0.0, NoteKind::Don);
        s.hit(1.05, 0.0, NoteKind::Don);
        assert_eq!(s.section.score, 450);
        s.check_branch_points(2.0, 0.0);
        assert_eq!(s.current_branch(), Branch::Normal);
    }

    #[test]
    fn unparsed_condition_selects_normal() {
        let mut bp = branch_point(2.0, 4.0, BranchCondition::Roll, 0.0, 0.0);
        bp.condition = None;
        let mut s = session_with_branches(vec![], vec![bp], vec![]);
        s.check_branch_points(2.0, 0.0);
        assert_eq!(s.current_branch(), Branch::Normal);
    }

    #[test]
    fn branch_is_decided_at_judge_time_with_offset() {
        let mut s = session_with_branches(vec![], vec![branch_point(2.0, 4.0, BranchCondition::Roll, 5.0, 10.0)], vec![]);
        s.section.rolls = 10;
        s.check_branch_points(2.05, 0.1);
        assert_eq!(s.current_branch(), Branch::Normal);
        assert_eq!(s.section.rolls, 10);

        s.check_branch_points(2.1, 0.1);
        assert_eq!(s.current_branch(), Branch::Master);
    }

    #[test]
    fn upcoming_branch_notes_become_active_at_judge_time() {
        let mut s = session_with_branches(
            vec![branched_note(4.0, Branch::Normal), branched_note(4.0, Branch::Master)],
            vec![branch_point(2.0, 4.0, BranchCondition::Roll, 1.0, 2.0)],
            vec![],
        );
        // Before the judgement, upcoming notes follow the current branch.
        assert!(s.is_active(Some(Branch::Normal), 4.0));
        assert!(!s.is_active(Some(Branch::Master), 4.0));

        s.section.rolls = 5;
        s.check_branch_points(2.0, 0.0);
        assert!(!s.is_active(Some(Branch::Normal), 4.0));
        assert!(s.is_active(Some(Branch::Master), 4.0));
        assert!(matches!(s.hit(4.0, 0.0, NoteKind::Don), HitOutcome::Note { .. }));
        assert!(s.chart.notes[1].judged && !s.chart.notes[0].judged);
    }

    #[test]
    fn switching_branch_does_not_miss_past_notes_of_new_branch() {
        let mut s = session_with_branches(
            vec![
                // Section 1 (4.0..8.0), played on Normal.
                branched_note(4.0, Branch::Normal),
                branched_note(4.0, Branch::Master),
                // Section 2 (10.0..), played on Master.
                branched_note(10.0, Branch::Normal),
                branched_note(10.0, Branch::Master),
            ],
            vec![
                branch_point(2.0, 4.0, BranchCondition::Roll, 1.0, 2.0),
                branch_point(8.0, 10.0, BranchCondition::Roll, 1.0, 2.0),
            ],
            vec![],
        );
        s.check_branch_points(2.0, 0.0);
        s.hit(4.0, 0.0, NoteKind::Don);

        s.section.rolls = 5;
        s.check_branch_points(8.0, 0.0);
        assert_eq!(s.current_branch(), Branch::Master);
        assert_eq!(s.branch_at(4.0), Branch::Normal);
        assert_eq!(s.branch_at(10.0), Branch::Master);

        assert_eq!(s.update(9.0, 0.0), 0);
        assert_eq!(s.score.miss, 0);
        assert!(!s.chart.notes[1].judged);

        assert_eq!(s.update(11.0, 0.0), 1);
        assert!(s.chart.notes[3].judged && !s.chart.notes[2].judged);
    }

    #[test]
    fn section_reset_clears_stats_without_changing_branch() {
        let mut s = session_with_branches(
            vec![],
            vec![
                branch_point(1.0, 1.5, BranchCondition::Roll, 1.0, 2.0),
                branch_point(3.0, 3.5, BranchCondition::Roll, 1.0, 2.0),
            ],
            vec![2.0],
        );
        s.section.rolls = 5;
        s.check_branch_points(1.0, 0.0);
        assert_eq!(s.current_branch(), Branch::Master);

        s.section.rolls = 5;
        s.check_branch_points(2.5, 0.0);
        assert_eq!(s.current_branch(), Branch::Master);
        assert_eq!(s.section.rolls, 0);

        // Nothing hit since the section reset.
        s.check_branch_points(3.0, 0.0);
        assert_eq!(s.current_branch(), Branch::Normal);
    }

    #[test]
    fn multiple_passed_events_are_processed_in_time_order() {
        // Section reset listed separately but occurs before the second judgement.
        let mut s = session_with_branches(
            vec![],
            vec![
                branch_point(1.0, 2.0, BranchCondition::Roll, 1.0, 2.0),
                branch_point(3.0, 4.0, BranchCondition::Roll, 1.0, 2.0),
            ],
            vec![0.5],
        );
        s.section.rolls = 5;
        s.check_branch_points(10.0, 0.0);
        // The reset at 0.5 cleared the stats before the first judgement.
        assert_eq!(s.decided_branches, vec![Branch::Normal, Branch::Normal]);
    }

    #[test]
    fn judgements_feed_branch_evaluation() {
        let mut s = session_with_branches(
            vec![note(0.5, NoteKind::Don), note(1.0, NoteKind::Don)],
            vec![branch_point(2.0, 4.0, BranchCondition::Precision, 50.0, 90.0)],
            vec![],
        );
        s.hit(0.5, 0.0, NoteKind::Don);
        s.update(1.5, 0.0);
        s.check_branch_points(2.0, 0.0);
        assert_eq!(s.current_branch(), Branch::Expert);
    }
}
