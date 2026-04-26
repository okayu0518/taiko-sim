use std::fs;
use std::path::Path;

use encoding_rs::{SHIFT_JIS, UTF_8};

use crate::models::{Branch, BranchPoint, Chart, Note, NoteKind, Roll};

const COURSE_PRIORITY: [&str; 4] = ["oni", "hard", "normal", "easy"];

#[derive(Clone)]
struct CourseBlock {
    headers: std::collections::HashMap<String, String>,
    body_lines: Vec<String>,
}

pub fn parse_tja(path: &Path, preferred_course: Option<&str>) -> Result<(Chart, Option<String>), String> {
    let text = read_tja_text(path)?;
    let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

    let (blocks, global_headers) = extract_course_blocks(&lines);
    if blocks.is_empty() {
        return Err(format!("No #START/#END block found in {}", path.display()));
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

fn read_tja_text(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    if let Ok(s) = String::from_utf8(bytes.clone()) {
        return Ok(strip_bom(s));
    }

    let (decoded_sjis, _, had_errors_sjis) = SHIFT_JIS.decode(&bytes);
    if !had_errors_sjis {
        return Ok(decoded_sjis.to_string());
    }

    let (decoded_utf8, _, _) = UTF_8.decode(&bytes);
    Ok(strip_bom(decoded_utf8.to_string()))
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
    let mut open_rolls: Vec<(f64, bool, bool, Option<Branch>)> = Vec::new();
    let mut current_time = 0.0_f64;
    let mut current_bpm = bpm;
    let mut current_scroll = 1.0_f64;
    let mut measure_num = 4_i32;
    let mut measure_den = 4_i32;
    let mut measure_buf = String::new();

    for line in body_lines {
        if line.starts_with('#') {
            let cmd = line[1..].trim().to_string();
            let cmd_upper = cmd.to_uppercase();
            if cmd_upper.starts_with("BPMCHANGE") {
                let value = cmd[9..].trim();
                if let Ok(v) = value.parse::<f64>() {
                    current_bpm = v.max(1e-6);
                }
            } else if cmd_upper.starts_with("SCROLL") {
                let value = cmd[6..].trim();
                if let Ok(v) = value.parse::<f64>() {
                    current_scroll = v;
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
            } else if cmd_upper.starts_with("DELAY") {
                let value = cmd[5..].trim();
                if let Ok(v) = value.parse::<f64>() {
                    current_time += v;
                }
            } else if cmd_upper.starts_with("BRANCHSTART") {
                has_branch = true;
                branch_start_time = Some(current_time);
                max_branch_time = current_time;
                let args = cmd[11..].trim();
                let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
                if parts.len() >= 3 {
                    let req_type = parts[0].chars().next().unwrap_or('p');
                    let expert_threshold = parts[1].parse::<f64>().unwrap_or(0.0);
                    let master_threshold = parts[2].parse::<f64>().unwrap_or(0.0);
                    branch_points.push(BranchPoint {
                        time_sec: current_time,
                        requirement_type: req_type,
                        expert_threshold,
                        master_threshold,
                    });
                }
            } else if cmd_upper == "N" {
                if let Some(_start) = branch_start_time {
                    max_branch_time = max_branch_time.max(current_time);
                    current_time = _start;
                }
                current_branch = Some(Branch::Normal);
            } else if cmd_upper == "E" {
                if let Some(_start) = branch_start_time {
                    max_branch_time = max_branch_time.max(current_time);
                    current_time = _start;
                }
                current_branch = Some(Branch::Expert);
            } else if cmd_upper == "M" {
                if let Some(_start) = branch_start_time {
                    max_branch_time = max_branch_time.max(current_time);
                    current_time = _start;
                }
                current_branch = Some(Branch::Master);
            } else if cmd_upper == "BRANCHEND" {
                if let Some(_start) = branch_start_time {
                    max_branch_time = max_branch_time.max(current_time);
                    current_time = max_branch_time;
                    branch_start_time = None;
                }
                current_branch = None;
            } else if cmd_upper == "SECTION" {
                branch_points.push(BranchPoint {
                    time_sec: current_time,
                    requirement_type: 's', // section reset
                    expert_threshold: 0.0,
                    master_threshold: 0.0,
                });
            }
            continue;
        }

        for ch in line.chars() {
            if ch == ',' {
                current_time = consume_measure(
                    &measure_buf,
                    &mut notes,
                    &mut rolls,
                    &mut open_rolls,
                    current_time,
                    current_bpm,
                    current_scroll,
                    measure_num,
                    measure_den,
                    current_branch,
                );
                measure_buf.clear();
            } else if ch.is_ascii_digit() {
                measure_buf.push(ch);
            }
        }
    }

    if !measure_buf.is_empty() {
        current_time = consume_measure(
            &measure_buf,
            &mut notes,
            &mut rolls,
            &mut open_rolls,
            current_time,
            current_bpm,
            current_scroll,
            measure_num,
            measure_den,
            current_branch,
        );
    }

    while let Some((start, big, balloon, branch)) = open_rolls.pop() {
        rolls.push(Roll {
            start_sec: start,
            end_sec: current_time,
            big,
            balloon,
            branch,
            scroll_speed: current_scroll,
        });
    }

    let duration_sec = current_time + 2.0;
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

fn consume_measure(
    measure_buf: &str,
    notes: &mut Vec<Note>,
    rolls: &mut Vec<Roll>,
    open_rolls: &mut Vec<(f64, bool, bool, Option<Branch>)>,
    current_time: f64,
    bpm: f64,
    scroll: f64,
    measure_num: i32,
    measure_den: i32,
    branch: Option<Branch>,
) -> f64 {
    let beat_sec = 60.0 / bpm.max(1e-6);
    let measure_sec = beat_sec * 4.0 * (measure_num as f64 / measure_den.max(1) as f64);

    if measure_buf.is_empty() {
        return current_time + measure_sec;
    }

    let step = measure_sec / measure_buf.len() as f64;
    for (idx, ch) in measure_buf.chars().enumerate() {
        let note_time = current_time + idx as f64 * step;
        match ch {
            '1' | '3' => notes.push(Note {
                time_sec: note_time,
                kind: NoteKind::Don,
                judged: false,
                hit: false,
                branch,
                scroll_speed: scroll,
            }),
            '2' | '4' => notes.push(Note {
                time_sec: note_time,
                kind: NoteKind::Kat,
                judged: false,
                hit: false,
                branch,
                scroll_speed: scroll,
            }),
            '5' => open_rolls.push((note_time, false, false, branch)),
            '6' => open_rolls.push((note_time, true, false, branch)),
            '7' | '9' => open_rolls.push((note_time, true, true, branch)),
            '8' => {
                if let Some((start, big, balloon, b)) = open_rolls.pop() {
                    rolls.push(Roll {
                        start_sec: start,
                        end_sec: note_time,
                        big,
                        balloon,
                        branch: b,
                        scroll_speed: scroll,
                    });
                }
            }
            _ => {}
        }
    }

    current_time + measure_sec
}

fn clean_line(raw: &str) -> String {
    if let Some((left, _)) = raw.split_once("//") {
        left.trim().to_string()
    } else {
        raw.trim().to_string()
    }
}
