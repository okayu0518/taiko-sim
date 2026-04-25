use std::path::PathBuf;

use macroquad::audio::{load_sound, play_sound, stop_sound, PlaySoundParams, Sound};
use macroquad::prelude::*;

use crate::models::{Chart, NoteKind, ScoreState, SongEntry};
use crate::repository::load_song_entries;
use crate::tja_parser::parse_tja;

const JUDGE_PERFECT: f64 = 0.050;
const JUDGE_GOOD: f64 = 0.100;
const MISS_WINDOW: f64 = 0.140;
const SCROLL_AHEAD_SEC: f64 = 2.0;

const TARGET_X: f32 = 260.0;
const LANE_Y: f32 = 340.0;

enum Scene {
    SongSelect,
    Play,
    Result,
}

pub struct TaikoSimApp {
    songs: Vec<SongEntry>,
    selected_idx: usize,
    scene: Scene,

    current_song: Option<SongEntry>,
    chart: Option<Chart>,
    score: ScoreState,
    start_time_sec: f64,
    current_sound: Option<Sound>,

    error_message: String,
}

impl TaikoSimApp {
    pub fn new(root_dir: PathBuf) -> Self {
        let data_dir = root_dir.join("data");
        let songs = load_song_entries(&data_dir);
        Self {
            songs,
            selected_idx: 0,
            scene: Scene::SongSelect,
            current_song: None,
            chart: None,
            score: ScoreState::default(),
            start_time_sec: 0.0,
            current_sound: None,
            error_message: String::new(),
        }
    }

    pub async fn run(&mut self) {
        loop {
            if is_key_pressed(KeyCode::Escape) {
                match self.scene {
                    Scene::SongSelect => break,
                    Scene::Play => self.to_result(),
                    Scene::Result => break,
                }
            }

            match self.scene {
                Scene::SongSelect => self.update_song_select().await,
                Scene::Play => self.update_play(),
                Scene::Result => self.update_result(),
            }

            match self.scene {
                Scene::SongSelect => self.draw_song_select(),
                Scene::Play => self.draw_play(),
                Scene::Result => self.draw_result(),
            }

            next_frame().await;
        }
    }

    async fn update_song_select(&mut self) {
        if self.songs.is_empty() {
            return;
        }

        if is_key_pressed(KeyCode::Up) {
            self.selected_idx = if self.selected_idx == 0 {
                self.songs.len().saturating_sub(1)
            } else {
                self.selected_idx - 1
            };
        }
        if is_key_pressed(KeyCode::Down) {
            self.selected_idx = (self.selected_idx + 1) % self.songs.len();
        }

        if is_key_pressed(KeyCode::Enter) {
            self.start_selected_song().await;
        }
    }

    fn update_play(&mut self) {
        if self.chart.is_none() {
            return;
        }

        if is_key_pressed(KeyCode::F) || is_key_pressed(KeyCode::J) {
            self.try_hit(NoteKind::Don);
        }
        if is_key_pressed(KeyCode::D) || is_key_pressed(KeyCode::K) {
            self.try_hit(NoteKind::Kat);
        }

        let now = self.current_time_sec();
        let mut miss_count = 0_i32;
        if let Some(chart) = self.chart.as_mut() {
            for note in &mut chart.notes {
                if note.judged {
                    continue;
                }
                let note_time = note.time_sec - chart.offset_sec;
                if now - note_time > MISS_WINDOW {
                    note.judged = true;
                    note.hit = false;
                    miss_count += 1;
                }
            }

            if now > chart.duration_sec + 1.0 {
                self.to_result();
            }
        }

        for _ in 0..miss_count {
            self.register_judge("Miss");
        }
    }

    fn update_result(&mut self) {
        if is_key_pressed(KeyCode::Enter) {
            self.scene = Scene::SongSelect;
        }
    }

    async fn start_selected_song(&mut self) {
        self.error_message.clear();
        let Some(song) = self.songs.get(self.selected_idx).cloned() else {
            return;
        };

        let (chart, wave) = match parse_tja(&song.tja_path, Some("Oni")) {
            Ok(v) => v,
            Err(e) => {
                self.error_message = format!("譜面読み込み失敗: {e}");
                return;
            }
        };

        let audio_path = resolve_audio_path(&song, wave.as_deref());
        let Some(audio) = audio_path else {
            self.error_message = "音源ファイルが見つかりません".to_string();
            return;
        };

        let audio_str = match audio.to_str() {
            Some(v) => v,
            None => {
                self.error_message = "音源パスが無効です".to_string();
                return;
            }
        };

        let sound = match load_sound(audio_str).await {
            Ok(s) => s,
            Err(e) => {
                self.error_message = format!("音源読み込み失敗: {e}");
                return;
            }
        };

        play_sound(
            &sound,
            PlaySoundParams {
                looped: false,
                volume: 1.0,
            },
        );

        self.current_song = Some(song);
        self.chart = Some(chart);
        self.score = ScoreState::default();
        self.start_time_sec = get_time();
        self.current_sound = Some(sound);
        self.scene = Scene::Play;
    }

    fn current_time_sec(&self) -> f64 {
        get_time() - self.start_time_sec
    }

    fn try_hit(&mut self, input_kind: NoteKind) {
        let now = self.current_time_sec();

        let Some((idx, delta)) = self.find_target_note(now) else {
            self.register_judge("Miss");
            return;
        };

        let Some(chart) = self.chart.as_mut() else {
            return;
        };

        if chart.notes[idx].kind != input_kind {
            self.register_judge("Miss");
            return;
        }

        if delta <= JUDGE_PERFECT {
            chart.notes[idx].judged = true;
            chart.notes[idx].hit = true;
            self.register_judge("Perfect");
        } else if delta <= JUDGE_GOOD {
            chart.notes[idx].judged = true;
            chart.notes[idx].hit = true;
            self.register_judge("Good");
        } else {
            self.register_judge("Miss");
        }
    }

    fn find_target_note(&self, now: f64) -> Option<(usize, f64)> {
        let chart = self.chart.as_ref()?;

        let mut best_idx: Option<usize> = None;
        let mut best_abs = f64::MAX;

        for (idx, note) in chart.notes.iter().enumerate() {
            if note.judged {
                continue;
            }

            let note_time = note.time_sec - chart.offset_sec;
            let abs_delta = (now - note_time).abs();
            if abs_delta <= JUDGE_GOOD && abs_delta < best_abs {
                best_abs = abs_delta;
                best_idx = Some(idx);
            }
        }

        best_idx.map(|i| (i, best_abs))
    }

    fn register_judge(&mut self, judge: &str) {
        self.score.last_judge = judge.to_string();
        match judge {
            "Perfect" => {
                self.score.perfect += 1;
                self.score.score += 300;
                self.score.combo += 1;
            }
            "Good" => {
                self.score.good += 1;
                self.score.score += 100;
                self.score.combo += 1;
            }
            _ => {
                self.score.miss += 1;
                self.score.combo = 0;
            }
        }
        if self.score.combo > self.score.max_combo {
            self.score.max_combo = self.score.combo;
        }
    }

    fn to_result(&mut self) {
        if let Some(sound) = &self.current_sound {
            stop_sound(sound);
        }
        self.current_sound = None;
        self.scene = Scene::Result;
    }

    fn draw_song_select(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));
        draw_text("Song Select", 60.0, 70.0, 56.0, WHITE);
        draw_text(
            "UP/DOWN: select  ENTER: play  ESC: quit",
            60.0,
            105.0,
            28.0,
            Color::from_rgba(170, 180, 200, 255),
        );

        if self.songs.is_empty() {
            draw_text("No songs found in data/songs", 60.0, 190.0, 34.0, RED);
            return;
        }

        let mut y = 170.0;
        for (idx, song) in self.songs.iter().enumerate() {
            let selected = idx == self.selected_idx;
            let color = if selected {
                Color::from_rgba(255, 230, 140, 255)
            } else {
                Color::from_rgba(230, 230, 230, 255)
            };
            let prefix = if selected { "> " } else { "  " };
            let label = format!("{}{} ({})", prefix, song.title, song.song_id);
            draw_text(&label, 80.0, y, 38.0, color);
            y += 44.0;
        }

        if !self.error_message.is_empty() {
            draw_text(
                &self.error_message,
                60.0,
                screen_height() - 40.0,
                30.0,
                Color::from_rgba(255, 120, 120, 255),
            );
        }
    }

    fn draw_play(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));

        let Some(chart) = self.chart.as_ref() else {
            return;
        };

        draw_rectangle(
            40.0,
            LANE_Y - 40.0,
            screen_width() - 80.0,
            90.0,
            Color::from_rgba(39, 45, 56, 255),
        );
        draw_line(
            TARGET_X,
            LANE_Y - 45.0,
            TARGET_X,
            LANE_Y + 55.0,
            4.0,
            Color::from_rgba(255, 230, 140, 255),
        );

        let now = self.current_time_sec();
        for note in &chart.notes {
            if note.judged && note.hit {
                continue;
            }

            let note_time = note.time_sec - chart.offset_sec;
            let diff = note_time - now;
            if !(-0.25..=SCROLL_AHEAD_SEC).contains(&diff) {
                continue;
            }

            let x = TARGET_X + ((diff / SCROLL_AHEAD_SEC) as f32) * 820.0;
            let color = match note.kind {
                NoteKind::Don => Color::from_rgba(255, 120, 100, 255),
                NoteKind::Kat => Color::from_rgba(100, 180, 255, 255),
            };
            draw_circle(x, LANE_Y, 20.0, color);
        }

        let title = format!("{} [{} Lv{}]", chart.title, chart.course, chart.level);
        draw_text(&title, 40.0, 56.0, 40.0, WHITE);

        let judge = format!("Judge: {}", self.score.last_judge);
        let score = format!("Score: {}", self.score.score);
        let combo = format!("Combo: {}", self.score.combo);
        draw_text(&judge, 40.0, 570.0, 36.0, WHITE);
        draw_text(&score, 320.0, 570.0, 36.0, WHITE);
        draw_text(&combo, 560.0, 570.0, 36.0, WHITE);
        draw_text(
            "F/J: Don  D/K: Kat  ESC: result",
            40.0,
            630.0,
            30.0,
            Color::from_rgba(170, 180, 200, 255),
        );
    }

    fn draw_result(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));
        draw_text("Result", 60.0, 70.0, 56.0, WHITE);

        let song_name = self
            .current_song
            .as_ref()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        draw_text(&song_name, 80.0, 130.0, 36.0, Color::from_rgba(200, 220, 255, 255));

        let lines = [
            format!("Score: {}", self.score.score),
            format!("Max Combo: {}", self.score.max_combo),
            format!("Perfect: {}", self.score.perfect),
            format!("Good: {}", self.score.good),
            format!("Miss: {}", self.score.miss),
        ];
        let mut y = 200.0;
        for line in lines {
            draw_text(&line, 80.0, y, 40.0, Color::from_rgba(230, 230, 230, 255));
            y += 56.0;
        }

        draw_text(
            "ENTER: song select  ESC: quit",
            80.0,
            640.0,
            30.0,
            Color::from_rgba(170, 180, 200, 255),
        );
    }
}

fn resolve_audio_path(song: &SongEntry, wave: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = &song.audio_path {
        if path.exists() {
            return Some(path.clone());
        }
    }

    if let Some(wave_name) = wave {
        let candidate = song.tja_path.parent()?.join(wave_name.trim());
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}
