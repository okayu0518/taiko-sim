use std::path::PathBuf;

#[derive(Clone)]
pub struct SongEntry {
    pub song_id: String,
    pub title: String,
    pub tja_path: PathBuf,
    pub audio_path: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Don,
    Kat,
}

#[derive(Clone)]
pub struct Note {
    pub time_sec: f64,
    pub kind: NoteKind,
    pub judged: bool,
    pub hit: bool,
}

#[derive(Clone)]
pub struct Chart {
    pub title: String,
    pub course: String,
    pub level: i32,
    pub offset_sec: f64,
    pub notes: Vec<Note>,
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
    pub last_judge: String,
}
