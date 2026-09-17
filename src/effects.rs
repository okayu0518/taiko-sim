use std::f32::consts::TAU;

use macroquad::prelude::*;

use crate::judge::Judge;
use crate::models::NoteKind;

const LIFE_INPUT_FLASH: f64 = 0.12;
const LIFE_BURST: f64 = 0.25;
const LIFE_JUDGE_TEXT: f64 = 0.40;
const LIFE_FLYING_NOTE: f64 = 0.50;

/// Fade in/out duration of the gogo time visuals.
const GOGO_FADE_SEC: f64 = 0.15;
/// Ranges closer than this are treated as continuous (e.g. split at branch boundaries).
const GOGO_JOIN_EPS: f64 = 1e-6;

/// Where hit notes fly to (base coordinates, top-right of the screen).
const FLY_TARGET: Vec2 = Vec2::new(1180.0, 150.0);

pub fn note_color(kind: NoteKind) -> Color {
    match kind {
        NoteKind::Don => Color::from_rgba(255, 120, 100, 255),
        NoteKind::Kat => Color::from_rgba(100, 180, 255, 255),
    }
}

fn judge_color(judge: Judge) -> Color {
    match judge {
        Judge::Ryo => Color::from_rgba(255, 200, 60, 255),
        Judge::Ka => Color::from_rgba(240, 240, 240, 255),
        Judge::Fuka => Color::from_rgba(140, 160, 200, 255),
    }
}

fn with_alpha(mut c: Color, a: f32) -> Color {
    c.a *= a.clamp(0.0, 1.0);
    c
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(2)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum EffectKind {
    InputFlash(NoteKind),
    Burst(NoteKind, Judge, f32),
    JudgeText(Judge),
    FlyingNote(NoteKind, f32),
}

impl EffectKind {
    fn lifetime(self) -> f64 {
        match self {
            EffectKind::InputFlash(_) => LIFE_INPUT_FLASH,
            EffectKind::Burst(..) => LIFE_BURST,
            EffectKind::JudgeText(_) => LIFE_JUDGE_TEXT,
            EffectKind::FlyingNote(..) => LIFE_FLYING_NOTE,
        }
    }
}

struct Effect {
    kind: EffectKind,
    start: f64,
}

impl Effect {
    /// Normalized age in [0, 1], or `None` if the effect is not active at `now`.
    fn progress(&self, now: f64) -> Option<f32> {
        let age = now - self.start;
        let life = self.kind.lifetime();
        (0.0..life).contains(&age).then(|| (age / life) as f32)
    }
}

/// Short-lived visual effects for gameplay. Times are in game seconds.
#[derive(Default)]
pub struct Effects {
    items: Vec<Effect>,
}

impl Effects {
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn spawn_input(&mut self, now: f64, kind: NoteKind) {
        self.items.push(Effect { kind: EffectKind::InputFlash(kind), start: now });
    }

    /// `radius` is the hit note's radius, so big notes produce bigger effects.
    pub fn spawn_hit(&mut self, now: f64, kind: NoteKind, judge: Judge, radius: f32) {
        self.items.push(Effect { kind: EffectKind::Burst(kind, judge, radius), start: now });
        self.items.push(Effect { kind: EffectKind::FlyingNote(kind, radius), start: now });
        self.spawn_judge_text(now, judge);
    }

    pub fn spawn_judge_text(&mut self, now: f64, judge: Judge) {
        // Only the latest judgement is shown.
        self.items.retain(|e| !matches!(e.kind, EffectKind::JudgeText(_)));
        self.items.push(Effect { kind: EffectKind::JudgeText(judge), start: now });
    }

    /// Drop expired effects.
    pub fn update(&mut self, now: f64) {
        self.items.retain(|e| now - e.start < e.kind.lifetime());
    }

    /// Effects drawn below notes (world camera).
    pub fn draw_under_notes(&self, now: f64, lane_rect: Rect) {
        for e in &self.items {
            let (EffectKind::InputFlash(kind), Some(t)) = (e.kind, e.progress(now)) else { continue; };
            let color = with_alpha(note_color(kind), 0.22 * (1.0 - t));
            draw_rectangle(lane_rect.x, lane_rect.y, lane_rect.w, lane_rect.h, color);
        }
    }

    /// Effects drawn above notes (world camera).
    pub fn draw_over_notes(&self, now: f64, target: Vec2) {
        for e in &self.items {
            let Some(t) = e.progress(now) else { continue; };
            match e.kind {
                EffectKind::Burst(kind, judge, radius) => draw_burst(target, radius, kind, judge, t),
                EffectKind::FlyingNote(kind, radius) => draw_flying_note(target, radius, kind, t),
                _ => {}
            }
        }
    }

    /// Judge text in base (1280x720) coordinates, drawn through `draw_text`
    /// so the caller can apply its own font and scaling.
    pub fn draw_texts(&self, now: f64, target: Vec2, mut draw_text: impl FnMut(&str, f32, f32, u16, Color)) {
        const FONT_SIZE: u16 = 44;
        for e in &self.items {
            let (EffectKind::JudgeText(judge), Some(t)) = (e.kind, e.progress(now)) else { continue; };
            let label = judge.label();
            // CJK glyphs are roughly square, so center by character count.
            let w = label.chars().count() as f32 * FONT_SIZE as f32;
            let x = target.x - w * 0.5;
            let y = target.y - 70.0 - 18.0 * ease_out((t * 2.5).min(1.0));
            let alpha = if t < 0.6 { 1.0 } else { (1.0 - t) / 0.4 };
            draw_text(label, x + 2.0, y + 2.0, FONT_SIZE, with_alpha(BLACK, 0.7 * alpha));
            draw_text(label, x, y, FONT_SIZE, with_alpha(judge_color(judge), alpha));
        }
    }
}

/// Visual intensity (0..=1) of gogo time at `t`: fades in after a range starts
/// and out after it ends. Contiguous ranges are joined so there is no dip between them.
pub fn gogo_intensity(t: f64, ranges: &[(f64, f64)]) -> f32 {
    let mut sorted = ranges.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut joined: Vec<(f64, f64)> = Vec::new();
    for (start, end) in sorted {
        match joined.last_mut() {
            Some(last) if start <= last.1 + GOGO_JOIN_EPS => last.1 = last.1.max(end),
            _ => joined.push((start, end)),
        }
    }

    joined.iter().map(|&(start, end)| {
        if t < start {
            0.0
        } else if t < end {
            ((t - start) / GOGO_FADE_SEC).min(1.0)
        } else {
            (1.0 - (t - end) / GOGO_FADE_SEC).max(0.0)
        }
    }).fold(0.0, f64::max) as f32
}

/// Orange lane tint during gogo time.
pub fn draw_gogo_lane(lane_rect: Rect, intensity: f32) {
    if intensity <= 0.0 { return; }
    let color = Color::from_rgba(255, 110, 30, 255);
    draw_rectangle(lane_rect.x, lane_rect.y, lane_rect.w, lane_rect.h, with_alpha(color, 0.3 * intensity));
}

/// Flickering flame around the judge circle during gogo time. `time` drives the animation.
pub fn draw_gogo_flame(center: Vec2, ring_radius: f32, intensity: f32, time: f64) {
    if intensity <= 0.0 { return; }
    let time = time as f32;

    let glow_r = ring_radius + 22.0 + 4.0 * (time * 6.0).sin();
    draw_circle(center.x, center.y, glow_r, with_alpha(Color::from_rgba(255, 90, 20, 255), 0.25 * intensity));
    draw_circle(center.x, center.y, ring_radius + 12.0, with_alpha(Color::from_rgba(255, 170, 40, 255), 0.35 * intensity));

    const TONGUES: usize = 12;
    const HALF_WIDTH: f32 = 0.18;
    let base_r = ring_radius + 6.0;
    for i in 0..TONGUES {
        let flicker = (time * 9.0 + i as f32 * 1.7).sin() * 0.5 + 0.5;
        let angle = i as f32 / TONGUES as f32 * TAU + time * 0.8;
        let len = 14.0 + 12.0 * flicker;
        let left = center + Vec2::from_angle(angle - HALF_WIDTH) * base_r;
        let right = center + Vec2::from_angle(angle + HALF_WIDTH) * base_r;
        let tip = center + Vec2::from_angle(angle) * (base_r + len);
        let color = Color::new(1.0, (140.0 + 60.0 * flicker) / 255.0, 40.0 / 255.0, 0.8 * intensity);
        draw_triangle(left, right, tip, color);
    }
}

fn draw_burst(center: Vec2, note_radius: f32, kind: NoteKind, judge: Judge, t: f32) {
    let strong = judge == Judge::Ryo;
    let fade = 1.0 - t;
    let base = note_color(kind);

    let max_scale = if strong { 2.4 } else { 1.8 };
    let r = note_radius * (1.0 + (max_scale - 1.0) * ease_out(t));
    draw_circle(center.x, center.y, r, with_alpha(WHITE, (if strong { 0.55 } else { 0.35 }) * fade * fade));
    draw_circle_lines(center.x, center.y, r, 6.0 * fade + 1.0, with_alpha(base, fade));

    if strong {
        let color = with_alpha(judge_color(judge), fade);
        for i in 0..8 {
            let dir = Vec2::from_angle(i as f32 / 8.0 * TAU);
            let inner = center + dir * (r + 4.0);
            let outer = center + dir * (r + 4.0 + 18.0 * fade);
            draw_line(inner.x, inner.y, outer.x, outer.y, 3.0, color);
        }
    }
}

fn draw_flying_note(start: Vec2, note_radius: f32, kind: NoteKind, t: f32) {
    // Quadratic bezier arcing up and to the right.
    let control = vec2(start.x + 250.0, start.y - 320.0);
    let s = ease_out(t);
    let p = start * (1.0 - s).powi(2) + control * 2.0 * (1.0 - s) * s + FLY_TARGET * s * s;
    let r = note_radius * (1.0 - 0.3 * t);
    let alpha = if t < 0.8 { 1.0 } else { (1.0 - t) / 0.2 };
    draw_circle(p.x, p.y, r, with_alpha(note_color(kind), alpha));
    draw_circle_lines(p.x, p.y, r, 2.0, with_alpha(BLACK, alpha));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_expire_after_lifetime() {
        let mut fx = Effects::default();
        fx.spawn_input(0.0, NoteKind::Don);
        fx.spawn_hit(0.0, NoteKind::Kat, Judge::Ryo, 24.0);
        assert_eq!(fx.items.len(), 4);

        fx.update(LIFE_INPUT_FLASH + 0.001);
        assert_eq!(fx.items.len(), 3);

        fx.update(LIFE_FLYING_NOTE + 0.001);
        assert!(fx.items.is_empty());
    }

    #[test]
    fn only_latest_judge_text_is_kept() {
        let mut fx = Effects::default();
        fx.spawn_judge_text(0.0, Judge::Ryo);
        fx.spawn_judge_text(0.1, Judge::Fuka);
        let texts: Vec<_> = fx.items.iter().filter_map(|e| match e.kind {
            EffectKind::JudgeText(j) => Some(j),
            _ => None,
        }).collect();
        assert_eq!(texts, vec![Judge::Fuka]);
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1e-4, "expected {expected}, got {actual}");
    }

    #[test]
    fn gogo_intensity_fades_in_and_out() {
        let ranges = [(2.0, 4.0)];
        assert_near(gogo_intensity(1.9, &ranges), 0.0);
        assert_near(gogo_intensity(2.0, &ranges), 0.0);
        assert_near(gogo_intensity(2.0 + GOGO_FADE_SEC / 2.0, &ranges), 0.5);
        assert_near(gogo_intensity(3.0, &ranges), 1.0);
        assert_near(gogo_intensity(4.0 + GOGO_FADE_SEC / 2.0, &ranges), 0.5);
        assert_near(gogo_intensity(5.0, &ranges), 0.0);
        assert_near(gogo_intensity(3.0, &[]), 0.0);
    }

    #[test]
    fn contiguous_gogo_ranges_do_not_dip() {
        let ranges = [(4.0, 6.0), (2.0, 4.0)];
        assert_near(gogo_intensity(4.0 + GOGO_FADE_SEC / 2.0, &ranges), 1.0);
    }

    #[test]
    fn progress_is_normalized() {
        let e = Effect { kind: EffectKind::Burst(NoteKind::Don, Judge::Ka, 24.0), start: 1.0 };
        assert_eq!(e.progress(0.9), None);
        assert_eq!(e.progress(1.0), Some(0.0));
        assert!((e.progress(1.0 + LIFE_BURST * 0.5).unwrap() - 0.5).abs() < 1e-5);
        assert_eq!(e.progress(1.0 + LIFE_BURST), None);
    }
}
