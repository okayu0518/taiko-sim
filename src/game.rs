use std::path::PathBuf;
use std::{fs::File, io::BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::Instant;

use macroquad::prelude::*;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

use crate::models::{AppConfig, KeybindConfig, NoteKind, ScoreState, ScrollSpeed, SongEntry};

use crate::effects::{note_color, Effects};
use crate::gameplay::{HitOutcome, PlaySession};
use crate::judge::Judge;
use crate::repository::load_song_entries;
use crate::tja_parser::parse_tja;

const SCROLL_BEHIND_SEC: f64 = 0.25;
/// Pixels per beat at `#SCROLL 1`. One 4/4 measure spans roughly the judge circle to the lane end.
const PX_PER_BEAT: f32 = 240.0;

const BASE_W: f32 = 1280.0;
const BASE_H: f32 = 720.0;
const TARGET_X: f32 = 290.0;
const LANE_Y: f32 = 350.0;
const NOTE_RADIUS: f32 = 24.0;
const BIG_NOTE_RADIUS: f32 = 36.0;
const JUDGE_RING_RADIUS: f32 = 26.0;
const LANE_X: f32 = 140.0;
const LANE_W: f32 = 1110.0;
const LANE_H: f32 = 120.0;

enum Scene {
    SongSelect,
    Play,
    Result,
}

#[derive(Clone)]
struct SeData {
    samples: Arc<[f32]>,
    sample_rate: u32,
}

struct SeSource {
    data: Arc<[f32]>,
    sample_rate: u32,
    idx: usize,
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
        Some(self.data.len().saturating_sub(self.idx))
    }
    fn channels(&self) -> u16 { 1 }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn total_duration(&self) -> Option<Duration> { None }
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
        (last as f32 / 1000.0, avg as f32 / 1000.0, max as f32 / 1000.0, count)
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
    session: Option<PlaySession>,
    start_time_sec: f64,
    _audio_stream: Option<OutputStream>,
    audio_handle: Option<OutputStreamHandle>,
    sink: Option<Sink>,
    ui_font: Option<Font>,
    effects: Effects,
    se_don: Option<SeData>,
    se_ka: Option<SeData>,
    se_latency_stats: Arc<SeLatencyStats>,

    config_path: PathBuf,
    config: AppConfig,
    keybinds: Keybinds,

    error_message: String,
}

impl TaikoSimApp {
    pub fn new(root_dir: PathBuf, config: AppConfig) -> Self {
        let data_dir = root_dir.join("data");
        let songs = load_song_entries(&data_dir);
        let config_path = root_dir.join("config.json");
        let keybinds = Keybinds::from_config(&config.keybind);
        let se_latency_stats = Arc::new(SeLatencyStats::new());
        let (audio_stream, audio_handle) = OutputStream::try_default().ok()
            .map(|(s, h)| (Some(s), Some(h)))
            .unwrap_or((None, None));

        Self {
            root_dir,
            songs,
            selected_idx: 0,
            scene: Scene::SongSelect,
            current_song: None,
            session: None,
            start_time_sec: 0.0,
            _audio_stream: audio_stream,
            audio_handle,
            sink: None,
            ui_font: None,
            effects: Effects::default(),
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
        if self.songs.is_empty() { return; }

        if is_key_pressed(KeyCode::Up) {
            self.selected_idx = if self.selected_idx == 0 { self.songs.len() - 1 } else { self.selected_idx - 1 };
        }
        if is_key_pressed(KeyCode::Down) {
            self.selected_idx = (self.selected_idx + 1) % self.songs.len();
        }
        if is_key_pressed(KeyCode::Enter) {
            self.start_selected_song().await;
        }
    }

    fn update_play(&mut self) {
        if self.session.is_none() { return; }

        let now = self.current_time_sec();
        let offset = self.timing_offset_sec();
        if let Some(session) = self.session.as_mut() { session.check_branch_points(now, offset); }

        self.effects.update(now);

        if self.keybinds.don_pressed() {
            self.play_key_se(NoteKind::Don);
            self.effects.spawn_input(now, NoteKind::Don);
            self.try_hit(now, NoteKind::Don);
        }
        if self.keybinds.kat_pressed() {
            self.play_key_se(NoteKind::Kat);
            self.effects.spawn_input(now, NoteKind::Kat);
            self.try_hit(now, NoteKind::Kat);
        }
        if is_key_pressed(KeyCode::LeftBracket) { self.adjust_timing_offset(-5); }
        if is_key_pressed(KeyCode::RightBracket) { self.adjust_timing_offset(5); }

        if let Some(sink) = &self.sink {
            sink.set_volume(self.config.music_volume.clamp(0.0, 2.0));
        }

        let offset = self.timing_offset_sec();
        let Some(session) = self.session.as_mut() else { return; };
        let missed = session.update(now, offset);
        let finished = session.is_finished(now);
        if missed > 0 { self.effects.spawn_judge_text(now, Judge::Fuka); }
        if finished { self.to_result(); }
    }

    fn update_result(&mut self) {
        if is_key_pressed(KeyCode::Enter) { self.scene = Scene::SongSelect; }
    }

    async fn start_selected_song(&mut self) {
        self.error_message.clear();
        let Some(song) = self.songs.get(self.selected_idx).cloned() else { return; };

        self.ensure_bgm_output();
        let (chart, wave) = match parse_tja(&song.tja_path, Some("Oni")) {
            Ok(v) => v,
            Err(e) => { self.error_message = format!("譜面読み込み失敗: {e}"); return; }
        };

        let audio_path = resolve_audio_path(&song, wave.as_deref());
        let Some(audio) = audio_path else { self.error_message = "音源ファイルが見つかりません".to_string(); return; };

        let file = match File::open(&audio) {
            Ok(f) => f,
            Err(e) => { self.error_message = format!("音源読み込み失敗: {e}"); return; }
        };
        let source = match Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => { self.error_message = format!("音源デコード失敗: {e}"); return; }
        };
        let Some(audio_handle) = self.audio_handle.as_ref() else {
            self.error_message = "音声デバイス初期化失敗".to_string(); return;
        };
        let sink = match Sink::try_new(audio_handle) {
            Ok(s) => s,
            Err(e) => { self.error_message = format!("音声再生初期化失敗: {e}"); return; }
        };
        // Prepend silence and shift the game clock by the same amount so audio and chart stay in sync.
        let lead_in = self.config.lead_in_sec();
        sink.set_volume(self.config.music_volume.clamp(0.0, 2.0));
        sink.append(source.delay(Duration::from_secs_f64(lead_in)));
        sink.play();

        self.current_song = Some(song);
        self.session = Some(PlaySession::new(chart));
        self.effects.clear();
        self.start_time_sec = get_time() + lead_in;
        self.sink = Some(sink);
        self.scene = Scene::Play;
    }

    fn current_time_sec(&self) -> f64 { get_time() - self.start_time_sec }

    fn try_hit(&mut self, now: f64, input_kind: NoteKind) {
        let offset = self.timing_offset_sec();
        let Some(session) = self.session.as_mut() else { return; };
        if let HitOutcome::Note { judge, big } = session.hit(now, offset, input_kind) {
            self.effects.spawn_hit(now, input_kind, judge, note_radius(big));
        }
    }

    fn to_result(&mut self) {
        if let Some(sink) = &self.sink { sink.stop(); }
        self.sink = None;
        self.scene = Scene::Result;
    }

    fn draw_song_select(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));
        self.draw_ui_text("Song Select", 60.0, 70.0, 56, WHITE);
        self.draw_ui_text("UP/DOWN: select  ENTER: play  ESC: quit", 60.0, 105.0, 28, Color::from_rgba(170, 180, 200, 255));

        if self.songs.is_empty() {
            self.draw_ui_text("No songs found in data/songs", 60.0, 190.0, 34, RED);
            return;
        }

        for (idx, song) in self.songs.iter().enumerate() {
            let selected = idx == self.selected_idx;
            let color = if selected { Color::from_rgba(255, 230, 140, 255) } else { Color::from_rgba(230, 230, 230, 255) };
            let label = format!("{}{} ({})", if selected { "> " } else { "  " }, song.title, song.song_id);
            self.draw_ui_text(&label, 80.0, 170.0 + idx as f32 * 44.0, 38, color);
        }

        if !self.error_message.is_empty() {
            self.draw_ui_text(&self.error_message, 60.0, screen_height() - 40.0, 30, Color::from_rgba(255, 120, 120, 255));
        }
    }

    fn draw_play(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));
        let Some(session) = self.session.as_ref() else { return; };
        let chart = &session.chart;
        let offset = self.timing_offset_sec();

        let cam = Camera2D { zoom: vec2(2.0 / BASE_W, -2.0 / BASE_H), target: vec2(BASE_W / 2.0, BASE_H / 2.0), ..Default::default() };
        set_camera(&cam);

        let lane_rect = Rect::new(LANE_X, LANE_Y - (LANE_H * 0.5), LANE_W, LANE_H);
        let target = vec2(TARGET_X, LANE_Y);
        let now = self.current_time_sec();
        draw_rectangle(lane_rect.x, lane_rect.y, lane_rect.w, lane_rect.h, Color::from_rgba(39, 45, 56, 255));
        self.effects.draw_under_notes(now, lane_rect);
        draw_circle(TARGET_X, LANE_Y, JUDGE_RING_RADIUS, Color::from_rgba(255, 245, 180, 30));
        draw_circle_lines(TARGET_X, LANE_Y, JUDGE_RING_RADIUS, 3.0, Color::from_rgba(255, 230, 140, 255));

        let cur_branch = session.current_branch();
        self.draw_rolls(session, now);
        for note in &chart.notes {
            if note.judged && note.hit { continue; }
            if !session.is_active(note.branch, note.time_sec) { continue; }

            let diff = session.game_time(note.time_sec, offset) - now;
            if diff < -SCROLL_BEHIND_SEC { continue; }

            let x = scroll_x(diff, note.speed);
            let radius = note_radius(note.big);
            if !is_x_visible(x, radius) { continue; }
            draw_circle(x, LANE_Y, radius, note_color(note.kind));
            draw_circle_lines(x, LANE_Y, radius, 2.0, BLACK);
        }
        self.effects.draw_over_notes(now, target);

        set_default_camera();
        self.effects.draw_texts(now, target, |text, x, y, size, color| self.draw_ui_text(text, x, y, size, color));

        let branch_name = if chart.has_branch { format!(" [{:?}]", cur_branch) } else { "".to_string() };
        self.draw_ui_text(&format!("{} [{} Lv{}]{}", chart.title, chart.course, chart.level, branch_name), 40.0, 56.0, 40, WHITE);
        self.draw_ui_text(&format!("Score: {}", session.score.score), 320.0, 570.0, 36, WHITE);
        self.draw_ui_text(&session.score.combo.to_string(), TARGET_X - 120.0, LANE_Y - 56.0, 54, Color::from_rgba(255, 240, 170, 255));
        self.draw_ui_text("F/J: Don  D/K: Kat  Roll: drum  ESC: result", 40.0, 630.0, 30, Color::from_rgba(170, 180, 200, 255));
        self.draw_ui_text(&format!("Offset: {}ms ([ / ])", self.config.timing_offset_ms), 900.0, 570.0, 30, Color::from_rgba(220, 230, 255, 255));

        if session.active_roll(now, offset).is_some() {
            self.draw_ui_text(&format!("連打: {}", session.roll_hit_count), TARGET_X - 55.0, LANE_Y - 72.0, 30, Color::from_rgba(255, 245, 180, 255));
        }

        if self.config.debug {
            let (last_ms, avg_ms, max_ms, n) = self.se_latency_stats.snapshot_ms();
            self.draw_ui_text(&format!("FPS: {}", get_fps()), 1030.0, 40.0, 24, Color::from_rgba(170, 255, 170, 255));
            self.draw_ui_text(&format!("Time: {:.3}s", now), 1030.0, 68.0, 24, Color::from_rgba(170, 255, 170, 255));
            self.draw_ui_text(&format!("SE Latency ms  last:{:.1} avg:{:.1} max:{:.1} n:{}", last_ms, avg_ms, max_ms, n), 700.0, 96.0, 24, Color::from_rgba(170, 255, 170, 255));
            if let Some(se) = &self.se_don {
                self.draw_ui_text(&format!("SE SampleRate: {}Hz", se.sample_rate), 700.0, 124.0, 24, Color::from_rgba(170, 255, 170, 255));
            }
            if chart.has_branch {
                let section = session.section;
                self.draw_ui_text(&format!("Section: 良:{} 可:{} 不可:{} R:{}", section.perfect, section.good, section.miss, section.rolls), 700.0, 152.0, 24, Color::from_rgba(170, 255, 170, 255));
                self.draw_ui_text(&format!("Accuracy: {:.1}%", section.accuracy()), 700.0, 180.0, 24, Color::from_rgba(170, 255, 170, 255));
            }
        }
    }

    fn draw_rolls(&self, session: &PlaySession, now: f64) {
        let offset = self.timing_offset_sec();
        for roll in &session.chart.rolls {
            if !session.is_active(roll.branch, roll.start_sec) { continue; }
            let start_diff = session.game_time(roll.start_sec, offset) - now;
            let end_diff = session.game_time(roll.end_sec, offset) - now;
            if end_diff < -SCROLL_BEHIND_SEC { continue; }

            let start_x = scroll_x(start_diff, roll.head_speed);
            let end_x = scroll_x(end_diff, roll.tail_speed);
            if start_x.max(end_x) < LANE_X || start_x.min(end_x) > LANE_X + LANE_W { continue; }
            let body_left = start_x.min(end_x).max(LANE_X);
            let body_right = start_x.max(end_x).min(LANE_X + LANE_W);

            let base_color = if roll.balloon { Color::from_rgba(255, 200, 80, 255) } else if roll.big { Color::from_rgba(255, 160, 60, 255) } else { Color::from_rgba(255, 210, 90, 255) };

            if body_right > body_left { draw_rectangle(body_left, LANE_Y - 12.0, body_right - body_left, 24.0, base_color); }
            draw_circle(start_x, LANE_Y, 16.0, base_color);
            draw_circle(end_x, LANE_Y, 16.0, base_color);
            draw_circle_lines(start_x, LANE_Y, 16.0, 2.0, BLACK);
            draw_circle_lines(end_x, LANE_Y, 16.0, 2.0, BLACK);
        }
    }

    fn draw_result(&self) {
        clear_background(Color::from_rgba(24, 26, 33, 255));
        self.draw_ui_text("Result", 60.0, 70.0, 56, WHITE);
        let song_name = self.current_song.as_ref().map(|s| s.title.clone()).unwrap_or_else(|| "Unknown".to_string());
        self.draw_ui_text(&song_name, 80.0, 130.0, 36, Color::from_rgba(200, 220, 255, 255));

        let default_score = ScoreState::default();
        let score = self.session.as_ref().map_or(&default_score, |s| &s.score);
        let stats = [
            format!("Score: {}", score.score),
            format!("Max Combo: {}", score.max_combo),
            format!("良: {}", score.perfect),
            format!("可: {}", score.good),
            format!("不可: {}", score.miss),
        ];
        for (i, line) in stats.iter().enumerate() {
            self.draw_ui_text(line, 80.0, 200.0 + i as f32 * 56.0, 40, Color::from_rgba(230, 230, 230, 255));
        }
        self.draw_ui_text("ENTER: song select  ESC: quit", 80.0, 640.0, 30, Color::from_rgba(170, 180, 200, 255));
    }

    fn draw_ui_text(&self, text: &str, x: f32, y: f32, font_size: u16, color: Color) {
        let sy = screen_height() / BASE_H;
        let sx = screen_width() / BASE_W;
        draw_text_ex(text, x * sx, y * sy, TextParams {
            font: self.ui_font.as_ref(),
            font_size: ((font_size as f32) * sy).round() as u16,
            color, ..Default::default()
        });
    }

    fn timing_offset_sec(&self) -> f64 { self.config.timing_offset_ms as f64 / 1000.0 }

    fn adjust_timing_offset(&mut self, delta_ms: i32) {
        self.config.timing_offset_ms = (self.config.timing_offset_ms + delta_ms).clamp(-300, 300);
        if let Ok(text) = serde_json::to_string_pretty(&self.config) { let _ = std::fs::write(&self.config_path, text); }
    }

    async fn try_load_ui_font(&mut self) {
        let mut candidates = vec![
            self.root_dir.join("assets/fonts/NotoSansJP-Regular.ttf"),
            self.root_dir.join("assets/fonts/NotoSansCJKjp-Regular.otf"),
            self.root_dir.join("assets/fonts/NotoSansCJK-Regular.ttc"),
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc".into(),
            "/usr/share/fonts/noto/NotoSansJP-Regular.ttf".into(),
        ];
        if let Some(p) = &self.config.font_path { candidates.insert(0, self.root_dir.join(p)); }

        for path in candidates {
            if let Some(p_str) = path.to_str() {
                if let Ok(font) = load_ttf_font(p_str).await { self.ui_font = Some(font); return; }
            }
        }
    }

    fn try_load_se(&mut self) {
        self.se_don = load_se_data(&self.root_dir.join("assets/se/dong.wav"));
        self.se_ka = load_se_data(&self.root_dir.join("assets/se/ka.wav"));
    }

    fn play_key_se(&self, kind: NoteKind) {
        if !self.config.se_enabled { return; }
        let Some(handle) = self.audio_handle.as_ref() else { return; };
        let Some(se_data) = (if kind == NoteKind::Don { &self.se_don } else { &self.se_ka }).as_ref() else { return; };

        let source = SeSource {
            data: se_data.samples.clone(),
            sample_rate: se_data.sample_rate,
            idx: 0,
            gain: self.config.se_volume.clamp(0.0, 2.0),
            triggered_at: Instant::now(), recorded: false,
            latency_stats: self.se_latency_stats.clone(),
        };
        // Use play_raw to avoid Sink overhead
        let _ = handle.play_raw(source);
    }

    fn ensure_bgm_output(&mut self) {
        if self.audio_handle.is_none() {
            if let Ok((s, h)) = OutputStream::try_default() { self._audio_stream = Some(s); self.audio_handle = Some(h); }
        }
    }
}

/// Horizontal position of an object `diff_sec` seconds before its hit time.
fn scroll_x(diff_sec: f64, speed: ScrollSpeed) -> f32 {
    TARGET_X + (diff_sec * speed.beats_per_sec()) as f32 * PX_PER_BEAT
}

/// Whether a circle of `radius` at `x` overlaps the lane.
fn is_x_visible(x: f32, radius: f32) -> bool {
    x + radius >= LANE_X && x - radius <= LANE_X + LANE_W
}

fn note_radius(big: bool) -> f32 {
    if big { BIG_NOTE_RADIUS } else { NOTE_RADIUS }
}

fn load_se_data(path: &std::path::Path) -> Option<SeData> {
    let file = File::open(path).ok()?;
    let decoder = Decoder::new(BufReader::new(file)).ok()?;
    let sample_rate = decoder.sample_rate();
    let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
    Some(SeData { samples: Arc::from(samples.into_boxed_slice()), sample_rate })
}

fn resolve_audio_path(song: &SongEntry, wave: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = &song.audio_path { if p.exists() { return Some(p.clone()); } }
    if let Some(w) = wave {
        let cand = song.tja_path.parent()?.join(w.trim());
        if cand.exists() { return Some(cand); }
    }
    None
}

fn parse_keycode(raw: &str) -> Option<KeyCode> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "A" => Some(KeyCode::A), "B" => Some(KeyCode::B), "C" => Some(KeyCode::C), "D" => Some(KeyCode::D),
        "E" => Some(KeyCode::E), "F" => Some(KeyCode::F), "G" => Some(KeyCode::G), "H" => Some(KeyCode::H),
        "I" => Some(KeyCode::I), "J" => Some(KeyCode::J), "K" => Some(KeyCode::K), "L" => Some(KeyCode::L),
        "M" => Some(KeyCode::M), "N" => Some(KeyCode::N), "O" => Some(KeyCode::O), "P" => Some(KeyCode::P),
        "Q" => Some(KeyCode::Q), "R" => Some(KeyCode::R), "S" => Some(KeyCode::S), "T" => Some(KeyCode::T),
        "U" => Some(KeyCode::U), "V" => Some(KeyCode::V), "W" => Some(KeyCode::W), "X" => Some(KeyCode::X),
        "Y" => Some(KeyCode::Y), "Z" => Some(KeyCode::Z),
        "LEFT" => Some(KeyCode::Left), "RIGHT" => Some(KeyCode::Right),
        "UP" => Some(KeyCode::Up), "DOWN" => Some(KeyCode::Down),
        "SPACE" => Some(KeyCode::Space), "ENTER" => Some(KeyCode::Enter), "TAB" => Some(KeyCode::Tab),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BPM120: ScrollSpeed = ScrollSpeed { bpm: 120.0, scroll: 1.0 };

    #[test]
    fn object_at_hit_time_is_on_judge_circle() {
        assert_eq!(scroll_x(0.0, BPM120), TARGET_X);
    }

    #[test]
    fn one_beat_ahead_is_px_per_beat_away() {
        // BPM 120: one beat = 0.5s.
        assert_eq!(scroll_x(0.5, BPM120), TARGET_X + PX_PER_BEAT);
        assert_eq!(scroll_x(-0.5, BPM120), TARGET_X - PX_PER_BEAT);
    }

    #[test]
    fn speed_scales_with_bpm_and_scroll() {
        let base = scroll_x(0.5, BPM120) - TARGET_X;
        let bpm240 = scroll_x(0.5, ScrollSpeed { bpm: 240.0, scroll: 1.0 }) - TARGET_X;
        let scroll_half = scroll_x(0.5, ScrollSpeed { bpm: 120.0, scroll: 0.5 }) - TARGET_X;
        assert_eq!(bpm240, base * 2.0);
        assert_eq!(scroll_half, base * 0.5);
    }

    #[test]
    fn visibility_includes_partially_overlapping_notes() {
        assert!(is_x_visible(LANE_X - 10.0, NOTE_RADIUS));
        assert!(!is_x_visible(LANE_X - NOTE_RADIUS - 1.0, NOTE_RADIUS));
        assert!(is_x_visible(LANE_X + LANE_W + 10.0, NOTE_RADIUS));
        assert!(!is_x_visible(LANE_X + LANE_W + NOTE_RADIUS + 1.0, NOTE_RADIUS));
    }
}
