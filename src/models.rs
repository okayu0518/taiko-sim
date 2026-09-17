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

#[derive(Clone)]
pub struct Note {
    pub time_sec: f64,
    pub kind: NoteKind,
    pub judged: bool,
    pub hit: bool,
    pub big: bool,
    pub branch: Option<Branch>,
    pub scroll_speed: f64,
}

#[derive(Clone)]
pub struct Roll {
    pub start_sec: f64,
    pub end_sec: f64,
    pub big: bool,
    pub balloon: bool,
    pub branch: Option<Branch>,
    pub scroll_speed: f64,
}

#[derive(Clone)]
pub struct BranchPoint {
    pub time_sec: f64,
    pub requirement_type: char, // 'r' for roll, 'p' for precision
    pub expert_threshold: f64,
    pub master_threshold: f64,
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
    pub has_branch: bool,
    pub duration_sec: f64,
}

#[derive(Default, Clone)]
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
