use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct SongEntry {
    pub song_id: String,
    pub title: String,
    pub tja_path: PathBuf,
    pub audio_path: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum NoteKind {
    Don,
    Kat,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Branch {
    Normal,
    Expert,
    Master,
}

/// Scroll state of a chart object. Visual speed is proportional to `bpm * scroll`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScrollSpeed {
    pub bpm: f64,
    pub scroll: f64,
}

impl ScrollSpeed {
    /// Beats scrolled per second.
    pub fn beats_per_sec(&self) -> f64 {
        self.bpm / 60.0 * self.scroll
    }
}

#[derive(Clone)]
pub struct Note {
    pub time_sec: f64,
    pub kind: NoteKind,
    pub judged: bool,
    pub hit: bool,
    pub big: bool,
    pub branch: Option<Branch>,
    pub speed: ScrollSpeed,
    /// Inside gogo time.
    pub gogo: bool,
}

#[derive(Clone)]
pub struct Roll {
    pub start_sec: f64,
    pub end_sec: f64,
    pub big: bool,
    pub balloon: bool,
    pub branch: Option<Branch>,
    pub head_speed: ScrollSpeed,
    pub tail_speed: ScrollSpeed,
}

/// Requirement type of `#BRANCHSTART`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BranchCondition {
    /// `r`: roll hit count.
    Roll,
    /// `p`: accuracy in percent.
    Precision,
    /// `s`: score.
    Score,
}

#[derive(Clone, PartialEq, Debug)]
pub struct BranchPoint {
    /// Chart time the branch is decided: the bar line one measure before the branch.
    pub judge_time_sec: f64,
    /// Chart time the branched section starts.
    pub start_time_sec: f64,
    /// `None` when the requirement could not be parsed; the branch then becomes Normal.
    pub condition: Option<BranchCondition>,
    pub expert_threshold: f64,
    pub master_threshold: f64,
}

/// Gogo time between `#GOGOSTART` and `#GOGOEND` (half-open, chart time).
#[derive(Clone, PartialEq, Debug)]
pub struct GogoRange {
    pub start_sec: f64,
    pub end_sec: f64,
    /// Branch the range was written in; `None` outside branches.
    pub branch: Option<Branch>,
}

/// `#LEVELHOLD`: once reached on its branch, later branch judgements keep the current branch.
#[derive(Clone, PartialEq, Debug)]
pub struct LevelHold {
    pub time_sec: f64,
    /// Branch the command was written in; `None` applies regardless of branch.
    pub branch: Option<Branch>,
}

#[derive(Clone)]
pub struct Chart {
    pub title: String,
    pub course: String,
    pub level: i32,
    pub offset_sec: f64,
    pub notes: Vec<Note>,
    pub rolls: Vec<Roll>,
    pub branch_points: Vec<BranchPoint>,
    /// Chart times of `#SECTION` (branch stats reset).
    pub section_resets: Vec<f64>,
    pub level_holds: Vec<LevelHold>,
    pub gogo_ranges: Vec<GogoRange>,
    pub has_branch: bool,
    pub duration_sec: f64,
}

#[derive(Default, Clone, PartialEq, Eq, Debug)]
pub struct ScoreState {
    pub score: i32,
    pub combo: i32,
    pub max_combo: i32,
    pub perfect: i32,
    pub good: i32,
    pub miss: i32,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AppConfig {
    pub timing_offset_ms: i32,
    pub se_volume: f32,
    pub music_volume: f32,
    pub keybind: KeybindConfig,
    pub vsync: bool,
    pub font_path: Option<String>,
    pub debug: bool,
    pub se_enabled: bool,
    /// Silence before the song starts, in seconds.
    pub lead_in_sec: f64,
}

impl AppConfig {
    pub const MAX_LEAD_IN_SEC: f64 = 10.0;

    /// Lead-in time clamped to `0..=MAX_LEAD_IN_SEC`.
    pub fn lead_in_sec(&self) -> f64 {
        self.lead_in_sec.clamp(0.0, Self::MAX_LEAD_IN_SEC)
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            timing_offset_ms: 0,
            se_volume: 0.9,
            music_volume: 1.0,
            keybind: KeybindConfig::default(),
            vsync: true,
            font_path: None,
            debug: false,
            se_enabled: true,
            lead_in_sec: 3.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct KeybindConfig {
    pub don_left: String,
    pub don_right: String,
    pub kat_left: String,
    pub kat_right: String,
}

impl Default for KeybindConfig {
    fn default() -> Self {
        Self {
            don_left: "F".to_string(),
            don_right: "J".to_string(),
            kat_left: "D".to_string(),
            kat_right: "K".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lead_in_defaults_when_missing_from_config() {
        let cfg: AppConfig = serde_json::from_str(r#"{"timing_offset_ms": 10}"#).unwrap();
        assert_eq!(cfg.lead_in_sec(), 3.0);
    }

    #[test]
    fn lead_in_is_clamped() {
        let with = |v: f64| AppConfig { lead_in_sec: v, ..Default::default() }.lead_in_sec();
        assert_eq!(with(-1.0), 0.0);
        assert_eq!(with(0.0), 0.0);
        assert_eq!(with(2.5), 2.5);
        assert_eq!(with(60.0), AppConfig::MAX_LEAD_IN_SEC);
    }
}
