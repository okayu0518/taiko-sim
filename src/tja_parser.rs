use std::fs;
use std::path::Path;

use encoding_rs::{SHIFT_JIS, UTF_8};

use crate::models::{Branch, BranchCondition, BranchPoint, Chart, GogoRange, LevelHold, Note, NoteKind, Roll, ScrollSpeed};

const COURSE_PRIORITY: [&str; 5] = ["oni", "hard", "normal", "easy", "edit"];

/// A selectable difficulty defined by a TJA `#START` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CourseOption {
    pub id: String,
    pub name: String,
    pub level: i32,
}

fn normalized_course(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "0" | "easy" => "easy".to_owned(),
        "1" | "normal" => "normal".to_owned(),
        "2" | "hard" => "hard".to_owned(),
        "3" | "oni" => "oni".to_owned(),
        "4" | "edit" | "ura" => "edit".to_owned(),
        other => other.to_owned(),
    }
}

fn display_course(raw: &str) -> String {
    match normalized_course(raw).as_str() {
        "easy" => "Easy".to_owned(),
        "normal" => "Normal".to_owned(),
        "hard" => "Hard".to_owned(),
        "oni" => "Oni".to_owned(),
        "edit" => "Ura (Edit)".to_owned(),
        _ => raw.to_owned(),
    }
}

/// List available courses without building all charts.
pub fn list_courses(path: &Path) -> Result<Vec<CourseOption>, String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let text = decode_tja_bytes(bytes);
    list_courses_text(&text)
}

fn list_courses_text(text: &str) -> Result<Vec<CourseOption>, String> {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let (blocks, _) = extract_course_blocks(&lines);
    if blocks.is_empty() {
        return Err("No #START/#END block found".to_owned());
    }

    let mut courses = Vec::new();
    for b in blocks {
        let id = b.headers.get("COURSE").cloned().unwrap_or_else(|| "Oni".to_owned());
        if courses.iter().any(|c: &CourseOption| normalized_course(&c.id) == normalized_course(&id)) {
            continue;
        }
        let level = b.headers.get("LEVEL")
            .and_then(|s| s.trim().parse::<i32>().ok())
            .unwrap_or(0);
        courses.push(CourseOption { name: display_course(&id), id, level });
    }
    Ok(courses)
}

#[derive(Clone)]
struct CourseBlock {
    headers: std::collections::HashMap<String, String>,
    body_lines: Vec<String>,
}

pub fn parse_tja(path: &Path, preferred_course: Option<&str>) -> Result<(Chart, Option<String>), String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let text = decode_tja_bytes(bytes);
    parse_tja_text(&text, preferred_course).map_err(|e| format!("{} in {}", e, path.display()))
}

/// Parse TJA source text. Returns the chart and the `WAVE` header, if any.
fn parse_tja_text(text: &str, preferred_course: Option<&str>) -> Result<(Chart, Option<String>), String> {
    let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

    let (blocks, global_headers) = extract_course_blocks(&lines);
    if blocks.is_empty() {
        return Err("No #START/#END block found".to_string());
    }

    let selected = select_block(&blocks, preferred_course);
    let mut headers = global_headers;
    for (k, v) in &selected.headers {
        headers.insert(k.clone(), v.clone());
    }

    let chart = build_chart(&headers, &selected.body_lines)?;
    let wave = headers.get("WAVE").cloned();
    Ok((chart, wave))
}

/// Decode TJA bytes as UTF-8 (BOM stripped), falling back to Shift_JIS.
fn decode_tja_bytes(bytes: Vec<u8>) -> String {
    let bytes = match String::from_utf8(bytes) {
        Ok(s) => return strip_bom(s),
        Err(e) => e.into_bytes(),
    };

    let (decoded_sjis, _, had_errors_sjis) = SHIFT_JIS.decode(&bytes);
    if !had_errors_sjis {
        return decoded_sjis.to_string();
    }

    let (decoded_utf8, _, _) = UTF_8.decode(&bytes);
    strip_bom(decoded_utf8.to_string())
}

fn strip_bom(mut s: String) -> String {
    if s.starts_with('\u{feff}') {
        s.remove(0);
    }
    s
}

fn extract_course_blocks(
    lines: &[String],
) -> (
    Vec<CourseBlock>,
    std::collections::HashMap<String, String>,
) {
    let mut global_headers = std::collections::HashMap::new();
    let mut current_headers = std::collections::HashMap::new();
    let mut blocks = Vec::new();

    let mut in_block = false;
    let mut body = Vec::new();

    for raw in lines {
        let line = clean_line(raw);
        if line.is_empty() {
            continue;
        }

        if !in_block && line.contains(':') && !line.starts_with('#') {
            let mut split = line.splitn(2, ':');
            let key = split.next().unwrap_or("").trim().to_uppercase();
            let value = split.next().unwrap_or("").trim().to_string();
            current_headers.insert(key.clone(), value.clone());
            if !current_headers.contains_key("COURSE") {
                global_headers.insert(key, value);
            }
            continue;
        }

        let upper = line.to_uppercase();
        if upper == "#START" {
            in_block = true;
            body.clear();
            continue;
        }
        if upper == "#END" && in_block {
            in_block = false;
            blocks.push(CourseBlock {
                headers: current_headers.clone(),
                body_lines: body.clone(),
            });
            continue;
        }

        if in_block {
            body.push(line);
        }
    }

    (blocks, global_headers)
}

fn select_block<'a>(blocks: &'a [CourseBlock], preferred_course: Option<&str>) -> &'a CourseBlock {
    if let Some(pref) = preferred_course {
        let p = pref.trim().to_lowercase();
        for b in blocks {
            if b
                .headers
                .get("COURSE")
                .map(|c| normalized_course(c) == normalized_course(&p))
                .unwrap_or(false)
            {
                return b;
            }
        }
    }

    for name in COURSE_PRIORITY {
        for b in blocks {
            if b
                .headers
                .get("COURSE")
                .map(|c| normalized_course(c) == name)
                .unwrap_or(false)
            {
                return b;
            }
        }
    }

    &blocks[0]
}

fn build_chart(
    headers: &std::collections::HashMap<String, String>,
    body_lines: &[String],
) -> Result<Chart, String> {
    let title = headers.get("TITLE").cloned().unwrap_or_else(|| "Unknown".to_string());
    let course = headers
        .get("COURSE")
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());
    let level = headers
        .get("LEVEL")
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let bpm = headers
        .get("BPM")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(120.0);
    let offset_sec = headers
        .get("OFFSET")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(0.0);

    if bpm <= 0.0 {
        return Err("Invalid BPM".to_string());
    }

    let mut notes = Vec::new();
    let mut rolls = Vec::new();
    let mut branch_points = Vec::new();
    let mut section_resets = Vec::new();
    let mut level_holds = Vec::new();
    let mut has_branch = false;
    let mut branch_start_time: Option<f64> = None;
    // Gogo state at #BRANCHSTART; every branch starts from it.
    let mut branch_start_gogo = false;
    let mut max_branch_time = 0.0_f64;
    let mut open_rolls: Vec<OpenRoll> = Vec::new();
    let mut timing = TimingState { time: 0.0, bpm, scroll: 1.0, branch: None, gogo_since: None, gogo_ranges: Vec::new() };
    let mut measure_num = 4_i32;
    let mut measure_den = 4_i32;
    let mut measure_items: Vec<MeasureItem> = Vec::new();
    // Length of the last completed measure, used to place branch judgements.
    let mut last_measure_sec = 0.0_f64;

    for line in body_lines {
        if line.starts_with('#') {
            let cmd = line[1..].trim().to_string();
            let cmd_upper = cmd.to_uppercase();
            if let Some(item) = parse_timing_command(&cmd, &cmd_upper) {
                // Inside a measure, timing commands apply from their position.
                if measure_items.iter().any(|i| matches!(i, MeasureItem::Note(_))) {
                    measure_items.push(item);
                } else {
                    timing.apply(&item);
                }
            } else if cmd_upper.starts_with("MEASURE") {
                let value = cmd[7..].trim();
                if let Some((lhs, rhs)) = value.split_once('/') {
                    if let Ok(v) = lhs.trim().parse::<i32>() {
                        measure_num = v.max(1);
                    }
                    if let Ok(v) = rhs.trim().parse::<i32>() {
                        measure_den = v.max(1);
                    }
                }
            } else if cmd_upper.starts_with("BRANCHSTART") {
                // A new branch without #BRANCHEND first merges the previous one.
                if branch_start_time.is_some() {
                    timing.move_to(max_branch_time.max(timing.time), timing.branch, timing.is_gogo());
                }
                branch_start_gogo = timing.is_gogo();
                has_branch = true;
                branch_start_time = Some(timing.time);
                max_branch_time = timing.time;
                branch_points.push(parse_branch_start(&cmd[11..], timing.time, last_measure_sec));
            } else if let Some(branch) = match cmd_upper.as_str() {
                "N" => Some(Branch::Normal),
                "E" => Some(Branch::Expert),
                "M" => Some(Branch::Master),
                _ => None,
            } {
                if let Some(start) = branch_start_time {
                    max_branch_time = max_branch_time.max(timing.time);
                    timing.move_to(start, Some(branch), branch_start_gogo);
                } else {
                    timing.move_to(timing.time, Some(branch), timing.is_gogo());
                }
            } else if cmd_upper == "BRANCHEND" {
                let end = match branch_start_time.take() {
                    Some(_) => max_branch_time.max(timing.time),
                    None => timing.time,
                };
                timing.move_to(end, None, timing.is_gogo());
            } else if cmd_upper == "SECTION" {
                section_resets.push(timing.time);
            } else if cmd_upper == "LEVELHOLD" {
                level_holds.push(LevelHold { time_sec: timing.time, branch: timing.branch });
            }
            continue;
        }

        for ch in line.chars() {
            if ch == ',' {
                let measure_start = timing.time;
                consume_measure(
                    &measure_items,
                    &mut timing,
                    &mut notes,
                    &mut rolls,
                    &mut open_rolls,
                    measure_num,
                    measure_den,
                );
                last_measure_sec = timing.time - measure_start;
                measure_items.clear();
            } else if ch.is_ascii_digit() {
                measure_items.push(MeasureItem::Note(ch));
            }
        }
    }

    if !measure_items.is_empty() {
        consume_measure(
            &measure_items,
            &mut timing,
            &mut notes,
            &mut rolls,
            &mut open_rolls,
            measure_num,
            measure_den,
        );
    }

    while let Some(open) = open_rolls.pop() {
        rolls.push(open.close(timing.time, timing.speed()));
    }
    timing.close_gogo();

    let duration_sec = timing.time + 2.0;
    Ok(Chart {
        title,
        course,
        level,
        offset_sec,
        notes,
        rolls,
        branch_points,
        section_resets,
        level_holds,
        gogo_ranges: timing.gogo_ranges,
        has_branch,
        duration_sec,
    })
}

/// Parse `#BRANCHSTART` arguments (`type, expert, master`).
///
/// The branch is judged at the bar line one measure before `start_time_sec`.
fn parse_branch_start(args: &str, start_time_sec: f64, previous_measure_sec: f64) -> BranchPoint {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    let threshold = |i: usize| parts.get(i).and_then(|v| v.parse::<f64>().ok());
    let condition = match parts.first().and_then(|p| p.chars().next()).map(|c| c.to_ascii_lowercase()) {
        Some('r') => Some(BranchCondition::Roll),
        Some('p') => Some(BranchCondition::Precision),
        Some('s') => Some(BranchCondition::Score),
        _ => None,
    };
    let (condition, expert_threshold, master_threshold) = match (condition, threshold(1), threshold(2)) {
        (Some(c), Some(e), Some(m)) => (Some(c), e, m),
        _ => (None, 0.0, 0.0),
    };
    BranchPoint {
        judge_time_sec: (start_time_sec - previous_measure_sec).max(0.0),
        start_time_sec,
        condition,
        expert_threshold,
        master_threshold,
    }
}

/// Current position, branch, scroll and gogo state while reading a chart.
struct TimingState {
    time: f64,
    bpm: f64,
    scroll: f64,
    branch: Option<Branch>,
    /// Start time of the open gogo range.
    gogo_since: Option<f64>,
    gogo_ranges: Vec<GogoRange>,
}

impl TimingState {
    fn speed(&self) -> ScrollSpeed {
        ScrollSpeed { bpm: self.bpm, scroll: self.scroll }
    }

    fn is_gogo(&self) -> bool {
        self.gogo_since.is_some()
    }

    fn apply(&mut self, item: &MeasureItem) {
        match *item {
            MeasureItem::BpmChange(v) => self.bpm = v.max(1e-6),
            MeasureItem::Scroll(v) => self.scroll = v,
            MeasureItem::Delay(v) => self.time += v,
            MeasureItem::GogoStart => {
                self.gogo_since.get_or_insert(self.time);
            }
            MeasureItem::GogoEnd => self.close_gogo(),
            MeasureItem::Note(_) => {}
        }
    }

    /// Close the open gogo range at the current time. Empty ranges are dropped.
    fn close_gogo(&mut self) {
        if let Some(start_sec) = self.gogo_since.take() {
            if self.time > start_sec {
                self.gogo_ranges.push(GogoRange { start_sec, end_sec: self.time, branch: self.branch });
            }
        }
    }

    /// Jump to `time` on `branch` (branch rewind/merge), splitting the gogo range there.
    fn move_to(&mut self, time: f64, branch: Option<Branch>, gogo: bool) {
        self.close_gogo();
        self.time = time;
        self.branch = branch;
        if gogo {
            self.gogo_since = Some(time);
        }
    }
}

/// An element of a measure: a note character or a command placed between notes.
enum MeasureItem {
    Note(char),
    BpmChange(f64),
    Scroll(f64),
    Delay(f64),
    GogoStart,
    GogoEnd,
}

/// Parse commands that may appear inside a measure
/// (`#BPMCHANGE`, `#SCROLL`, `#DELAY`, `#GOGOSTART`, `#GOGOEND`).
fn parse_timing_command(cmd: &str, cmd_upper: &str) -> Option<MeasureItem> {
    let value = |prefix: &str| cmd[prefix.len()..].trim().parse::<f64>().ok();
    if cmd_upper == "GOGOSTART" {
        Some(MeasureItem::GogoStart)
    } else if cmd_upper == "GOGOEND" {
        Some(MeasureItem::GogoEnd)
    } else if cmd_upper.starts_with("BPMCHANGE") {
        Some(MeasureItem::BpmChange(value("BPMCHANGE")?))
    } else if cmd_upper.starts_with("SCROLL") {
        Some(MeasureItem::Scroll(value("SCROLL")?))
    } else if cmd_upper.starts_with("DELAY") {
        Some(MeasureItem::Delay(value("DELAY")?))
    } else {
        None
    }
}

/// A roll whose end (`8`) has not been read yet.
struct OpenRoll {
    start_sec: f64,
    big: bool,
    balloon: bool,
    branch: Option<Branch>,
    head_speed: ScrollSpeed,
}

impl OpenRoll {
    fn close(self, end_sec: f64, tail_speed: ScrollSpeed) -> Roll {
        Roll {
            start_sec: self.start_sec,
            end_sec,
            big: self.big,
            balloon: self.balloon,
            branch: self.branch,
            head_speed: self.head_speed,
            tail_speed,
        }
    }
}

/// Emit the notes of one measure and advance `timing` to the next measure.
///
/// Each note character takes an equal number of beats; its duration in
/// seconds uses the BPM in effect at that character.
fn consume_measure(
    items: &[MeasureItem],
    timing: &mut TimingState,
    notes: &mut Vec<Note>,
    rolls: &mut Vec<Roll>,
    open_rolls: &mut Vec<OpenRoll>,
    measure_num: i32,
    measure_den: i32,
) {
    let measure_beats = 4.0 * (measure_num as f64 / measure_den.max(1) as f64);
    let note_count = items.iter().filter(|i| matches!(i, MeasureItem::Note(_))).count();

    if note_count == 0 {
        for item in items {
            timing.apply(item);
        }
        timing.time += measure_beats * 60.0 / timing.bpm;
        return;
    }

    let beats_per_char = measure_beats / note_count as f64;
    for item in items {
        let MeasureItem::Note(ch) = *item else {
            timing.apply(item);
            continue;
        };

        let note_time = timing.time;
        let speed = timing.speed();
        let branch = timing.branch;
        let gogo = timing.is_gogo();
        let open_roll = |big, balloon| OpenRoll { start_sec: note_time, big, balloon, branch, head_speed: speed };
        match ch {
            '1' | '3' => notes.push(Note {
                time_sec: note_time,
                kind: NoteKind::Don,
                judged: false,
                hit: false,
                big: ch == '3',
                branch,
                speed,
                gogo,
            }),
            '2' | '4' => notes.push(Note {
                time_sec: note_time,
                kind: NoteKind::Kat,
                judged: false,
                hit: false,
                big: ch == '4',
                branch,
                speed,
                gogo,
            }),
            '5' => open_rolls.push(open_roll(false, false)),
            '6' => open_rolls.push(open_roll(true, false)),
            '7' | '9' => open_rolls.push(open_roll(true, true)),
            '8' => {
                if let Some(open) = open_rolls.pop() {
                    rolls.push(open.close(note_time, speed));
                }
            }
            _ => {}
        }
        timing.time += beats_per_char * 60.0 / timing.bpm;
    }
}

fn clean_line(raw: &str) -> String {
    if let Some((left, _)) = raw.split_once("//") {
        left.trim().to_string()
    } else {
        raw.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str, preferred: Option<&str>) -> Chart {
        parse_tja_text(src, preferred).expect("parse failed").0
    }

    /// Parse a single Oni course at BPM 120 (one 4/4 measure = 2.0s).
    fn parse_body(body: &str) -> Chart {
        parse(&format!("BPM:120\nCOURSE:Oni\n#START\n{body}\n#END\n"), None)
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "expected {expected}, got {actual}");
    }

    fn note_times(chart: &Chart) -> Vec<f64> {
        chart.notes.iter().map(|n| n.time_sec).collect()
    }

    fn assert_times(actual: &[f64], expected: &[f64]) {
        assert_eq!(actual.len(), expected.len(), "times: {actual:?}");
        for (a, e) in actual.iter().zip(expected) {
            assert_close(*a, *e);
        }
    }

    // --- headers / course selection ---

    #[test]
    fn headers_are_parsed() {
        let src = "TITLE:Sample\nBPM:150\nWAVE:sample.ogg\nOFFSET:-1.5\n\nCOURSE:Oni\nLEVEL:7\n#START\n1,\n#END\n";
        let (chart, wave) = parse_tja_text(src, None).unwrap();
        assert_eq!(chart.title, "Sample");
        assert_eq!(chart.course, "Oni");
        assert_eq!(chart.level, 7);
        assert_close(chart.offset_sec, -1.5);
        assert_eq!(wave.as_deref(), Some("sample.ogg"));
    }

    #[test]
    fn missing_headers_use_defaults() {
        let chart = parse("#START\n1,1,\n#END\n", None);
        assert_eq!(chart.title, "Unknown");
        assert_eq!(chart.course, "Unknown");
        assert_eq!(chart.level, 0);
        assert_close(chart.offset_sec, 0.0);
        // Default BPM 120 -> 2.0s per measure.
        assert_times(&note_times(&chart), &[0.0, 2.0]);
    }

    #[test]
    fn missing_start_block_is_error() {
        assert!(parse_tja_text("TITLE:x\nBPM:120\n1,\n", None).is_err());
    }

    #[test]
    fn non_positive_bpm_is_error() {
        assert!(parse_tja_text("BPM:0\n#START\n1,\n#END\n", None).is_err());
    }

    const MULTI_COURSE: &str = "BPM:120\n\
        COURSE:Easy\nLEVEL:2\n#START\n1,\n#END\n\
        COURSE:Hard\nLEVEL:5\n#START\n1,\n#END\n\
        COURSE:Oni\nLEVEL:9\n#START\n1,\n#END\n";

    #[test]
    fn preferred_course_is_selected_case_insensitively() {
        let chart = parse(MULTI_COURSE, Some("hard"));
        assert_eq!(chart.course, "Hard");
        assert_eq!(chart.level, 5);
    }

    #[test]
    fn course_priority_is_used_without_preference() {
        assert_eq!(parse(MULTI_COURSE, None).course, "Oni");
        let src = "COURSE:Easy\n#START\n1,\n#END\nCOURSE:Normal\n#START\n1,\n#END\n";
        assert_eq!(parse(src, None).course, "Normal");
    }

    #[test]
    fn difficulty_list_includes_ura_with_levels() {
        let tja = "TITLE:Test\nCOURSE:0\nLEVEL:2\n#START\n1,\n#END\nCOURSE:3\nLEVEL:9\n#START\n1,\n#END\nCOURSE:4\nLEVEL:10\n#START\n2,\n#END\n";
        let courses = list_courses_text(tja).unwrap();
        assert_eq!(courses, vec![
            CourseOption { id: "0".into(), name: "Easy".into(), level: 2 },
            CourseOption { id: "3".into(), name: "Oni".into(), level: 9 },
            CourseOption { id: "4".into(), name: "Ura (Edit)".into(), level: 10 },
        ]);
        assert_eq!(parse(tja, Some("Ura")).course, "4");
        assert_eq!(parse(tja, Some("Edit")).level, 10);
        assert_eq!(parse(tja, Some("Oni")).level, 9);
    }

    #[test]
    fn duplicate_aliases_collapse_to_one_difficulty() {
        let tja = "COURSE:Edit\nLEVEL:10\n#START\n2,\n#END\nCOURSE:Ura\n#START\n2,\n#END\n";
        assert_eq!(list_courses_text(tja).unwrap().len(), 1);
        assert_eq!(parse(tja, Some("4")).course, "Edit");
    }

    #[test]
    fn no_course_metadata_still_has_a_selectable_default() {
        let courses = list_courses_text("#START\n1,\n#END").unwrap();
        assert_eq!(courses[0].name, "Oni");
        assert_eq!(courses[0].level, 0);
        assert!(list_courses_text("TITLE:Missing").is_err());
    }

    #[test]
    fn missing_preferred_course_falls_back_to_priority() {
        assert_eq!(parse(MULTI_COURSE, Some("Edit")).course, "Oni");
    }

    #[test]
    fn unknown_courses_fall_back_to_first_block() {
        let src = "COURSE:Edit\nLEVEL:10\n#START\n1,\n#END\nCOURSE:Tower\n#START\n1,\n#END\n";
        assert_eq!(parse(src, None).course, "Edit");
    }

    #[test]
    fn course_headers_are_inherited_by_later_courses() {
        let src = "COURSE:Oni\nLEVEL:9\n#START\n1,\n#END\nCOURSE:Hard\n#START\n1,\n#END\n";
        assert_eq!(parse(src, Some("Hard")).level, 9);
    }

    #[test]
    fn global_headers_apply_to_every_course() {
        let src = "TITLE:Song\nCOURSE:Easy\n#START\n1,\n#END\nCOURSE:Oni\n#START\n1,\n#END\n";
        assert_eq!(parse(src, Some("Easy")).title, "Song");
        assert_eq!(parse(src, Some("Oni")).title, "Song");
    }

    // --- text handling ---

    #[test]
    fn comments_are_stripped() {
        let src = "TITLE:Song // comment\n// full line comment\n#START\n1020, // trailing\n#END\n";
        let chart = parse(src, None);
        assert_eq!(chart.title, "Song");
        assert_eq!(chart.notes.len(), 2);
    }

    #[test]
    fn crlf_line_endings_are_supported() {
        let chart = parse("BPM:120\r\n#START\r\n1,\r\n1,\r\n#END\r\n", None);
        assert_times(&note_times(&chart), &[0.0, 2.0]);
    }

    #[test]
    fn utf8_bom_is_stripped() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("TITLE:太鼓".as_bytes());
        assert_eq!(decode_tja_bytes(bytes), "TITLE:太鼓");
    }

    #[test]
    fn shift_jis_is_decoded() {
        let (bytes, _, _) = SHIFT_JIS.encode("TITLE:太鼓の達人");
        assert!(std::str::from_utf8(&bytes).is_err());
        assert_eq!(decode_tja_bytes(bytes.into_owned()), "TITLE:太鼓の達人");
    }

    // --- note timing ---

    #[test]
    fn notes_are_spaced_by_measure_division() {
        let chart = parse_body("1020,\n10002000,");
        let kinds: Vec<_> = chart.notes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, vec![NoteKind::Don, NoteKind::Kat, NoteKind::Don, NoteKind::Kat]);
        assert_times(&note_times(&chart), &[0.0, 1.0, 2.0, 3.0]);
        assert_close(chart.duration_sec, 4.0 + 2.0);
    }

    #[test]
    fn empty_measure_advances_time() {
        let chart = parse_body("1,\n,\n1,");
        assert_times(&note_times(&chart), &[0.0, 4.0]);
    }

    #[test]
    fn measure_can_span_multiple_lines() {
        let chart = parse_body("10\n20,");
        assert_times(&note_times(&chart), &[0.0, 1.0]);
    }

    #[test]
    fn trailing_measure_without_comma_is_parsed() {
        let chart = parse_body("1,\n11");
        assert_times(&note_times(&chart), &[0.0, 2.0, 3.0]);
    }

    #[test]
    fn big_notes_are_flagged() {
        let chart = parse_body("1234,");
        let kinds: Vec<_> = chart.notes.iter().map(|n| (n.kind, n.big)).collect();
        assert_eq!(kinds, vec![
            (NoteKind::Don, false),
            (NoteKind::Kat, false),
            (NoteKind::Don, true),
            (NoteKind::Kat, true),
        ]);
    }

    #[test]
    fn bpm_change_affects_following_measures() {
        let chart = parse_body("1,\n#BPMCHANGE 240\n1,\n1,");
        assert_times(&note_times(&chart), &[0.0, 2.0, 3.0]);
    }

    #[test]
    fn bpm_change_inside_measure_affects_only_following_notes() {
        // First half at BPM 120 (1.0s), second half at BPM 240 (0.5s).
        let chart = parse_body("10\n#BPMCHANGE 240\n10,\n1,");
        assert_times(&note_times(&chart), &[0.0, 1.0, 1.5]);
        assert_eq!(speeds(&chart), vec![(120.0, 1.0), (240.0, 1.0), (240.0, 1.0)]);
    }

    #[test]
    fn scroll_inside_measure_affects_only_following_notes() {
        let chart = parse_body("11\n#SCROLL 2\n11,");
        assert_times(&note_times(&chart), &[0.0, 0.5, 1.0, 1.5]);
        let scrolls: Vec<_> = speeds(&chart).into_iter().map(|(_, s)| s).collect();
        assert_eq!(scrolls, vec![1.0, 1.0, 2.0, 2.0]);
    }

    #[test]
    fn delay_inside_measure_shifts_following_notes() {
        let chart = parse_body("10\n#DELAY 0.25\n10,\n1,");
        assert_times(&note_times(&chart), &[0.0, 1.25, 2.25]);
    }

    #[test]
    fn commands_before_first_note_apply_to_whole_measure() {
        let chart = parse_body("#BPMCHANGE 240\n1010,\n1,");
        assert_times(&note_times(&chart), &[0.0, 0.5, 1.0]);
    }

    #[test]
    fn measure_change_scales_measure_length() {
        let chart = parse_body("#MEASURE 3/4\n1,\n1,\n#MEASURE 4/4\n1,\n1,");
        assert_times(&note_times(&chart), &[0.0, 1.5, 3.0, 5.0]);
    }

    #[test]
    fn delay_shifts_following_notes() {
        let chart = parse_body("1,\n#DELAY 0.5\n1,");
        assert_times(&note_times(&chart), &[0.0, 2.5]);
    }

    fn speeds(chart: &Chart) -> Vec<(f64, f64)> {
        chart.notes.iter().map(|n| (n.speed.bpm, n.speed.scroll)).collect()
    }

    #[test]
    fn scroll_applies_to_following_notes() {
        let chart = parse_body("1,\n#SCROLL 1.5\n1,");
        assert_eq!(speeds(&chart), vec![(120.0, 1.0), (120.0, 1.5)]);
    }

    #[test]
    fn notes_record_bpm_for_scroll_speed() {
        let chart = parse_body("1,\n#BPMCHANGE 240\n1,");
        assert_eq!(speeds(&chart), vec![(120.0, 1.0), (240.0, 1.0)]);
    }

    #[test]
    fn roll_head_and_tail_keep_their_own_speed() {
        let chart = parse_body("5000,\n#BPMCHANGE 240\n#SCROLL 2\n8000,\n6000,");
        let speed = |r: &Roll| ((r.head_speed.bpm, r.head_speed.scroll), (r.tail_speed.bpm, r.tail_speed.scroll));
        assert_eq!(chart.rolls.iter().map(speed).collect::<Vec<_>>(), vec![
            ((120.0, 1.0), (240.0, 2.0)),
            // Unclosed roll ends with the final state.
            ((240.0, 2.0), (240.0, 2.0)),
        ]);
    }

    // --- rolls ---

    #[test]
    fn rolls_are_parsed_with_kind() {
        let chart = parse_body("5000,8000,\n6008,\n7080,\n98,");
        assert!(chart.notes.is_empty());
        let rolls: Vec<_> = chart.rolls.iter().map(|r| (r.start_sec, r.end_sec, r.big, r.balloon)).collect();
        assert_eq!(rolls, vec![
            (0.0, 2.0, false, false),
            (4.0, 5.5, true, false),
            (6.0, 7.0, true, true),
            (8.0, 9.0, true, true),
        ]);
    }

    #[test]
    fn unclosed_roll_ends_at_chart_end() {
        let chart = parse_body("5000,0000,");
        assert_eq!(chart.rolls.len(), 1);
        assert_close(chart.rolls[0].start_sec, 0.0);
        assert_close(chart.rolls[0].end_sec, 4.0);
    }

    #[test]
    fn stray_roll_end_is_ignored() {
        let chart = parse_body("8010,");
        assert!(chart.rolls.is_empty());
        assert_eq!(chart.notes.len(), 1);
    }

    // --- branches ---

    fn branch_point(judge: f64, start: f64, condition: Option<BranchCondition>, expert: f64, master: f64) -> BranchPoint {
        BranchPoint { judge_time_sec: judge, start_time_sec: start, condition, expert_threshold: expert, master_threshold: master }
    }

    #[test]
    fn branches_rewind_time_and_merge_at_longest() {
        let chart = parse_body(
            "1,\n#BRANCHSTART p,80,90\n#N\n1,\n#E\n2,\n#M\n1,1,\n#BRANCHEND\n2,",
        );
        assert!(chart.has_branch);

        let notes: Vec<_> = chart.notes.iter().map(|n| (n.time_sec, n.kind, n.branch)).collect();
        assert_eq!(notes, vec![
            (0.0, NoteKind::Don, None),
            (2.0, NoteKind::Don, Some(Branch::Normal)),
            (2.0, NoteKind::Kat, Some(Branch::Expert)),
            (2.0, NoteKind::Don, Some(Branch::Master)),
            (4.0, NoteKind::Don, Some(Branch::Master)),
            // Merges after the longest branch.
            (6.0, NoteKind::Kat, None),
        ]);

        assert_eq!(chart.branch_points, vec![
            branch_point(0.0, 2.0, Some(BranchCondition::Precision), 80.0, 90.0),
        ]);
    }

    #[test]
    fn branch_is_judged_one_measure_before_start() {
        let chart = parse_body("1,\n#MEASURE 3/4\n1,\n#BRANCHSTART r,5,10\n#N\n1,\n#E\n1,\n#M\n1,\n#BRANCHEND");
        // Previous measure is 3/4 (1.5s) starting at 2.0.
        assert_eq!(chart.branch_points, vec![
            branch_point(2.0, 3.5, Some(BranchCondition::Roll), 5.0, 10.0),
        ]);
    }

    #[test]
    fn branch_at_chart_start_is_judged_immediately() {
        let chart = parse_body("#BRANCHSTART p,1,2\n#N\n1,\n#E\n1,\n#M\n1,\n#BRANCHEND");
        assert_close(chart.branch_points[0].judge_time_sec, 0.0);
        assert_close(chart.branch_points[0].start_time_sec, 0.0);
    }

    #[test]
    fn branch_conditions_are_parsed() {
        let chart = parse_body(
            "1,\n#BRANCHSTART s, 1000, 2000\n#N\n1,\n#BRANCHEND\n\
             #BRANCHSTART x,1,2\n#N\n1,\n#BRANCHEND\n\
             #BRANCHSTART p\n#N\n1,\n#BRANCHEND",
        );
        let conditions: Vec<_> = chart.branch_points.iter()
            .map(|bp| (bp.condition, bp.expert_threshold, bp.master_threshold))
            .collect();
        assert_eq!(conditions, vec![
            (Some(BranchCondition::Score), 1000.0, 2000.0),
            (None, 0.0, 0.0),
            (None, 0.0, 0.0),
        ]);
    }

    #[test]
    fn section_resets_are_recorded() {
        let chart = parse_body("1,\n#SECTION\n#BRANCHSTART r, 5, 10\n#N\n1,\n#E\n1,\n#M\n1,\n#BRANCHEND\n1,\n#SECTION\n1,");
        assert_eq!(chart.section_resets, vec![2.0, 6.0]);
        assert_eq!(chart.branch_points.len(), 1);
    }

    #[test]
    fn consecutive_branch_starts_merge_previous_branch() {
        // No #BRANCHEND between the branches; #N is longer than #M.
        let chart = parse_body(
            "1,\n#BRANCHSTART r,1,2\n#N\n1,1,\n#E\n1,\n#M\n1,\n\
             #BRANCHSTART r,1,2\n#N\n2,\n#E\n2,\n#M\n2,\n#BRANCHEND",
        );
        let starts: Vec<_> = chart.branch_points.iter().map(|bp| bp.start_time_sec).collect();
        assert_eq!(starts, vec![2.0, 6.0]);
        let kat_times: Vec<_> = chart.notes.iter().filter(|n| n.kind == NoteKind::Kat).map(|n| n.time_sec).collect();
        assert_eq!(kat_times, vec![6.0, 6.0, 6.0]);
    }

    #[test]
    fn level_holds_record_time_and_branch() {
        let chart = parse_body("#BRANCHSTART r,1,2\n#N\n1,\n#E\n1,\n#M\n#LEVELHOLD\n1,\n#BRANCHEND\n1,\n#LEVELHOLD");
        assert_eq!(chart.level_holds, vec![
            LevelHold { time_sec: 0.0, branch: Some(Branch::Master) },
            LevelHold { time_sec: 4.0, branch: None },
        ]);
    }

    // --- gogo time ---

    fn gogo_flags(chart: &Chart) -> Vec<bool> {
        chart.notes.iter().map(|n| n.gogo).collect()
    }

    fn gogo_ranges(chart: &Chart) -> Vec<(f64, f64, Option<Branch>)> {
        chart.gogo_ranges.iter().map(|r| (r.start_sec, r.end_sec, r.branch)).collect()
    }

    #[test]
    fn gogo_time_flags_notes_and_records_range() {
        let chart = parse_body("1,\n#GOGOSTART\n1,\n#GOGOEND\n1,");
        assert_eq!(gogo_flags(&chart), vec![false, true, false]);
        assert_eq!(gogo_ranges(&chart), vec![(2.0, 4.0, None)]);
    }

    #[test]
    fn gogo_commands_inside_measure_apply_from_their_position() {
        let chart = parse_body("10\n#GOGOSTART\n10\n#GOGOEND\n10,");
        assert_eq!(gogo_flags(&chart), vec![false, true, false]);
        let ranges = gogo_ranges(&chart);
        assert_eq!(ranges.len(), 1);
        assert_close(ranges[0].0, 2.0 / 3.0);
        assert_close(ranges[0].1, 4.0 / 3.0);
    }

    #[test]
    fn unclosed_gogo_ends_at_chart_end() {
        let chart = parse_body("#GOGOSTART\n1,1,");
        assert_eq!(gogo_ranges(&chart), vec![(0.0, 4.0, None)]);
    }

    #[test]
    fn gogo_state_is_per_branch() {
        let chart = parse_body(
            "#GOGOSTART\n1,\n#BRANCHSTART r,1,2\n#N\n1,\n#E\n#GOGOEND\n1,\n#M\n1,\n#BRANCHEND\n1,",
        );
        // Each branch starts with the state at #BRANCHSTART; after #BRANCHEND the last branch's state continues.
        let notes: Vec<_> = chart.notes.iter().map(|n| (n.time_sec, n.branch, n.gogo)).collect();
        assert_eq!(notes, vec![
            (0.0, None, true),
            (2.0, Some(Branch::Normal), true),
            (2.0, Some(Branch::Expert), false),
            (2.0, Some(Branch::Master), true),
            (4.0, None, true),
        ]);
        assert_eq!(gogo_ranges(&chart), vec![
            (0.0, 2.0, None),
            (2.0, 4.0, Some(Branch::Normal)),
            (2.0, 4.0, Some(Branch::Master)),
            (4.0, 6.0, None),
        ]);
    }

    #[test]
    fn rolls_keep_their_branch() {
        let chart = parse_body("#BRANCHSTART r,1,2\n#N\n5008,\n#E\n0,\n#M\n0,\n#BRANCHEND");
        assert_eq!(chart.rolls.len(), 1);
        assert_eq!(chart.rolls[0].branch, Some(Branch::Normal));
    }
}
