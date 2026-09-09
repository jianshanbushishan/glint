use glint_core::Point;

/// Separate straight retraces in display coordinates without editing the template.
/// Curves, corners, and closed shapes are deliberately left untouched.
pub fn separate_retraces(points: &[Point]) -> Vec<Vec<Point>> {
    if points.is_empty() {
        return Vec::new();
    }
    let unchanged = || vec![points.to_vec()];
    let start = points[0];
    if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return unchanged();
    }
    let farthest = points
        .iter()
        .max_by(|a, b| {
            (a.x - start.x)
                .hypot(a.y - start.y)
                .total_cmp(&(b.x - start.x).hypot(b.y - start.y))
        })
        .unwrap();
    let length = (farthest.x - start.x).hypot(farthest.y - start.y);
    if length <= 1. {
        return unchanged();
    }
    let axis = (
        (farthest.x - start.x) / length,
        (farthest.y - start.y) / length,
    );
    let normal = (-axis.1, axis.0);
    if points
        .iter()
        .any(|p| ((p.x - start.x) * normal.0 + (p.y - start.y) * normal.1).abs() > 1.)
    {
        return unchanged();
    }

    // Track extrema rather than individual sample deltas: subpixel steps can
    // form a real return stroke, while subpixel jitter is not a new direction.
    let mut turns = vec![0.];
    let mut extreme = 0.;
    let mut direction = 0.;
    for p in points {
        let position = (p.x - start.x) * axis.0 + (p.y - start.y) * axis.1;
        if direction == 0. {
            if position.abs() > 1. {
                direction = position.signum();
                extreme = position;
            }
        } else if (position - extreme) * direction >= 0. {
            extreme = position;
        } else if (position - extreme).abs() > 1. {
            turns.push(extreme);
            direction = -direction;
            extreme = position;
        }
    }
    turns.push(extreme);
    let count = turns.len() - 1;
    if count < 2 {
        return unchanged();
    }
    // Unusual recordings may contain many returns. Keep the displacement
    // bounded so that the preview cannot grow beyond its reserved padding.
    let spacing = 8_f64.min(24. / (count - 1) as f64);
    turns
        .windows(2)
        .enumerate()
        .map(|(index, ends)| {
            let offset = (index as f64 - (count - 1) as f64 / 2.) * spacing;
            ends.iter()
                .map(|&position| Point {
                    x: start.x + axis.0 * position + normal.0 * offset,
                    y: start.y + axis.1 * position + normal.1 * offset,
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(coords: &[(f64, f64)]) -> Vec<Point> {
        coords.iter().map(|&(x, y)| Point { x, y }).collect()
    }

    #[test]
    fn returns_have_separate_lanes_and_real_directions() {
        for coords in [
            vec![(0., 0.), (100., 0.), (0., 0.)],
            vec![(100., 0.), (0., 0.), (100., 0.)],
            vec![(0., 100.), (0., 0.), (0., 100.)],
            vec![(0., 0.), (100., 0.), (40., 0.)],
            vec![(0., 0.), (100., 0.), (0., 0.), (100., 0.)],
        ] {
            let input = points(&coords);
            let output = separate_retraces(&input);
            assert_eq!(output.len(), input.len() - 1);
            for (segment, original) in output.iter().zip(input.windows(2)) {
                let dx = segment[1].x - segment[0].x;
                let dy = segment[1].y - segment[0].y;
                assert!((dx - (original[1].x - original[0].x)).abs() < 1e-8);
                assert!((dy - (original[1].y - original[0].y)).abs() < 1e-8);
            }
            for pair in output.windows(2) {
                let a = pair[0][1];
                let b = pair[1][0];
                assert!(((a.x - b.x).hypot(a.y - b.y) - 8.).abs() < 1e-8);
            }
            assert_eq!(input.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), coords);
        }
    }

    #[test]
    fn ordinary_corners_and_closed_shapes_are_unchanged() {
        for coords in [
            vec![(0., 0.), (100., 0.), (100., 100.)],
            vec![(0., 0.), (50., 100.), (100., 0.)],
            vec![(0., 0.), (100., 0.), (100., 100.), (0., 100.), (0., 0.)],
            vec![(0., 0.), (100., 0.)],
        ] {
            let output = separate_retraces(&points(&coords));
            assert_eq!(output.len(), 1);
            assert_eq!(
                output[0].iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(),
                coords
            );
        }
    }

    #[test]
    fn dense_samples_duplicates_and_small_jitter_do_not_add_lanes() {
        let mut input = Vec::new();
        for i in (0..=1000).chain((0..1000).rev()) {
            let p = Point {
                x: i as f64 / 10.,
                y: if i % 2 == 0 { 0. } else { 0.2 },
            };
            input.extend([p, p]);
        }
        input.extend(points(&[(0.3, 0.), (0., 0.)]));
        let output = separate_retraces(&input);
        assert_eq!(output.len(), 2);
        assert!((output[0][1].x - 100.).abs() < 1e-8);
        assert!((output[1][1].x).abs() < 1e-8);
    }

    #[test]
    fn degenerate_inputs_and_many_returns_are_bounded() {
        assert!(separate_retraces(&[]).is_empty());
        for input in [points(&[(2., 3.)]), points(&[(2., 3.), (2., 3.)])] {
            let output = separate_retraces(&input);
            assert_eq!(output[0].len(), input.len());
        }
        let input: Vec<_> = (0..100)
            .map(|i| Point {
                x: (i % 2) as f64 * 100.,
                y: 0.,
            })
            .collect();
        let output = separate_retraces(&input);
        assert_eq!(output.len(), 99);
        assert!(output.iter().flatten().all(|p| p.y.abs() <= 12.));
    }
}
