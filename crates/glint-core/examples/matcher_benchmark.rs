//! Manual, deterministic timing; excludes configuration construction from recognition.
use glint_core::{ActionKind, ActionSpec, Config, Matcher, Package, Point};
use std::{hint::black_box, time::Instant};

fn main() {
    for dense in [false, true] {
        let mut config = Config::default();
        if dense {
            for template in &mut config.gestures {
                let original = template.points.clone();
                template.points = original
                    .windows(2)
                    .flat_map(|pair| {
                        (0..32).map(move |i| {
                            let t = i as f64 / 32.;
                            Point {
                                x: pair[0].x + (pair[1].x - pair[0].x) * t,
                                y: pair[0].y + (pair[1].y - pair[0].y) * t,
                            }
                        })
                    })
                    .chain(original.last().copied())
                    .collect();
            }
        }
        config.packages = vec![Package {
            id: "global".into(),
            name: "Global".into(),
            patterns: vec![".*".into()],
            actions: config
                .gestures
                .iter()
                .map(|t| ActionSpec {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    gesture: t.id.clone(),
                    action: ActionKind::Keys {
                        keys: "CTRL+C".into(),
                    },
                })
                .collect(),
        }];
        let build = Instant::now();
        let matcher = Matcher::new(&config).unwrap();
        let build_elapsed = build.elapsed();
        let build_start = Instant::now();
        for _ in 0..20 {
            black_box(Matcher::new(black_box(&config)).unwrap());
        }
        let repeated_build = build_start.elapsed();
        let start = Instant::now();
        let count = 200_000;
        for i in 0..count {
            let template = &config.gestures[i % config.gestures.len()];
            black_box(matcher.recognize_binding(
                black_box("test.exe"),
                black_box(&template.points),
                None,
            ));
        }
        let recognition = start.elapsed();
        let prefix_start = Instant::now();
        let mut accepted = 0;
        for _ in 0..100 {
            let mut tracker = matcher.prefix_tracker("test.exe", 16.0);
            for i in 0..=628 {
                let angle = i as f64 / 100.;
                accepted += usize::from(black_box(tracker.push(black_box(Point {
                    x: 100. * angle.sin(),
                    y: 100. * (1. - angle.cos()),
                }))));
            }
        }
        let prefix = prefix_start.elapsed();
        println!(
            "{}",
            serde_json::json!({
                "dense": dense, "templates": config.gestures.len(),
                "initial_build_ms": build_elapsed.as_secs_f64() * 1000.,
                "build_count": 20, "build_ms": repeated_build.as_secs_f64() * 1000.,
                "recognition_count": count, "recognition_ms": recognition.as_secs_f64() * 1000.,
                "prefix_pushes": 62900, "prefix_ms": prefix.as_secs_f64() * 1000.,
                "accepted": accepted,
            })
        );
    }
}
