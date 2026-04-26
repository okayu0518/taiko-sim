use std::path::PathBuf;
use std::{fs::File, io::BufReader};
use std::{sync::Arc, time::Duration};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use macroquad::prelude::*;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use serde::{Deserialize, Serialize};

use crate::models::{Chart, NoteKind, Roll, ScoreState, SongEntry};
use crate::repository::load_song_entries;
use crate::tja_parser::parse_tja;

const JUDGE_PERFECT: f64 = 0.050;
const JUDGE_GOOD: f64 = 0.100;
const MISS_WINDOW: f64 = 0.140;
const SCROLL_AHEAD_SEC: f64 = 2.0;
const SCROLL_BEHIND_SEC: f64 = 0.25;
const ROLL_HIT_SCORE: i32 = 30;

const BASE_W: f32 = 1280.0;
const BASE_H: f32 = 720.0;
const TARGET_X: f32 = 290.0;
const LANE_Y: f32 = 350.0;
const NOTE_RADIUS: f32 = 24.0;
const JUDGE_RING_RADIUS: f32 = 26.0;
const LANE_X: f32 = 140.0;
const LANE_W: f32 = 1110.0;
const LANE_H: f32 = 120.0;
const SCROLL_PIXELS: f32 = 860.0;

enum Scene {
    SongSelect,
    Play,
    Result,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct AppConfig {
    timing_offset_ms: i32,
    se_volume: f32,
    music_volume: f32,
    keybind: KeybindConfig,
    vsync: bool,
    font_path: Option<String>,
    debug: bool,
    se_enabled: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct KeybindConfig {
    don_left: String,
    don_right: String,
    kat_left: String,
    kat_right: String,
}

#[derive(Clone)]
struct SeData {
    channels: u16,
    sample_rate: u32,
    samples: Arc<[f32]>,
}

struct SeLatencyStats {
    last_us: AtomicU64,
    max_us: AtomicU64,
    total_us: AtomicU64,
    count: AtomicU64,
}

impl SeLatencyStats {
    fn new() -> Self {
        Self {
            last_us: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            total_us: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    fn record(&self, us: u64) {
        self.last_us.store(us, Ordering::Relaxed);
        self.total_us.fetch_add(us, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        let mut current = self.max_us.load(Ordering::Relaxed);
        while us > current {
            match self.max_us.compare_exchange(current, us, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(v) => current = v,
            }
        }
    }

    fn snapshot_ms(&self) -> (f32, f32, f32, u64) {
        let last = self.last_us.load(Ordering::Relaxed);
        let max = self.max_us.load(Ordering::Relaxed);
        let total = self.total_us.load(Ordering::Relaxed);
        let count = self.count.load(Ordering::Relaxed);
        let avg = if count == 0 { 0 } else { total / count };
        (
            last as f32 / 1000.0,
            avg as f32 / 1000.0,
            max as f32 / 1000.0,
            count,
        )
    }
}

struct SeSource {
    data: Arc<[f32]>,
    idx: usize,
    channels: u16,
    sample_rate: u32,
    gain: f32,
    triggered_at: Instant,
    recorded: bool,
    latency_stats: Arc<SeLatencyStats>,
}

impl Iterator for SeSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.recorded {
            let elapsed_us = self.triggered_at.elapsed().as_micros() as u64;
            self.latency_stats.record(elapsed_us);
            self.recorded = true;
        }

        let out = self.data.get(self.idx).copied().map(|v| v * self.gain);
        if out.is_some() {
            self.idx += 1;
        }
        out
    }
}

impl Source for SeSource {
    fn current_frame_len(&self) -> Option<usize> {
        let remain = self.data.len().saturating_sub(self.idx);
        Some(remain / self.channels.max(1) as usize)
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let frame_count = self.data.len() as f64 / self.channels.max(1) as f64;
        Some(Duration::from_secs_f64(frame_count / self.sample_rate.max(1) as f64))
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
        }
    }
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

struct Keybinds {
    don_left: KeyCode,
    don_right: KeyCode,
    kat_left: KeyCode,
    kat_right: KeyCode,
}

impl Keybinds {
    fn from_config(cfg: &KeybindConfig) -> Self {
        Self {
            don_left: parse_keycode(&cfg.don_left).unwrap_or(KeyCode::F),
            don_right: parse_keycode(&cfg.don_right).unwrap_or(KeyCode::J),
            kat_left: parse_keycode(&cfg.kat_left).unwrap_or(KeyCode::D),
            kat_right: parse_keycode(&cfg.kat_right).unwrap_or(KeyCode::K),
        }
    }

    fn don_pressed(&self) -> bool {
        is_key_pressed(self.don_left) || is_key_pressed(self.don_right)
    }

    fn kat_pressed(&self) -> bool {
        is_key_pressed(self.kat_left) || is_key_pressed(self.kat_right)
    }
}

pub struct TaikoSimApp {
    root_dir: PathBuf,
    songs: Vec<SongEntry>,
    selected_idx: usize,
    scene: Scene,

    current_song: Option<SongEntry>,
    chart: Option<Chart>,
    score: ScoreState,
    start_time_sec: f64,
    _audio_stream: Option<OutputStream>,
    audio_handle: Option<OutputStreamHandle>,
    sink: Option<Sink>,
    ui_font: Option<Font>,
    roll_hit_count: i32,
    se_don: Option<SeData>,
    se_ka: Option<SeData>,
    se_latency_stats: Arc<SeLatencyStats>,

    config_path: PathBuf,
    config: AppConfig,
    keybinds: Keybinds,

    error_message: String,
}

impl TaikoSimApp {
    pub fn new(root_dir: PathBuf) -> Self {
        let data_dir = root_dir.join("data");
        let songs = load_song_entries(&data_dir);
        let config_path = root_dir.join("config.json");
        let config = load_config(&config_path);
        let keybinds = Keybinds::from_config(&config.keybind);
        let se_latency_stats = Arc::new(SeLatencyStats::new());
        let (audio_stream, audio_handle) = match OutputStream::try_default() {
            Ok((stream, handle)) => (Some(stream), Some(handle)),
            Err(_) => (None, None),
        };
        Self {
            root_dir,
            songs,
            selected_idx: 0,
            scene: Scene::SongSelect,
            current_song: None,
            chart: None,
            score: ScoreState::default(),
            start_time_sec: 0.0,
            _audio_stream: audio_stream,
            audio_handle,
            sink: None,
            ui_font: None,
            roll_hit_count: 0,
            se_don: None,
            se_ka: None,
            se_latency_stats,
            config_path,
            config,
            keybinds,
            error_message: String::new(),
        }
    }

    pub async fn run(&mut self) {
        self.try_load_ui_font().await;
        self.try_load_se();

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

        if self.keybinds.don_pressed() {
            self.play_key_se(NoteKind::Don);
            self.try_hit(NoteKind::Don);
        }
        if self.keybinds.kat_pressed() {
            self.play_key_se(NoteKind::Kat);
            self.try_hit(NoteKind::Kat);
        }
        if is_key_pressed(KeyCode::LeftBracket) {
            self.adjust_timing_offset(-5);
        }
        if is_key_pressed(KeyCode::RightBracket) {
            self.adjust_timing_offset(5);
        }

        if let Some(sink) = &self.sink {
            sink.set_volume(self.config.music_volume.clamp(0.0, 2.0));
        }

        let now = self.current_time_sec();
        if self.current_active_roll(now).is_none() {
            self.roll_hit_count = 0;
        }

        let timing_offset_sec = self.timing_offset_sec();
        let mut miss_count = 0_i32;
        if let Some(chart) = self.chart.as_mut() {
            for note in &mut chart.notes {
                if note.judged {
                    continue;
                }
                let note_time = note.time_sec - chart.offset_sec + timing_offset_sec;
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

        let file = match File::open(audio_str) {
            Ok(f) => f,
            Err(e) => {
                self.error_message = format!("音源読み込み失敗: {e}");
                return;
            }
        };
        let source = match Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => {
                self.error_message = format!("音源デコード失敗: {e}");
                return;
            }
        };
        let Some(audio_handle) = self.audio_handle.as_ref() else {
            self.error_message = "音声デバイス初期化失敗".to_string();
            return;
        };
        let sink = match Sink::try_new(audio_handle) {
            Ok(s) => s,
            Err(e) => {
                self.error_message = format!("音声再生初期化失敗: {e}");
                return;
            }
        };
        sink.set_volume(self.config.music_volume.clamp(0.0, 2.0));
        sink.append(source);
        sink.play();

        self.current_song = Some(song);
        self.chart = Some(chart);
        self.score = ScoreState::default();
        self.roll_hit_count = 0;
        self.start_time_sec = get_time();
        self.sink = Some(sink);
        self.scene = Scene::Play;
    }

    fn current_time_sec(&self) -> f64 {
        get_time() - self.start_time_sec
    }

    fn try_hit(&mut self, input_kind: NoteKind) {
        let now = self.current_time_sec();
        if self.hit_roll_if_active(now, input_kind) {
            return;
        }

        let Some((idx, delta)) = self.find_nearest_note(now, MISS_WINDOW) else {
            return;
        };

        let Some(chart) = self.chart.as_mut() else {
            return;
        };

        let note = &mut chart.notes[idx];

        if note.kind != input_kind {
            note.judged = true;
            note.hit = false;
            self.register_judge("Miss");
            return;
        }

        if delta <= JUDGE_PERFECT {
            note.judged = true;
            note.hit = true;
            self.register_judge("Perfect");
        } else if delta <= JUDGE_GOOD {
            note.judged = true;
            note.hit = true;
            self.register_judge("Good");
        } else if delta <= MISS_WINDOW {
            note.judged = true;
            note.hit = false;
            self.register_judge("Miss");
        }
    }

    fn find_nearest_note(&self, now: f64, window: f64) -> Option<(usize, f64)> {
        let chart = self.chart.as_ref()?;

        let mut best_idx: Option<usize> = None;
        let mut best_abs = f64::MAX;

        for (idx, note) in chart.notes.iter().enumerate() {
            if note.judged {
                continue;
            }

            let note_time = note.time_sec - chart.offset_sec + self.timing_offset_sec();
            let abs_delta = (now - note_time).abs();
            if abs_delta <= window && abs_delta < best_abs {
                best_abs = abs_delta;
                best_idx = Some(idx);
            }
        }

        best_idx.map(|i| (i, best_abs))
    }

    fn hit_roll_if_active(&mut self, now: f64, input_kind: NoteKind) -> bool {
        let Some(chart) = self.chart.as_ref() else {
            return false;
        };

        for roll in &chart.rolls {
            let start = roll.start_sec - chart.offset_sec + self.timing_offset_sec();
            let end = roll.end_sec - chart.offset_sec + self.timing_offset_sec();
            if now < start || now > end {
                continue;
            }

            if !roll.balloon {
                self.score.score += ROLL_HIT_SCORE;
                self.roll_hit_count += 1;
                self.score.last_judge = if roll.big {
                    "Roll(Big)".to_string()
                } else {
                    "Roll".to_string()
                };
                return true;
            }

            if roll.balloon && input_kind == NoteKind::Don {
                self.score.score += ROLL_HIT_SCORE;
                self.roll_hit_count += 1;
                self.score.last_judge = "Balloon".to_string();
                return true;
            }
        }

        false
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
        if let Some(sink) = &self.sink {
            sink.stop();
        }
        self.sink = None;
        self.scene = Scene::Result;
    }

    fn draw_song_select(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));
        self.draw_ui_text("Song Select", 60.0, 70.0, 56, WHITE);
        self.draw_ui_text(
            "UP/DOWN: select  ENTER: play  ESC: quit",
            60.0,
            105.0,
            28,
            Color::from_rgba(170, 180, 200, 255),
        );

        if self.songs.is_empty() {
            self.draw_ui_text("No songs found in data/songs", 60.0, 190.0, 34, RED);
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
            self.draw_ui_text(&label, 80.0, y, 38, color);
            y += 44.0;
        }

        if !self.error_message.is_empty() {
            self.draw_ui_text(
                &self.error_message,
                60.0,
                screen_height() - 40.0,
                30,
                Color::from_rgba(255, 120, 120, 255),
            );
        }
    }

    fn draw_play(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));

        let Some(chart) = self.chart.as_ref() else {
            return;
        };

        let cam = Camera2D {
            zoom: vec2(2.0 / BASE_W, -2.0 / BASE_H),
            target: vec2(BASE_W / 2.0, BASE_H / 2.0),
            ..Default::default()
        };
        set_camera(&cam);

        draw_rectangle(
            LANE_X,
            LANE_Y - (LANE_H * 0.5),
            LANE_W,
            LANE_H,
            Color::from_rgba(39, 45, 56, 255),
        );
        draw_circle(
            TARGET_X,
            LANE_Y,
            JUDGE_RING_RADIUS,
            Color::from_rgba(255, 245, 180, 30),
        );
        draw_circle_lines(
            TARGET_X,
            LANE_Y,
            JUDGE_RING_RADIUS,
            3.0,
            Color::from_rgba(255, 230, 140, 255),
        );

        let now = self.current_time_sec();
        self.draw_rolls(chart, now);
        for note in &chart.notes {
            if note.judged && note.hit {
                continue;
            }

            let note_time = note.time_sec - chart.offset_sec + self.timing_offset_sec();
            let diff = note_time - now;
            if !(-SCROLL_BEHIND_SEC..=SCROLL_AHEAD_SEC).contains(&diff) {
                continue;
            }

            let x = TARGET_X + ((diff / SCROLL_AHEAD_SEC) as f32) * SCROLL_PIXELS;
            let color = match note.kind {
                NoteKind::Don => Color::from_rgba(255, 120, 100, 255),
                NoteKind::Kat => Color::from_rgba(100, 180, 255, 255),
            };
            draw_circle(x, LANE_Y, NOTE_RADIUS, color);
            draw_circle_lines(x, LANE_Y, NOTE_RADIUS, 2.0, BLACK);
        }

        set_default_camera();

        let title = format!("{} [{} Lv{}]", chart.title, chart.course, chart.level);
        self.draw_ui_text(&title, 40.0, 56.0, 40, WHITE);

        let judge = format!("Judge: {}", self.score.last_judge);
        let score = format!("Score: {}", self.score.score);
        let combo = format!("{}", self.score.combo);
        let offset_text = format!("Offset: {}ms ([ / ])", self.config.timing_offset_ms);
        self.draw_ui_text(&judge, 40.0, 570.0, 36, WHITE);
        self.draw_ui_text(&score, 320.0, 570.0, 36, WHITE);
        self.draw_ui_text(
            &combo,
            TARGET_X - 120.0,
            LANE_Y - 56.0,
            54,
            Color::from_rgba(255, 240, 170, 255),
        );
        self.draw_ui_text(
            "F/J: Don  D/K: Kat  Roll: drum  ESC: result",
            40.0,
            630.0,
            30,
            Color::from_rgba(170, 180, 200, 255),
        );
        self.draw_ui_text(
            &offset_text,
            900.0,
            570.0,
            30,
            Color::from_rgba(220, 230, 255, 255),
        );

        if self.current_active_roll(now).is_some() {
            let roll_text = format!("連打: {}", self.roll_hit_count);
            self.draw_ui_text(
                &roll_text,
                TARGET_X - 55.0,
                LANE_Y - 72.0,
                30,
                Color::from_rgba(255, 245, 180, 255),
            );
        }

        if self.config.debug {
            let fps_text = format!("FPS: {}", get_fps());
            let time_text = format!("Time: {:.3}s", now);
            let (last_ms, avg_ms, max_ms, n) = self.se_latency_stats.snapshot_ms();
            let se_text = format!("SE Latency ms  last:{:.1} avg:{:.1} max:{:.1} n:{}", last_ms, avg_ms, max_ms, n);
            self.draw_ui_text(&fps_text, 1030.0, 40.0, 24, Color::from_rgba(170, 255, 170, 255));
            self.draw_ui_text(&time_text, 1030.0, 68.0, 24, Color::from_rgba(170, 255, 170, 255));
            self.draw_ui_text(&se_text, 700.0, 96.0, 24, Color::from_rgba(170, 255, 170, 255));
        }
    }

    fn draw_rolls(&self, chart: &Chart, now: f64) {
        for roll in &chart.rolls {
            self.draw_roll(roll, chart.offset_sec, now);
        }
    }

    fn current_active_roll(&self, now: f64) -> Option<&Roll> {
        let chart = self.chart.as_ref()?;
        for roll in &chart.rolls {
            let start = roll.start_sec - chart.offset_sec + self.timing_offset_sec();
            let end = roll.end_sec - chart.offset_sec + self.timing_offset_sec();
            if now >= start && now <= end {
                return Some(roll);
            }
        }
        None
    }

    fn draw_roll(&self, roll: &Roll, offset_sec: f64, now: f64) {
        let start_diff = (roll.start_sec - offset_sec + self.timing_offset_sec()) - now;
        let end_diff = (roll.end_sec - offset_sec + self.timing_offset_sec()) - now;

        if end_diff < -SCROLL_BEHIND_SEC || start_diff > SCROLL_AHEAD_SEC {
            return;
        }

        let start_x = TARGET_X + ((start_diff / SCROLL_AHEAD_SEC) as f32) * SCROLL_PIXELS;
        let end_x = TARGET_X + ((end_diff / SCROLL_AHEAD_SEC) as f32) * SCROLL_PIXELS;
        let body_left = start_x.min(end_x).max(LANE_X);
        let body_right = start_x.max(end_x).min(LANE_X + LANE_W);

        let base_color = if roll.balloon {
            Color::from_rgba(255, 200, 80, 255)
        } else if roll.big {
            Color::from_rgba(255, 160, 60, 255)
        } else {
            Color::from_rgba(255, 210, 90, 255)
        };

        if body_right > body_left {
            draw_rectangle(body_left, LANE_Y - 12.0, body_right - body_left, 24.0, base_color);
        }
        draw_circle(start_x, LANE_Y, 16.0, base_color);
        draw_circle(end_x, LANE_Y, 16.0, base_color);
        draw_circle_lines(start_x, LANE_Y, 16.0, 2.0, BLACK);
        draw_circle_lines(end_x, LANE_Y, 16.0, 2.0, BLACK);
    }

    fn draw_result(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));
        self.draw_ui_text("Result", 60.0, 70.0, 56, WHITE);

        let song_name = self
            .current_song
            .as_ref()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        self.draw_ui_text(&song_name, 80.0, 130.0, 36, Color::from_rgba(200, 220, 255, 255));

        let lines = [
            format!("Score: {}", self.score.score),
            format!("Max Combo: {}", self.score.max_combo),
            format!("Perfect: {}", self.score.perfect),
            format!("Good: {}", self.score.good),
            format!("Miss: {}", self.score.miss),
        ];
        let mut y = 200.0;
        for line in lines {
            self.draw_ui_text(&line, 80.0, y, 40, Color::from_rgba(230, 230, 230, 255));
            y += 56.0;
        }

        self.draw_ui_text(
            "ENTER: song select  ESC: quit",
            80.0,
            640.0,
            30,
            Color::from_rgba(170, 180, 200, 255),
        );
    }

    fn draw_ui_text(&self, text: &str, x: f32, y: f32, font_size: u16, color: Color) {
        let sx = screen_width() / BASE_W;
        let sy = screen_height() / BASE_H;
        let scaled_size = ((font_size as f32) * sy).round().clamp(10.0, 180.0) as u16;
        draw_text_ex(
            text,
            x * sx,
            y * sy,
            TextParams {
                font: self.ui_font.as_ref(),
                font_size: scaled_size,
                color,
                ..Default::default()
            },
        );
    }

    fn timing_offset_sec(&self) -> f64 {
        self.config.timing_offset_ms as f64 / 1000.0
    }

    fn adjust_timing_offset(&mut self, delta_ms: i32) {
        let new_value = (self.config.timing_offset_ms + delta_ms).clamp(-300, 300);
        self.config.timing_offset_ms = new_value;
        self.score.last_judge = format!("Offset {}ms", new_value);
        save_config(&self.config_path, &self.config);
    }

    async fn try_load_ui_font(&mut self) {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(path) = self.config.font_path.clone() {
            candidates.push(self.root_dir.join(path));
        }
        candidates.push(self.root_dir.join("assets/fonts/NotoSansJP-Regular.ttf"));
        candidates.push(self.root_dir.join("assets/fonts/NotoSansCJKjp-Regular.otf"));
        candidates.push(self.root_dir.join("assets/fonts/NotoSansCJK-Regular.ttc"));
        candidates.push(PathBuf::from("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"));
        candidates.push(PathBuf::from("/usr/share/fonts/noto-cjk/NotoSansCJK-Medium.ttc"));
        candidates.push(PathBuf::from("/usr/share/fonts/noto/NotoSansJP-Regular.ttf"));
        candidates.push(PathBuf::from("/usr/share/fonts/noto/NotoSansCJK-Regular.ttc"));
        candidates.push(PathBuf::from("/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf"));

        for path in candidates {
            if !path.exists() {
                continue;
            }
            let Some(path_str) = path.to_str() else {
                continue;
            };
            if let Ok(font) = load_ttf_font(path_str).await {
                self.ui_font = Some(font);
                return;
            }
        }
    }

    fn try_load_se(&mut self) {
        let don_path = self.root_dir.join("assets/se/dong.wav");
        let ka_path = self.root_dir.join("assets/se/ka.wav");

        self.se_don = load_se_data(&don_path);
        self.se_ka = load_se_data(&ka_path);
    }

    fn play_key_se(&self, kind: NoteKind) {
        if !self.config.se_enabled {
            return;
        }

        let Some(audio_handle) = self.audio_handle.as_ref() else {
            return;
        };

        let se = match kind {
            NoteKind::Don => self.se_don.as_ref(),
            NoteKind::Kat => self.se_ka.as_ref(),
        };
        let Some(se_data) = se else {
            return;
        };

        let source = SeSource {
            data: se_data.samples.clone(),
            idx: 0,
            channels: se_data.channels,
            sample_rate: se_data.sample_rate,
            gain: self.config.se_volume.clamp(0.0, 2.0),
            triggered_at: Instant::now(),
            recorded: false,
            latency_stats: self.se_latency_stats.clone(),
        };

        if audio_handle.play_raw(source).is_err() {
            return;
        }
    }
}

fn load_se_data(path: &std::path::Path) -> Option<SeData> {
    let file = File::open(path).ok()?;
    let decoder = Decoder::new(BufReader::new(file)).ok()?;
    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels();
    let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
    Some(SeData {
        channels,
        sample_rate,
        samples: Arc::from(samples.into_boxed_slice()),
    })
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

fn load_config(path: &std::path::Path) -> AppConfig {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return AppConfig::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn parse_keycode(raw: &str) -> Option<KeyCode> {
    let s = raw.trim().to_ascii_uppercase();
    match s.as_str() {
        "A" => Some(KeyCode::A),
        "B" => Some(KeyCode::B),
        "C" => Some(KeyCode::C),
        "D" => Some(KeyCode::D),
        "E" => Some(KeyCode::E),
        "F" => Some(KeyCode::F),
        "G" => Some(KeyCode::G),
        "H" => Some(KeyCode::H),
        "I" => Some(KeyCode::I),
        "J" => Some(KeyCode::J),
        "K" => Some(KeyCode::K),
        "L" => Some(KeyCode::L),
        "M" => Some(KeyCode::M),
        "N" => Some(KeyCode::N),
        "O" => Some(KeyCode::O),
        "P" => Some(KeyCode::P),
        "Q" => Some(KeyCode::Q),
        "R" => Some(KeyCode::R),
        "S" => Some(KeyCode::S),
        "T" => Some(KeyCode::T),
        "U" => Some(KeyCode::U),
        "V" => Some(KeyCode::V),
        "W" => Some(KeyCode::W),
        "X" => Some(KeyCode::X),
        "Y" => Some(KeyCode::Y),
        "Z" => Some(KeyCode::Z),
        "LEFT" => Some(KeyCode::Left),
        "RIGHT" => Some(KeyCode::Right),
        "UP" => Some(KeyCode::Up),
        "DOWN" => Some(KeyCode::Down),
        "SPACE" => Some(KeyCode::Space),
        "ENTER" => Some(KeyCode::Enter),
        "TAB" => Some(KeyCode::Tab),
        "LSHIFT" => Some(KeyCode::LeftShift),
        "RSHIFT" => Some(KeyCode::RightShift),
        "LCTRL" => Some(KeyCode::LeftControl),
        "RCTRL" => Some(KeyCode::RightControl),
        _ => None,
    }
}

fn save_config(path: &std::path::Path, cfg: &AppConfig) {
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path, text);
    }
}
