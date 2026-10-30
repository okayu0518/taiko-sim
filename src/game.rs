use std::path::PathBuf;
use std::{fs::File, io::BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::Instant;

use macroquad::prelude::*;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

use crate::models::{AppConfig, KeybindConfig, NoteKind, ScoreState, ScrollSpeed, SongEntry};

use crate::effects::{draw_gogo_flame, draw_gogo_lane, gogo_intensity, note_color, Effects};
use crate::gameplay::{HitOutcome, PlaySession};
use crate::judge::Judge;
use crate::repository::load_song_entries;
use crate::tja_parser::{list_courses, parse_tja, CourseOption};

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
const SONGS_PER_PAGE: usize = 10;
const MENU_LIST_TOP: f32 = 171.0;
const MENU_ROW_SPACING: f32 = 44.0;

fn song_page_start(selected: usize) -> usize {
    (selected / SONGS_PER_PAGE) * SONGS_PER_PAGE
}

fn balloon_position(start_diff: f64, end_diff: f64, speed: ScrollSpeed) -> f32 {
    if start_diff <= 0.0 && end_diff >= 0.0 {
        TARGET_X
    } else {
        scroll_x(start_diff, speed)
    }
}

enum Scene {
    SongSelect,
    DifficultySelect,
    Play,
    Result,
}

/// D/F/J/K handle all menu navigation, independent of the gameplay keybinds.
#[derive(Debug, PartialEq, Eq)]
enum MenuAction {
    Previous,
    Next,
    Confirm,
    Back,
}

fn menu_action(pressed: impl Fn(KeyCode) -> bool) -> Option<MenuAction> {
    if pressed(KeyCode::F) {
        Some(MenuAction::Back)
    } else if pressed(KeyCode::J) {
        Some(MenuAction::Confirm)
    } else if pressed(KeyCode::D) {
        Some(MenuAction::Previous)
    } else if pressed(KeyCode::K) {
        Some(MenuAction::Next)
    } else {
        None
    }
}

/// Capture individual non-repeat keydown events in arrival order, including
/// repeated presses of the same key between two rendered frames.
#[derive(Default)]
struct DrumInputEvents {
    pressed: Vec<KeyCode>,
}

impl macroquad::miniquad::EventHandler for DrumInputEvents {
    fn update(&mut self) {}
    fn draw(&mut self) {}

    fn key_down_event(
        &mut self,
        key: KeyCode,
        _mods: macroquad::miniquad::KeyMods,
        repeat: bool,
    ) {
        if !repeat {
            self.pressed.push(key);
        }
    }
}

#[derive(Clone)]
struct SeData {
    samples: Arc<[f32]>,
    sample_rate: u32,
    channels: u16,
}

struct SeSource {
    data: Arc<[f32]>,
    sample_rate: u32,
    channels: u16,
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
    fn channels(&self) -> u16 { self.channels }
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

    fn kind_for_key(&self, key: KeyCode) -> Option<NoteKind> {
        if key == self.don_left || key == self.don_right {
            Some(NoteKind::Don)
        } else if key == self.kat_left || key == self.kat_right {
            Some(NoteKind::Kat)
        } else {
            None
        }
    }

    fn inputs_from_events(&self, events: &[KeyCode]) -> Vec<NoteKind> {
        events.iter().filter_map(|&key| self.kind_for_key(key)).collect()
    }
}

pub struct TaikoSimApp {
    root_dir: PathBuf,
    songs: Vec<SongEntry>,
    selected_idx: usize,
    courses: Vec<CourseOption>,
    selected_course_idx: usize,
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
        // Allow users to drop TJA files in a predictable place on first launch.
        let _ = std::fs::create_dir_all(data_dir.join("songs"));
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
            courses: Vec::new(),
            selected_course_idx: 0,
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

        // Unlike is_key_pressed (a HashSet per frame), this captures every
        // physical press event separately and preserves the arrival order.
        let input_subscriber = macroquad::input::utils::register_input_subscriber();
        let mut drum_events = DrumInputEvents::default();

        loop {
            drum_events.pressed.clear();
            macroquad::input::utils::repeat_all_miniquad_input(&mut drum_events, input_subscriber);

            if is_key_pressed(KeyCode::Escape) {
                match self.scene {
                    Scene::SongSelect | Scene::Result => break,
                    Scene::DifficultySelect => self.scene = Scene::SongSelect,
                    Scene::Play => self.to_result(),
                }
                // Do not let an Escape key affect the new scene in this frame.
                next_frame().await;
                continue;
            }

            match self.scene {
                Scene::SongSelect => {
                    if self.update_song_select().await { break; }
                }
                Scene::DifficultySelect => self.update_difficulty_select().await,
                Scene::Play => self.update_play(&drum_events.pressed),
                Scene::Result => self.update_result().await,
            }

            match self.scene {
                Scene::SongSelect => self.draw_song_select(),
                Scene::DifficultySelect => self.draw_difficulty_select(),
                Scene::Play => self.draw_play(),
                Scene::Result => self.draw_result(),
            }

            next_frame().await;
        }
    }

    /// Returns true when the user wants to leave the application.
    async fn update_song_select(&mut self) -> bool {
        let action = menu_action(is_key_pressed);
        if matches!(action, Some(MenuAction::Back)) {
            return true;
        }

        if matches!(action, Some(MenuAction::Confirm)) || is_key_pressed(KeyCode::Enter) {
            if self.songs.is_empty() {
                // Songs may have been copied while the game was running.
                self.songs = load_song_entries(&self.root_dir.join("data"));
                self.selected_idx = 0;
            } else {
                self.open_difficulty_select();
            }
            return false;
        }

        if !self.songs.is_empty() {
            if matches!(action, Some(MenuAction::Previous)) || is_key_pressed(KeyCode::Up) {
                self.selected_idx = if self.selected_idx == 0 { self.songs.len() - 1 } else { self.selected_idx - 1 };
            } else if matches!(action, Some(MenuAction::Next)) || is_key_pressed(KeyCode::Down) {
                self.selected_idx = (self.selected_idx + 1) % self.songs.len();
            }
        }
        false
    }

    fn update_play(&mut self, pressed: &[KeyCode]) {
        if self.session.is_none() { return; }

        let now = self.current_time_sec();
        let offset = self.timing_offset_sec();
        if let Some(session) = self.session.as_mut() { session.check_branch_points(now, offset); }

        self.effects.update(now);

        for kind in self.keybinds.inputs_from_events(pressed) {
            self.play_key_se(kind);
            self.effects.spawn_input(now, kind);
            self.try_hit(now, kind);
        }
        if is_key_pressed(KeyCode::LeftBracket) { self.adjust_timing_offset(-5); }
        if is_key_pressed(KeyCode::RightBracket) { self.adjust_timing_offset(5); }

        if let Some(sink) = &self.sink {
            sink.set_volume(self.config.music_volume.clamp(0.0, 2.0));
        }

        let offset = self.timing_offset_sec();
        let Some(session) = self.session.as_mut() else { return; };
        let missed = session.update(now, offset);
        let finished = session.is_finished(now, offset);
        if missed > 0 { self.effects.spawn_judge_text(now, Judge::Fuka); }
        if finished { self.to_result(); }
    }

    async fn update_result(&mut self) {
        if is_key_pressed(KeyCode::F) || is_key_pressed(KeyCode::Enter) {
            self.scene = Scene::SongSelect;
        } else if is_key_pressed(KeyCode::J) {
            self.start_selected_song().await;
        }
    }

    fn open_difficulty_select(&mut self) {
        self.error_message.clear();
        let Some(song) = self.songs.get(self.selected_idx) else { return; };
        match list_courses(&song.tja_path) {
            Ok(courses) => {
                self.selected_course_idx = courses.iter().position(|c| c.name == "Oni").unwrap_or(0);
                self.courses = courses;
                self.scene = Scene::DifficultySelect;
            }
            Err(e) => self.error_message = format!("難易度読み込み失敗: {e}"),
        }
    }

    async fn update_difficulty_select(&mut self) {
        match menu_action(is_key_pressed) {
            Some(MenuAction::Back) => self.scene = Scene::SongSelect,
            Some(MenuAction::Confirm) => self.start_selected_song().await,
            Some(MenuAction::Previous) => {
                if !self.courses.is_empty() {
                    self.selected_course_idx = if self.selected_course_idx == 0 {
                        self.courses.len() - 1
                    } else {
                        self.selected_course_idx - 1
                    };
                }
            }
            Some(MenuAction::Next) => {
                if !self.courses.is_empty() {
                    self.selected_course_idx = (self.selected_course_idx + 1) % self.courses.len();
                }
            }
            None => {}
        }
    }

    async fn start_selected_song(&mut self) {
        self.error_message.clear();
        let Some(song) = self.songs.get(self.selected_idx).cloned() else { return; };

        let Some(course) = self.courses.get(self.selected_course_idx) else {
            self.error_message = "難易度が選択されていません".to_owned();
            return;
        };
        let course_id = course.id.clone();
        self.ensure_bgm_output();
        let (chart, wave) = match parse_tja(&song.tja_path, Some(&course_id)) {
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
        self.draw_menu_header("SONG SELECT", "D/K: change song   J: difficulty   F: quit");

        if self.songs.is_empty() {
            self.draw_ui_text("No songs found", 100.0, 250.0, 34, WHITE);
            self.draw_ui_text("Add TJA and audio to data/songs/<song>/", 100.0, 304.0, 24, Color::from_rgba(165, 183, 210, 255));
            self.draw_ui_text("Press J to refresh the song list", 100.0, 344.0, 24, Color::from_rgba(255, 207, 118, 255));
            return;
        }

        let start = song_page_start(self.selected_idx);
        let end = (start + SONGS_PER_PAGE).min(self.songs.len());
        for (row, idx) in (start..end).enumerate() {
            let song = &self.songs[idx];
            let selected = idx == self.selected_idx;
            let baseline = MENU_LIST_TOP + row as f32 * MENU_ROW_SPACING + 29.0;
            let y = baseline - 30.0;
            self.draw_menu_row(y, selected);

            let number_color = if selected {
                Color::from_rgba(255, 216, 136, 255)
            } else {
                Color::from_rgba(117, 139, 170, 255)
            };
            self.draw_ui_text(&format!("{:02}", idx + 1), 98.0, baseline, 21, number_color);
            self.draw_ui_fitted_text(
                &song.title, 161.0, baseline, 27, 840.0,
                if selected { WHITE } else { Color::from_rgba(203, 215, 235, 255) },
            );
            if selected {
                self.draw_ui_text("SELECTED", 1024.0, baseline, 18, Color::from_rgba(255, 207, 118, 255));
            }
        }

        let page = start / SONGS_PER_PAGE + 1;
        let pages = self.songs.len().div_ceil(SONGS_PER_PAGE);
        self.draw_menu_footer(&format!(
            "{} songs    Page {}/{}    Selected {}/{}",
            self.songs.len(), page, pages, self.selected_idx + 1, self.songs.len()
        ));

        if !self.error_message.is_empty() {
            self.draw_ui_fitted_text(
                &self.error_message, 77.0, 622.0, 21, 1100.0,
                Color::from_rgba(255, 141, 141, 255)
            );
        }
    }

    fn draw_difficulty_select(&self) {
        self.draw_menu_header("DIFFICULTY", "D/K: choose difficulty   J: play   F: back");

        if let Some(song) = self.songs.get(self.selected_idx) {
            self.draw_ui_fitted_text(
                &song.title, 93.0, 190.0, 29, 1060.0,
                Color::from_rgba(224, 232, 248, 255),
            );
        }

        for (i, course) in self.courses.iter().enumerate() {
            let selected = i == self.selected_course_idx;
            let baseline = 252.0 + 63.0 * i as f32;
            self.draw_menu_row(baseline - 34.0, selected);
            self.draw_ui_fitted_text(
                &course.name, 119.0, baseline, 31, 720.0,
                if selected { WHITE } else { Color::from_rgba(194, 210, 230, 255) },
            );
            self.draw_ui_text(
                &format!("Lv {}", course.level), 970.0, baseline, 27,
                if selected { Color::from_rgba(255, 210, 125, 255) }
                else { Color::from_rgba(143, 163, 193, 255) },
            );
        }
        self.draw_menu_footer("Only difficulties present in this TJA file are shown");

        if !self.error_message.is_empty() {
            self.draw_ui_fitted_text(
                &self.error_message, 77.0, 621.0, 21, 1100.0,
                Color::from_rgba(255, 141, 141, 255)
            );
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
        let chart_now = session.chart_time(now, offset);
        let gogo = gogo_intensity(chart_now, &session.active_gogo_ranges());
        draw_rectangle(lane_rect.x, lane_rect.y, lane_rect.w, lane_rect.h, Color::from_rgba(39, 45, 56, 255));
        draw_gogo_lane(lane_rect, gogo);
        self.effects.draw_under_notes(now, lane_rect);
        draw_gogo_flame(target, JUDGE_RING_RADIUS, gogo, now);
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
        let course_name = self.courses.get(self.selected_course_idx)
            .map(|c| c.name.as_str())
            .unwrap_or(chart.course.as_str());
        self.draw_ui_fitted_text(&format!("{} [{} Lv{}]{}", chart.title, course_name, chart.level, branch_name), 40.0, 56.0, 35, 930.0, WHITE);
        self.draw_ui_text(&format!("Score: {}", session.score.score), 320.0, 570.0, 36, WHITE);
        self.draw_ui_text(&session.score.combo.to_string(), TARGET_X - 120.0, LANE_Y - 56.0, 54, Color::from_rgba(255, 240, 170, 255));
        self.draw_ui_text("F/J: Don  D/K: Kat  ESC: result", 40.0, 630.0, 30, Color::from_rgba(170, 180, 200, 255));
        self.draw_ui_text(&format!("Offset: {}ms ([ / ])", self.config.timing_offset_ms), 900.0, 570.0, 30, Color::from_rgba(220, 230, 255, 255));

        if let Some(roll) = session.active_roll(now, offset) {
            let label = if roll.balloon { "BALLOON" } else { "ROLL" };
            self.draw_ui_text(&format!("{}  {}", label, session.roll_hit_count),
                TARGET_X - 110.0, LANE_Y - 82.0, 25,
                Color::from_rgba(255, 232, 161, 255));
        }

        if self.config.debug {
            let (last_ms, avg_ms, max_ms, n) = self.se_latency_stats.snapshot_ms();
            self.draw_ui_text(&format!("FPS: {}", get_fps()), 1030.0, 40.0, 24, Color::from_rgba(170, 255, 170, 255));
            self.draw_ui_text(&format!("Time: {:.3}s", now), 1030.0, 68.0, 24, Color::from_rgba(170, 255, 170, 255));
            self.draw_ui_text(&format!("SE source-read ms  last:{:.1} avg:{:.1} max:{:.1} n:{}", last_ms, avg_ms, max_ms, n), 700.0, 96.0, 24, Color::from_rgba(170, 255, 170, 255));
            if let Some(se) = &self.se_don {
                self.draw_ui_text(&format!("SE SampleRate: {}Hz", se.sample_rate), 700.0, 124.0, 24, Color::from_rgba(170, 255, 170, 255));
            }
            if chart.has_branch {
                let section = session.section;
                self.draw_ui_text(&format!("Section: 良:{} 可:{} 不可:{} R:{}", section.perfect, section.good, section.miss, section.rolls), 700.0, 152.0, 24, Color::from_rgba(170, 255, 170, 255));
                self.draw_ui_text(&format!("Accuracy: {:.1}%", section.accuracy()), 700.0, 180.0, 24, Color::from_rgba(170, 255, 170, 255));
            }
            let gogo_label = if session.is_gogo_at(chart_now) { "ON" } else { "off" };
            self.draw_ui_text(&format!("Gogo: {gogo_label}"), 700.0, 208.0, 24, Color::from_rgba(170, 255, 170, 255));
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
            let body_left = start_x.min(end_x).max(LANE_X);
            let body_right = start_x.max(end_x).min(LANE_X + LANE_W);

            if roll.balloon {
                // Balloons are a separate note type, not yellow drumroll bars.
                let x = balloon_position(start_diff, end_diff, roll.head_speed);
                let active = start_diff <= 0.0 && end_diff >= 0.0;
                if active {
                    // Show the remaining duration as a thin timer tail.
                    let tail_right = end_x.clamp(TARGET_X, LANE_X + LANE_W);
                    draw_rectangle(
                        TARGET_X, LANE_Y + 48.0,
                        (tail_right - TARGET_X).max(0.0), 5.0,
                        Color::from_rgba(255, 185, 75, 190),
                    );
                } else if body_right > body_left {
                    draw_rectangle(
                        body_left, LANE_Y + 48.0, body_right - body_left, 5.0,
                        Color::from_rgba(255, 185, 75, 120),
                    );
                }
                if is_x_visible(x, 60.0) {
                    draw_balloon(x, LANE_Y, active, now);
                }
                continue;
            }

            if start_x.max(end_x) < LANE_X || start_x.min(end_x) > LANE_X + LANE_W { continue; }

            let color = if roll.big {
                Color::from_rgba(255, 152, 60, 255)
            } else {
                Color::from_rgba(255, 207, 85, 255)
            };
            if body_right > body_left {
                draw_rectangle(body_left, LANE_Y - 12.0, body_right - body_left, 24.0, color);
                draw_rectangle(body_left, LANE_Y - 10.0, body_right - body_left, 6.0,
                    Color::from_rgba(255, 243, 179, 170));
            }
            draw_circle(start_x, LANE_Y, if roll.big { 22.0 } else { 17.0 }, color);
            draw_circle(end_x, LANE_Y, 16.0, color);
            draw_circle_lines(start_x, LANE_Y, if roll.big { 22.0 } else { 17.0 }, 2.5,
                Color::from_rgba(117, 68, 33, 255));
            draw_circle_lines(end_x, LANE_Y, 16.0, 2.5, Color::from_rgba(117, 68, 33, 255));
        }
    }

    fn draw_result(&self) {
        self.draw_menu_header("RESULT", "F: song select   J: replay   ESC: quit");
        let song_name = self.current_song.as_ref()
            .map(|s| s.title.as_str()).unwrap_or("Unknown");
        let course = self.courses.get(self.selected_course_idx)
            .map(|c| c.name.as_str()).unwrap_or("Unknown");
        self.draw_ui_fitted_text(song_name, 92.0, 205.0, 34, 955.0, WHITE);
        self.draw_ui_text(&format!("Difficulty: {}", course), 95.0, 245.0, 23,
            Color::from_rgba(169, 191, 222, 255));

        let default_score = ScoreState::default();
        let score = self.session.as_ref().map_or(&default_score, |s| &s.score);
        self.draw_ui_text("SCORE", 96.0, 316.0, 20,
            Color::from_rgba(155, 175, 200, 255));
        self.draw_ui_text(&score.score.to_string(), 95.0, 382.0, 56,
            Color::from_rgba(255, 212, 132, 255));

        let stats = [
            ("MAX COMBO", score.max_combo),
            ("RYO", score.perfect),
            ("KA", score.good),
            ("MISS", score.miss),
        ];
        for (i, (name, value)) in stats.iter().enumerate() {
            let x = 93.0 + i as f32 * 275.0;
            self.draw_ui_rect(x, 436.0, 250.0, 100.0, Color::from_rgba(39, 54, 75, 255));
            self.draw_ui_text(name, x + 17.0, 473.0, 20,
                Color::from_rgba(153, 177, 207, 255));
            self.draw_ui_text(&value.to_string(), x + 17.0, 515.0, 33, WHITE);
        }
        self.draw_menu_footer("Press J to retry the selected difficulty");
    }

    fn draw_menu_header(&self, title: &str, controls: &str) {
        clear_background(Color::from_rgba(17, 23, 35, 255));
        self.draw_ui_rect(0.0, 0.0, BASE_W, 7.0, Color::from_rgba(255, 178, 98, 255));
        self.draw_ui_rect(60.0, 139.0, 1160.0, 476.0, Color::from_rgba(29, 39, 56, 255));
        self.draw_ui_rect(60.0, 139.0, 5.0, 476.0, Color::from_rgba(255, 178, 98, 255));
        self.draw_ui_text(title, 68.0, 69.0, 44, WHITE);
        self.draw_ui_fitted_text(controls, 71.0, 108.0, 20, 1120.0,
            Color::from_rgba(169, 190, 216, 255));
    }

    fn draw_menu_row(&self, top: f32, selected: bool) {
        self.draw_ui_rect(
            84.0, top, 1109.0, 40.0,
            if selected { Color::from_rgba(69, 81, 103, 255) }
            else { Color::from_rgba(36, 48, 67, 255) },
        );
        if selected {
            self.draw_ui_rect(84.0, top, 5.0, 40.0,
                Color::from_rgba(255, 195, 105, 255));
        }
    }

    fn draw_menu_footer(&self, text: &str) {
        self.draw_ui_fitted_text(text, 81.0, 675.0, 19, 1118.0,
            Color::from_rgba(158, 180, 207, 255));
    }

    fn draw_ui_rect(&self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let sx = screen_width() / BASE_W;
        let sy = screen_height() / BASE_H;
        draw_rectangle(x * sx, y * sy, w * sx, h * sy, color);
    }

    /// Limit the rendered width to a measured pixel budget, preserving Unicode
    /// character boundaries when inserting an ellipsis.
    fn draw_ui_fitted_text(
        &self, text: &str, x: f32, y: f32, font_size: u16,
        max_width: f32, color: Color,
    ) {
        let sx = screen_width() / BASE_W;
        let sy = screen_height() / BASE_H;
        let scaled_size = ((font_size as f32 * sy).round() as u16).max(1);
        let font = self.ui_font.as_ref();
        let fits = |value: &str| measure_text(value, font, scaled_size, 1.0).width <= max_width * sx;
        if fits(text) {
            self.draw_ui_text(text, x, y, font_size, color);
            return;
        }
        let mut clipped = text.to_owned();
        while !clipped.is_empty() {
            clipped.pop();
            let candidate = format!("{}...", clipped);
            if fits(&candidate) {
                self.draw_ui_text(&candidate, x, y, font_size, color);
                return;
            }
        }
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
            channels: se_data.channels,
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

/// Balloon/firmament-style note. The shape stays at the hit target while active.
fn draw_balloon(x: f32, y: f32, active: bool, time: f64) {
    let pulse = if active { ((time * 13.0).sin() as f32 + 1.0) * 2.0 } else { 0.0 };
    let w = 47.0 + pulse;
    let h = 36.0 + pulse * 0.7;
    let outline = Color::from_rgba(108, 56, 46, 255);
    // Shadow and the tied knot are separate from the oval so the silhouette
    // can never be mistaken for an ordinary long roll.
    draw_ellipse(x + 2.5, y + 3.0, w + 3.0, h + 3.0, 0.0,
        Color::from_rgba(66, 44, 51, 180));
    draw_ellipse(x, y, w, h, 0.0, Color::from_rgba(255, 150, 78, 255));
    draw_ellipse(x - 6.0, y - 7.0, w - 9.0, h - 8.0, 0.0,
        Color::from_rgba(255, 193, 100, 255));
    draw_ellipse_lines(x, y, w, h, 0.0, 3.0, outline);
    draw_ellipse(x - 16.0, y - 15.0, 13.0, 7.0, -25.0,
        Color::from_rgba(255, 246, 207, 180));
    draw_triangle(
        vec2(x - 7.0, y + h - 3.0), vec2(x + 7.0, y + h - 3.0),
        vec2(x, y + h + 11.0), Color::from_rgba(255, 164, 88, 255),
    );
    draw_line(x, y + h + 10.0, x + 6.0, y + h + 19.0, 2.0,
        Color::from_rgba(255, 217, 143, 255));
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
    let channels = decoder.channels();
    let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
    Some(SeData { samples: Arc::from(samples.into_boxed_slice()), sample_rate, channels })
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
    fn menu_controls_only_use_dfjk() {
        assert_eq!(menu_action(|k| k == KeyCode::D), Some(MenuAction::Previous));
        assert_eq!(menu_action(|k| k == KeyCode::K), Some(MenuAction::Next));
        assert_eq!(menu_action(|k| k == KeyCode::J), Some(MenuAction::Confirm));
        assert_eq!(menu_action(|k| k == KeyCode::F), Some(MenuAction::Back));
        assert_eq!(menu_action(|k| k == KeyCode::Enter), None);
    }

    #[test]
    fn key_events_preserve_repeated_taps_in_one_frame() {
        let keys = Keybinds::from_config(&KeybindConfig::default());
        let events = [KeyCode::F, KeyCode::F, KeyCode::D, KeyCode::K, KeyCode::J];
        assert_eq!(keys.inputs_from_events(&events), vec![
            NoteKind::Don, NoteKind::Don, NoteKind::Kat,
            NoteKind::Kat, NoteKind::Don,
        ]);
    }

    #[test]
    fn auto_repeat_events_are_not_counted_as_drums() {
        let mut events = DrumInputEvents::default();
        macroquad::miniquad::EventHandler::key_down_event(
            &mut events, KeyCode::F, macroquad::miniquad::KeyMods::default(), false
        );
        macroquad::miniquad::EventHandler::key_down_event(
            &mut events, KeyCode::F, macroquad::miniquad::KeyMods::default(), true
        );
        assert_eq!(events.pressed, vec![KeyCode::F]);
    }

    #[test]
    fn left_right_and_mixed_same_frame_inputs_are_independent() {
        let keys = Keybinds::from_config(&KeybindConfig::default());
        assert_eq!(keys.inputs_from_events(&[KeyCode::F, KeyCode::J]), vec![NoteKind::Don, NoteKind::Don]);
        assert_eq!(keys.inputs_from_events(&[KeyCode::F, KeyCode::K]), vec![NoteKind::Don, NoteKind::Kat]);
    }

    #[test]
    fn duplicate_mapping_does_not_generate_multiple_inputs() {
        let keys = Keybinds {
            don_left: KeyCode::F,
            don_right: KeyCode::F,
            kat_left: KeyCode::D,
            kat_right: KeyCode::K,
        };
        assert_eq!(keys.inputs_from_events(&[KeyCode::F]), vec![NoteKind::Don]);
    }

    #[test]
    fn stereo_se_preserves_channel_count() {
        let source = SeSource {
            data: Arc::from(vec![0.1_f32, -0.1, 0.2, -0.2].into_boxed_slice()),
            sample_rate: 48_000,
            channels: 2,
            idx: 0,
            gain: 1.0,
            triggered_at: Instant::now(),
            recorded: false,
            latency_stats: Arc::new(SeLatencyStats::new()),
        };
        assert_eq!(source.channels(), 2);
        assert_eq!(source.sample_rate(), 48_000);
        assert_eq!(source.count(), 4);
    }

    #[test]
    fn song_pages_are_bounded_and_follow_selection() {
        assert_eq!(song_page_start(0), 0);
        assert_eq!(song_page_start(9), 0);
        assert_eq!(song_page_start(10), 10);
        assert_eq!(song_page_start(27), 20);
    }

    #[test]
    fn balloon_stays_at_target_while_active() {
        assert_eq!(balloon_position(0.5, 1.5, BPM120), scroll_x(0.5, BPM120));
        assert_eq!(balloon_position(-0.25, 0.5, BPM120), TARGET_X);
        assert_eq!(balloon_position(-1.0, -0.25, BPM120), scroll_x(-1.0, BPM120));
    }

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
