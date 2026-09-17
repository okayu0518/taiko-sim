use std::fs;
use std::path::Path;

use encoding_rs::{SHIFT_JIS, UTF_8};

use crate::models::{Branch, BranchPoint, Chart, Note, NoteKind, Roll, ScrollSpeed};

const COURSE_PRIORITY: [&str; 4] = ["oni", "hard", "normal", "easy"];

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
                .map(|c| c.trim().eq_ignore_ascii_case(&p))
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
                .map(|c| c.trim().eq_ignore_ascii_case(name))
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
    let mut has_branch = false;
    let mut current_branch: Option<Branch> = None;
    let mut branch_start_time: Option<f64> = None;
    let mut max_branch_time = 0.0_f64;
    let mut open_rolls: Vec<OpenRoll> = Vec::new();
    let mut timing = TimingState { time: 0.0, bpm, scroll: 1.0 };
    let mut measure_num = 4_i32;
    let mut measure_den = 4_i32;
    let mut measure_items: Vec<MeasureItem> = Vec::new();

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
                has_branch = true;
                branch_start_time = Some(timing.time);
                max_branch_time = timing.time;
                let args = cmd[11..].trim();
                let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
                if parts.len() >= 3 {
                    let req_type = parts[0].chars().next().unwrap_or('p');
                    let expert_threshold = parts[1].parse::<f64>().unwrap_or(0.0);
                    let master_threshold = parts[2].parse::<f64>().unwrap_or(0.0);
                    branch_points.push(BranchPoint {
                        time_sec: timing.time,
                        requirement_type: req_type,
                        expert_threshold,
                        master_threshold,
                    });
                }
            } else if let Some(branch) = match cmd_upper.as_str() {
                "N" => Some(Branch::Normal),
                "E" => Some(Branch::Expert),
                "M" => Some(Branch::Master),
                _ => None,
            } {
                if let Some(start) = branch_start_time {
                    max_branch_time = max_branch_time.max(timing.time);
                    timing.time = start;
                }
                current_branch = Some(branch);
            } else if cmd_upper == "BRANCHEND" {
                if branch_start_time.is_some() {
                    max_branch_time = max_branch_time.max(timing.time);
                    timing.time = max_branch_time;
                    branch_start_time = None;
                }
                current_branch = None;
            } else if cmd_upper == "SECTION" {
                branch_points.push(BranchPoint {
                    time_sec: timing.time,
                    requirement_type: 's', // section reset
                    expert_threshold: 0.0,
                    master_threshold: 0.0,
                });
            }
            continue;
        }

        for ch in line.chars() {
            if ch == ',' {
                consume_measure(
                    &measure_items,
                    &mut timing,
                    &mut notes,
                    &mut rolls,
                    &mut open_rolls,
                    measure_num,
                    measure_den,
                    current_branch,
                );
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
            current_branch,
        );
    }

    while let Some(open) = open_rolls.pop() {
        rolls.push(open.close(timing.time, timing.speed()));
    }

    let duration_sec = timing.time + 2.0;
    Ok(Chart {
        title,
        course,
        level,
        offset_sec,
        notes,
        rolls,
        branch_points,
        has_branch,
        duration_sec,
    })
}

/// Current position and scroll state while reading a chart.
struct TimingState {
    time: f64,
    bpm: f64,
    scroll: f64,
}

impl TimingState {
    fn speed(&self) -> ScrollSpeed {
        ScrollSpeed { bpm: self.bpm, scroll: self.scroll }
    }

    fn apply(&mut self, item: &MeasureItem) {
        match *item {
            MeasureItem::BpmChange(v) => self.bpm = v.max(1e-6),
            MeasureItem::Scroll(v) => self.scroll = v,
            MeasureItem::Delay(v) => self.time += v,
            MeasureItem::Note(_) => {}
        }
    }
}

/// An element of a measure: a note character or a timing command placed between notes.
enum MeasureItem {
    Note(char),
    BpmChange(f64),
    Scroll(f64),
    Delay(f64),
}

/// Parse commands that may appear inside a measure (`#BPMCHANGE`, `#SCROLL`, `#DELAY`).
fn parse_timing_command(cmd: &str, cmd_upper: &str) -> Option<MeasureItem> {
    let value = |prefix: &str| cmd[prefix.len()..].trim().parse::<f64>().ok();
    if cmd_upper.starts_with("BPMCHANGE") {
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
#[allow(clippy::too_many_arguments)]
fn consume_measure(
    items: &[MeasureItem],
    timing: &mut TimingState,
    notes: &mut Vec<Note>,
    rolls: &mut Vec<Roll>,
    open_rolls: &mut Vec<OpenRoll>,
    measure_num: i32,
    measure_den: i32,
    branch: Option<Branch>,
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
            }),
            '2' | '4' => notes.push(Note {
                time_sec: note_time,
                kind: NoteKind::Kat,
                judged: false,
                hit: false,
                big: ch == '4',
                branch,
                speed,
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

        assert_eq!(chart.branch_points.len(), 1);
        let bp = &chart.branch_points[0];
        assert_close(bp.time_sec, 2.0);
        assert_eq!(bp.requirement_type, 'p');
        assert_close(bp.expert_threshold, 80.0);
        assert_close(bp.master_threshold, 90.0);
    }

    #[test]
    fn roll_branch_and_section_points_are_recorded() {
        let chart = parse_body("1,\n#SECTION\n#BRANCHSTART r, 5, 10\n#N\n1,\n#E\n1,\n#M\n1,\n#BRANCHEND");
        let points: Vec<_> = chart.branch_points.iter()
            .map(|bp| (bp.time_sec, bp.requirement_type, bp.expert_threshold, bp.master_threshold))
            .collect();
        assert_eq!(points, vec![(2.0, 's', 0.0, 0.0), (2.0, 'r', 5.0, 10.0)]);
    }

    #[test]
    fn rolls_keep_their_branch() {
        let chart = parse_body("#BRANCHSTART r,1,2\n#N\n5008,\n#E\n0,\n#M\n0,\n#BRANCHEND");
        assert_eq!(chart.rolls.len(), 1);
        assert_eq!(chart.rolls[0].branch, Some(Branch::Normal));
    }
}
