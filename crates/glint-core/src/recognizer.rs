use crate::{GestureTemplate, Point};
use std::sync::Arc;

const SAMPLES: usize = 64;

/// Cached direction descriptors for arc-length cuts and original vertices.
pub(crate) struct PrefixDescriptors(Vec<Vec<(f64, f64)>>);

impl PrefixDescriptors {
    pub(crate) fn new(points: &[Point]) -> Self {
        if directions(points).is_none() {
            return Self(Vec::new());
        }
        let mut lengths = vec![0.0];
        for pair in points.windows(2) {
            lengths.push(
                lengths.last().unwrap() + (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y),
            );
        }
        let total = *lengths.last().unwrap();
        // Keep all ordinary recording vertices, or the 224 strongest corners in
        // unusually dense recordings. Uniform cuts still cover the entire curve.
        let mut vertices: Vec<_> = points
            .windows(3)
            .enumerate()
            .map(|(i, p)| {
                let a = (p[1].x - p[0].x, p[1].y - p[0].y);
                let b = (p[2].x - p[1].x, p[2].y - p[1].y);
                let norm = a.0.hypot(a.1) * b.0.hypot(b.1);
                (
                    i + 1,
                    if norm > 1e-18 {
                        1.0 - (a.0 * b.0 + a.1 * b.1) / norm
                    } else {
                        0.0
                    },
                )
            })
            .collect();
        vertices.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut cuts: Vec<_> = vertices
            .into_iter()
            .take(224)
            .map(|(i, _)| lengths[i])
            .collect();
        cuts.extend((1..=256).map(|i| total * i as f64 / 256.0));
        // Geometric cuts also cover the very beginning of a long recording.
        cuts.extend((1..=20).map(|i| total * 0.5_f64.powi(i)));
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let mut descriptors: Vec<Vec<(f64, f64)>> = Vec::new();
        for cut in cuts {
            if cut < 1e-8 {
                continue;
            }
            // Binary arc lookup avoids rescanning or copying the recorded polyline
            // for every prefix. Each descriptor uses the completed scorer's window.
            let sampled: Vec<_> = (0..=SAMPLES)
                .map(|i| {
                    let target = cut * i as f64 / SAMPLES as f64;
                    let segment = lengths
                        .partition_point(|&length| length <= target)
                        .max(1)
                        .min(points.len() - 1);
                    let span = lengths[segment] - lengths[segment - 1];
                    let ratio = if span > 1e-9 {
                        ((target - lengths[segment - 1]) / span).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    let a = points[segment - 1];
                    let b = points[segment];
                    Point {
                        x: a.x + (b.x - a.x) * ratio,
                        y: a.y + (b.y - a.y) * ratio,
                    }
                })
                .collect();
            let descriptor: Vec<_> = sampled
                .windows(4)
                .map(|p| {
                    let dx = p[3].x - p[0].x;
                    let dy = p[3].y - p[0].y;
                    let norm = dx.hypot(dy);
                    if norm > cut * 1e-10 {
                        (dx / norm, dy / norm)
                    } else {
                        (0.0, 0.0)
                    }
                })
                .collect();
            // Adjacent straight prefixes are identical, and need only one entry.
            if descriptors.last().is_none_or(|previous| {
                previous
                    .iter()
                    .zip(&descriptor)
                    .any(|(a, b)| (a.0 - b.0).abs() + (a.1 - b.1).abs() > 1e-6)
            }) {
                descriptors.push(descriptor);
            }
        }
        Self(descriptors)
    }
}

/// An irreversible, tolerant prefix acceptance rule, independent of final recognition.
/// A template is removed when no cached prefix fits; removed templates never return.
/// This is a geometric heuristic, not a proof that the completed scorer cannot match.
pub struct PrefixTracker {
    candidates: Vec<Arc<PrefixDescriptors>>,
    threshold: f32,
    min_distance: f64,
    points: Vec<Point>,
    last: Option<Point>,
    distance: f64,
    next_check: f64,
    active: bool,
    invalid: bool,
}

impl PrefixTracker {
    pub(crate) fn new(
        candidates: Vec<Arc<PrefixDescriptors>>,
        threshold: f32,
        min_distance: f64,
    ) -> Self {
        Self {
            candidates,
            threshold: threshold.min(0.75),
            min_distance: if min_distance.is_finite() {
                min_distance.max(0.0)
            } else {
                0.0
            },
            points: Vec::new(),
            last: None,
            distance: 0.0,
            next_check: 8.0,
            active: false,
            invalid: !threshold.is_finite() || !(0.0..=1.0).contains(&threshold),
        }
    }

    pub fn is_invalid(&self) -> bool {
        self.invalid
    }

    /// Feed every input point, including the gesture origin. Checks occur every eight
    /// physical pixels initially, even when one event crosses several checkpoints.
    /// After 4,096 pixels checkpoint spacing grows with total travel, independently
    /// of event batching. History is compacted at 1,024 points. Recognition starts
    /// only after displacement from the origin reaches the movement threshold.
    pub fn push(&mut self, point: Point) -> bool {
        if self.invalid {
            return false;
        }
        if !point.x.is_finite() || !point.y.is_finite() {
            self.invalid = true;
            return false;
        }
        let Some(last) = self.last.replace(point) else {
            self.points.push(point);
            return true;
        };
        let length = (point.x - last.x).hypot(point.y - last.y);
        if !length.is_finite() || !(self.distance + length).is_finite() {
            self.invalid = true;
            return false;
        }
        if length < 1e-9 {
            return true;
        }
        let end = self.distance + length;
        while self.next_check <= end {
            let ratio = (self.next_check - self.distance) / length;
            let checkpoint = Point {
                x: last.x + (point.x - last.x) * ratio,
                y: last.y + (point.y - last.y) * ratio,
            };
            let origin = self.points[0];
            self.active |=
                (checkpoint.x - origin.x).hypot(checkpoint.y - origin.y) >= self.min_distance;
            self.append(checkpoint);
            if self.active {
                if let Some(sample) = directions(&self.points) {
                    self.candidates.retain(|candidate| {
                        candidate
                            .0
                            .iter()
                            .any(|reference| direction_score(&sample, reference) >= self.threshold)
                    });
                } else {
                    self.candidates.clear();
                }
                if self.candidates.is_empty() {
                    self.invalid = true;
                    return false;
                }
            }
            self.next_check += 8.0_f64.max(self.next_check / 512.0);
        }
        self.distance = end;
        self.append(point);
        true
    }

    fn append(&mut self, point: Point) {
        if self.points.last() == Some(&point) {
            return;
        }
        if self.points.len() >= 1024 {
            let last = *self.points.last().unwrap();
            let mut index = 0;
            self.points.retain(|_| {
                let keep = index % 2 == 0;
                index += 1;
                keep
            });
            self.points.push(last);
        }
        self.points.push(point);
    }
}

fn direction_score(sample: &[(f64, f64)], reference: &[(f64, f64)]) -> f32 {
    (sample
        .iter()
        .zip(reference)
        .map(|(a, b)| a.0 * b.0 + a.1 * b.1)
        .sum::<f64>()
        / sample.len() as f64)
        .clamp(0.0, 1.0) as f32
}

/// Direction vectors sampled at equal arc-length intervals. Translation and
/// uniform scale are deliberately ignored; direction and rotation are retained.
fn directions(points: &[Point]) -> Option<Vec<(f64, f64)>> {
    if points.len() < 2 || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let mut cleaned = vec![points[0]];
    for &point in &points[1..] {
        let last = cleaned.last()?;
        if (point.x - last.x).hypot(point.y - last.y) > 1e-9 {
            cleaned.push(point);
        }
    }
    let mut cumulative = vec![0.0];
    for pair in cleaned.windows(2) {
        let distance = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
        cumulative.push(cumulative.last()? + distance);
    }
    let length = *cumulative.last()?;
    if !length.is_finite() || length < 1e-8 {
        return None;
    }
    let mut resampled = Vec::with_capacity(SAMPLES + 1);
    let mut segment = 1;
    for i in 0..=SAMPLES {
        let target = length * i as f64 / SAMPLES as f64;
        while segment + 1 < cumulative.len() && cumulative[segment] < target {
            segment += 1;
        }
        let ratio = ((target - cumulative[segment - 1])
            / (cumulative[segment] - cumulative[segment - 1]))
            .clamp(0.0, 1.0);
        let a = cleaned[segment - 1];
        let b = cleaned[segment];
        resampled.push(Point {
            x: a.x + (b.x - a.x) * ratio,
            y: a.y + (b.y - a.y) * ratio,
        });
    }
    // A small fixed window limits the influence of sub-pixel mouse jitter.
    let mut result = Vec::with_capacity(SAMPLES - 2);
    for pair in resampled.windows(4) {
        let dx = pair[3].x - pair[0].x;
        let dy = pair[3].y - pair[0].y;
        let norm = dx.hypot(dy);
        result.push(if norm > length * 1e-10 {
            (dx / norm, dy / norm)
        } else {
            (0.0, 0.0)
        });
    }
    Some(result)
}

pub fn recognize(
    points: &[Point],
    templates: &[GestureTemplate],
    threshold: f32,
) -> Option<(String, f32)> {
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return None;
    }
    let sample = directions(points)?;
    let mut best: Option<(String, f32)> = None;
    for template in templates {
        let Some(reference) = directions(&template.points) else {
            continue;
        };
        let score = direction_score(&sample, &reference);
        if score >= threshold && best.as_ref().is_none_or(|(_, previous)| score > *previous) {
            best = Some((template.id.clone(), score));
        }
    }
    best
}

pub fn default_templates() -> Vec<GestureTemplate> {
    type Shape = (&'static str, &'static str, &'static [(f64, f64)]);
    let shapes: [Shape; 26] = [
        ("left", "向左", &[(100., 0.), (0., 0.)]),
        ("right", "向右", &[(0., 0.), (100., 0.)]),
        ("up", "向上", &[(0., 100.), (0., 0.)]),
        ("down", "向下", &[(0., 0.), (0., 100.)]),
        ("up_left", "左上", &[(100., 100.), (0., 0.)]),
        ("up_right", "右上", &[(0., 100.), (100., 0.)]),
        ("down_left", "左下", &[(100., 0.), (0., 100.)]),
        ("down_right", "右下", &[(0., 0.), (100., 100.)]),
        ("left_up", "左再上", &[(100., 100.), (0., 100.), (0., 0.)]),
        ("left_down", "左再下", &[(100., 0.), (0., 0.), (0., 100.)]),
        (
            "right_up",
            "右再上",
            &[(0., 100.), (100., 100.), (100., 0.)],
        ),
        (
            "right_down",
            "右再下",
            &[(0., 0.), (100., 0.), (100., 100.)],
        ),
        ("up_down", "上再下", &[(0., 100.), (0., 0.), (0., 100.)]),
        ("down_up", "下再上", &[(0., 0.), (0., 100.), (0., 0.)]),
        ("left_right", "左再右", &[(100., 0.), (0., 0.), (100., 0.)]),
        ("right_left", "右再左", &[(0., 0.), (100., 0.), (0., 0.)]),
        (
            "up_then_left",
            "上再左",
            &[(100., 100.), (100., 0.), (0., 0.)],
        ),
        (
            "up_then_right",
            "上再右",
            &[(0., 100.), (0., 0.), (100., 0.)],
        ),
        (
            "down_then_left",
            "下再左",
            &[(100., 0.), (100., 100.), (0., 100.)],
        ),
        (
            "down_then_right",
            "下再右",
            &[(0., 0.), (0., 100.), (100., 100.)],
        ),
        (
            "down_then_right_then_down",
            "下右下",
            &[(0., 0.), (0., 100.), (100., 100.), (100., 200.)],
        ),
        (
            "right_then_up_then_left",
            "右上左",
            &[(0., 100.), (100., 100.), (100., 0.), (0., 0.)],
        ),
        (
            "up_then_right_then_up",
            "上右上",
            &[(0., 200.), (0., 100.), (100., 100.), (100., 0.)],
        ),
        (
            "up_then_right_then_down_then_left",
            "上右下左",
            &[(0., 100.), (0., 0.), (100., 0.), (100., 100.), (0., 100.)],
        ),
        ("v", "V 字", &[(0., 0.), (50., 100.), (100., 0.)]),
        (
            "inverted_v",
            "倒 V 字",
            &[(0., 100.), (50., 0.), (100., 100.)],
        ),
    ];
    let mut templates: Vec<_> = shapes
        .into_iter()
        .map(|(id, name, points)| GestureTemplate {
            id: id.into(),
            name: name.into(),
            points: points.iter().map(|&(x, y)| Point { x, y }).collect(),
        })
        .collect();
    for (id, name, sign) in [
        ("clockwise", "顺时针圆", 1.0),
        ("counterclockwise", "逆时针圆", -1.0),
    ] {
        templates.push(GestureTemplate {
            id: id.into(),
            name: name.into(),
            points: (0..=64)
                .map(|i| {
                    let angle = -std::f64::consts::FRAC_PI_2
                        + sign * std::f64::consts::TAU * i as f64 / 64.0;
                    Point {
                        x: 50.0 + 50.0 * angle.cos(),
                        y: 50.0 + 50.0 * angle.sin(),
                    }
                })
                .collect(),
        });
    }
    templates
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tracker(points: &[Point]) -> PrefixTracker {
        PrefixTracker::new(vec![Arc::new(PrefixDescriptors::new(points))], 0.9, 16.0)
    }

    #[test]
    fn prefix_accepts_sampled_translated_scaled_defaults_and_recorded_polylines() {
        let mut templates = default_templates();
        templates.push(GestureTemplate {
            id: "recorded".into(),
            name: "recorded".into(),
            points: vec![
                Point { x: 0., y: 0. },
                Point { x: 43., y: 11. },
                Point { x: 71., y: -54. },
                Point { x: 160., y: -22. },
                Point { x: 130., y: 100. },
            ],
        });
        for template in templates {
            for scale in [0.7, 1.0, 3.7] {
                let mut guard = tracker(&template.points);
                let transform = |p: Point| Point {
                    x: p.x * scale + 123.4,
                    y: p.y * scale - 987.6,
                };
                assert!(guard.push(transform(template.points[0])));
                for pair in template.points.windows(2) {
                    for i in 1..=37 {
                        let ratio = i as f64 / 37.0;
                        let p = Point {
                            x: pair[0].x + (pair[1].x - pair[0].x) * ratio,
                            y: pair[0].y + (pair[1].y - pair[0].y) * ratio,
                        };
                        assert!(
                            guard.push(transform(p)),
                            "{} scale {scale} segment {pair:?} sample {i}",
                            template.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn prefix_wrong_direction_is_permanent_and_new_tracker_resets() {
        let left = [Point::default(), Point { x: -100., y: 0. }];
        let mut guard = tracker(&left);
        assert!(guard.push(Point::default()));
        assert!(!guard.push(Point { x: 80., y: 0. }));
        assert!(!guard.push(Point { x: -1000., y: 0. }));
        assert!(guard.is_invalid());
        let mut fresh = tracker(&left);
        assert!(fresh.push(Point::default()));
        assert!(fresh.push(Point { x: -1000., y: 0. }));
    }

    #[test]
    fn prefix_tolerates_jitter_and_bounds_history() {
        let mut guard = tracker(&[Point::default(), Point { x: 100., y: 0. }]);
        for i in 0..6000 {
            assert!(guard.push(Point {
                x: i as f64 * 0.5,
                y: if i % 2 == 0 { 0.2 } else { -0.2 }
            }));
        }
        assert!(guard.points.len() <= 1024);
    }

    #[test]
    fn prefix_checkpoints_cannot_be_skipped_by_large_events() {
        let left = [Point::default(), Point { x: -100., y: 0. }];
        let mut dense = tracker(&left);
        let mut sparse = tracker(&left);
        dense.push(Point::default());
        sparse.push(Point::default());
        for x in 1..=40 {
            dense.push(Point { x: x as f64, y: 0. });
        }
        sparse.push(Point { x: 40., y: 0. });
        assert!(dense.is_invalid() && sparse.is_invalid());
    }

    #[test]
    fn prefix_small_wiggles_do_not_activate_and_long_strokes_remain_valid() {
        let left = [Point::default(), Point { x: -100., y: 0. }];
        let mut guard = tracker(&left);
        guard.push(Point::default());
        for i in 0..1000 {
            assert!(guard.push(Point {
                x: if i % 2 == 0 { 5. } else { -5. },
                y: 0.
            }));
        }
        let mut long = tracker(&left);
        long.push(Point::default());
        assert!(long.push(Point {
            x: -100_000.,
            y: 0.
        }));
    }

    #[test]
    fn dense_recording_cache_is_bounded_and_accepts_its_sampled_curve() {
        let points: Vec<_> = (0..8192)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 8191.0;
                Point {
                    x: 100. * angle.sin(),
                    y: 100. * (1. - angle.cos()),
                }
            })
            .collect();
        let cache = Arc::new(PrefixDescriptors::new(&points));
        assert!(cache.0.len() <= 500);
        let mut guard = PrefixTracker::new(vec![cache], 0.9, 16.);
        for point in points.iter().step_by(7) {
            assert!(guard.push(*point));
        }
        assert!(guard.push(*points.last().unwrap()));
    }

    #[test]
    #[ignore = "manual release timing: cargo test -p glint-core --release prefix_benchmark -- --ignored --nocapture"]
    fn prefix_benchmark() {
        use std::time::Instant;
        let mut templates = default_templates();
        templates.push(GestureTemplate {
            id: "dense".into(),
            name: "dense".into(),
            points: (0..8192)
                .map(|i| {
                    let angle = i as f64 * std::f64::consts::TAU / 8191.0;
                    Point {
                        x: 100. * angle.sin(),
                        y: 100. * (1. - angle.cos()),
                    }
                })
                .collect(),
        });
        let start = Instant::now();
        let cache: Vec<_> = templates
            .iter()
            .map(|t| Arc::new(PrefixDescriptors::new(&t.points)))
            .collect();
        eprintln!(
            "29 template precompute (including 8192-point circle): {:?}; dense descriptors {}",
            start.elapsed(),
            cache.last().unwrap().0.len()
        );
        assert!(cache.last().unwrap().0.len() <= 500);
        let start = Instant::now();
        let mut worst = std::time::Duration::ZERO;
        for _ in 0..100 {
            let mut guard = PrefixTracker::new(cache.clone(), 0.85, 16.);
            for i in 0..=628 {
                let angle = i as f64 / 100.;
                let begin = Instant::now();
                std::hint::black_box(guard.push(Point {
                    x: 100. * angle.sin(),
                    y: 100. * (1. - angle.cos()),
                }));
                worst = worst.max(begin.elapsed());
            }
        }
        eprintln!(
            "62,900 pushes: {:?}; worst push {:?}",
            start.elapsed(),
            worst
        );
    }
    #[test]
    fn unique_defaults_recognize_themselves() {
        let all = default_templates();
        assert_eq!(all.len(), 28);
        for t in &all {
            let result = recognize(&t.points, &all, 0.8).unwrap();
            assert_eq!(result.0, t.id);
            assert!(result.1 > 0.99, "{} {}", t.id, result.1);
        }
    }
    #[test]
    fn translation_uniform_scale_and_sampling_invariant() {
        let all = default_templates();
        for t in &all {
            let changed = t
                .points
                .iter()
                .map(|p| Point {
                    x: p.x * 3.7 - 401.,
                    y: p.y * 3.7 + 909.,
                })
                .collect::<Vec<_>>();
            assert_eq!(recognize(&changed, &all, 0.99).unwrap().0, t.id);
        }
        let line = (0..=200)
            .map(|x| Point {
                x: x as f64,
                y: 40.,
            })
            .collect::<Vec<_>>();
        assert_eq!(recognize(&line, &all, 0.99).unwrap().0, "right");
    }
    #[test]
    fn reverse_is_a_different_direction() {
        let all = default_templates();
        let reverse = all[0].points.iter().rev().copied().collect::<Vec<_>>();
        assert_eq!(recognize(&reverse, &all, 0.8).unwrap().0, "right");
        assert!(recognize(&reverse, &all[..1], 0.8).is_none());
    }
    #[test]
    fn tolerates_mouse_jitter() {
        let points = (0..=100)
            .map(|i| Point {
                x: i as f64 * 2.,
                y: if i % 2 == 0 { 0.4 } else { -0.4 },
            })
            .collect::<Vec<_>>();
        let (id, score) = recognize(&points, &default_templates(), 0.9).unwrap();
        assert_eq!(id, "right");
        assert!((0.0..=1.0).contains(&score));
    }
    #[test]
    fn rejects_empty_stationary_nonfinite_and_invalid_thresholds() {
        let t = default_templates();
        for p in [
            vec![],
            vec![Point { x: 1., y: 2. }],
            vec![Point::default(); 20],
            vec![Point::default(), Point { x: f64::NAN, y: 1. }],
        ] {
            assert!(recognize(&p, &t, 0.8).is_none());
        }
        assert!(recognize(&t[0].points, &t, f32::NAN).is_none());
        assert!(recognize(&t[0].points, &t, 1.1).is_none());
        assert!(recognize(&t[0].points, &[], 0.1).is_none());
    }
}
