use anyhow::{Result, bail};
use glint_core::{GestureContext, MouseButton, Point, PrefixTracker};

/// Pure capture state: only button downs swallowed by Glint own a matching up.
#[derive(Default)]
pub(crate) struct Capture {
    pub active: Option<Stroke>,
    suppressed: u32,
}
pub(crate) struct Stroke {
    pub button: MouseButton,
    pub points: Vec<Point>,
    pub context: GestureContext,
    pub moved: bool,
    pub special: bool,
    pub recording: bool,
    wheel_started: bool,
    prefix: Option<PrefixTracker>,
}
impl Stroke {
    pub fn invalid(&self) -> bool {
        self.prefix.as_ref().is_some_and(PrefixTracker::is_invalid)
    }
}
impl Capture {
    pub fn begin(
        &mut self,
        button: MouseButton,
        point: Point,
        context: GestureContext,
        recording: bool,
    ) {
        self.suppressed |= button.mask();
        self.active = Some(Stroke {
            button,
            points: vec![point],
            context,
            moved: false,
            special: false,
            recording,
            wheel_started: false,
            prefix: None,
        });
    }
    pub fn track_prefix(&mut self, mut tracker: PrefixTracker) {
        if let Some(stroke) = &mut self.active
            && !stroke.recording
        {
            for &point in &stroke.points {
                tracker.push(point);
            }
            stroke.prefix = Some(tracker);
        }
    }
    pub fn owns(&self, button: MouseButton) -> bool {
        self.suppressed & button.mask() != 0
    }
    pub fn suppress(&mut self, button: MouseButton) {
        self.suppressed |= button.mask();
        if let Some(stroke) = &mut self.active {
            stroke.context.modifiers |= button.mask();
        }
    }
    pub fn release(&mut self, button: MouseButton) -> bool {
        let owned = self.owns(button);
        self.suppressed &= !button.mask();
        owned
    }
    pub fn cancel(&mut self) {
        // Keep ownership until the physical up, including during pause/reload.
        // No synthetic down remains held, so shutdown cannot leave a stuck key.
        self.active = None;
    }
    pub fn secondary_press(
        &mut self,
        button: MouseButton,
        point: Point,
        min_distance: f64,
    ) -> Option<(Vec<Point>, GestureContext)> {
        self.suppress(button);
        self.move_to(point, min_distance);
        let stroke = self.active.as_mut()?;
        if stroke.recording {
            return None;
        }
        // Consume the stroke now, so releasing the trigger cannot also run its
        // plain action (e.g. close-window after a force-close combination).
        stroke.special = true;
        if stroke.invalid() {
            return None;
        }
        let points = if stroke.moved {
            stroke.points.clone()
        } else {
            Vec::new()
        };
        Some((points, stroke.context.clone()))
    }
    pub fn wheel(
        &mut self,
        point: Point,
        min_distance: f64,
    ) -> Option<(Vec<Point>, GestureContext)> {
        self.move_to(point, min_distance);
        let stroke = self.active.as_mut()?;
        if stroke.recording {
            return None;
        }
        stroke.special = true;
        // Keep the first wheel event's stroke for repeated scrolling until release.
        stroke.wheel_started = true;
        (!stroke.invalid()).then(|| {
            let points = if stroke.moved {
                stroke.points.clone()
            } else {
                Vec::new()
            };
            (points, stroke.context.clone())
        })
    }
    pub fn move_to(&mut self, point: Point, min_distance: f64) {
        if let Some(stroke) = &mut self.active {
            if stroke.wheel_started {
                return;
            }
            let start = stroke.context.start;
            stroke.moved |= (point.x - start.x).hypot(point.y - start.y) >= min_distance;
            if stroke.points.last() != Some(&point) {
                stroke.points.push(point);
                if !stroke.special
                    && let Some(prefix) = &mut stroke.prefix
                {
                    prefix.push(point);
                }
            }
        }
    }
    pub fn finish(&mut self, point: Point, min_distance: f64) -> Option<Stroke> {
        // Include the release position even if no intermediate motion was delivered.
        self.move_to(point, min_distance);
        self.active.take()
    }
}

/// Virtual-key codes are stable Windows constants; parsing has no native effects.
pub(crate) fn parse_keys(text: &str) -> Result<Vec<u16>> {
    let mut keys = Vec::new();
    for token in text.split('+') {
        let key = match token.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => 0x11,
            "shift" => 0x10,
            "alt" => 0x12,
            "win" | "windows" | "super" => 0x5b,
            "enter" | "return" => 0x0d,
            "tab" => 0x09,
            "esc" | "escape" => 0x1b,
            "space" => 0x20,
            "backspace" => 0x08,
            "delete" | "del" => 0x2e,
            "insert" | "ins" => 0x2d,
            "home" => 0x24,
            "end" => 0x23,
            "pageup" | "pgup" => 0x21,
            "pagedown" | "pgdn" => 0x22,
            "printscreen" => 0x2c,
            "left" => 0x25,
            "up" => 0x26,
            "right" => 0x27,
            "down" => 0x28,
            "plus" => 0xbb,
            "minus" => 0xbd,
            "comma" => 0xbc,
            "period" => 0xbe,
            "volumeup" => 0xaf,
            "volumedown" => 0xae,
            "volumemute" => 0xad,
            "mediaplaypause" => 0xb3,
            "medianext" => 0xb0,
            "mediaprevious" => 0xb1,
            value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric() => {
                value.as_bytes()[0].to_ascii_uppercase() as u16
            }
            value if value.starts_with('f') => {
                let n: u16 = value[1..].parse().unwrap_or(0);
                if !(1..=24).contains(&n) {
                    bail!("Unknown key: {token}")
                }
                0x70 + n - 1
            }
            _ => bail!("Unknown key: {token}; use a chord such as Ctrl+Shift+C"),
        };
        if keys.contains(&key) {
            bail!("Repeated key in chord: {token}")
        }
        keys.push(key);
    }
    if keys.is_empty() {
        bail!("Empty key chord")
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_combinations_repeat_without_drift_and_reset_on_release() {
        use glint_core::{ActionKind, ActionSpec, Config, Matcher, Package};
        let config = Config {
            packages: vec![Package {
                id: "global".into(),
                name: "Global".into(),
                patterns: vec![".*".into()],
                actions: [
                    ("up+wheel_up", "VOLUMEUP"),
                    ("down+wheel_down", "VOLUMEDOWN"),
                    ("wheel_up", "CTRL+TAB"),
                ]
                .into_iter()
                .map(|(gesture, keys)| ActionSpec {
                    id: gesture.into(),
                    name: gesture.into(),
                    gesture: gesture.into(),
                    action: ActionKind::Keys { keys: keys.into() },
                })
                .collect(),
            }],
            ..Default::default()
        };
        let matcher = Matcher::new(&config).unwrap();
        let mut capture = Capture::default();
        for (y, event, expected) in [
            (-100., "wheel_up", "up+wheel_up"),
            (100., "wheel_down", "down+wheel_down"),
        ] {
            capture.begin(
                MouseButton::Right,
                Point::default(),
                GestureContext::default(),
                false,
            );
            capture.track_prefix(matcher.prefix_tracker("test.exe", 24.));
            let (points, _) = capture.wheel(Point { x: 0., y }, 24.).unwrap();
            assert_eq!(
                matcher
                    .recognize_binding("test.exe", &points, Some(event))
                    .unwrap()
                    .0,
                expected
            );
            let opposite = if event == "wheel_up" {
                "wheel_down"
            } else {
                "wheel_up"
            };
            assert!(
                matcher
                    .recognize_binding("test.exe", &points, Some(opposite))
                    .is_none()
            );
            let drift = Point { x: 300., y: -y };
            capture.move_to(drift, 24.);
            for _ in 0..3 {
                assert_eq!(capture.wheel(drift, 24.).unwrap().0, points);
            }
            assert!(capture.finish(drift, 24.).unwrap().special);
            assert!(capture.release(MouseButton::Right));
        }
        capture.begin(
            MouseButton::Right,
            Point::default(),
            GestureContext::default(),
            false,
        );
        let (points, _) = capture.wheel(Point::default(), 24.).unwrap();
        assert!(points.is_empty());
        assert_eq!(
            matcher
                .recognize_binding("test.exe", &points, Some("wheel_up"))
                .unwrap()
                .0,
            "wheel_up"
        );
        assert!(
            capture
                .wheel(Point { x: 0., y: -100. }, 24.)
                .unwrap()
                .0
                .is_empty()
        );
        capture.cancel();
        capture.begin(
            MouseButton::Right,
            Point::default(),
            GestureContext::default(),
            true,
        );
        assert!(capture.wheel(Point { x: 0., y: -100. }, 24.).is_none());
        let recorded = capture.finish(Point { x: 0., y: -200. }, 24.).unwrap();
        assert!(recorded.recording && !recorded.special);
        assert_eq!(recorded.points.last().unwrap().y, -200.);
        assert_eq!(parse_keys("VOLUMEUP").unwrap(), vec![0xaf]);
        assert_eq!(parse_keys("VOLUMEDOWN").unwrap(), vec![0xae]);
    }

    fn guarded_capture(recording: bool) -> Capture {
        let config = glint_core::Config {
            packages: vec![glint_core::Package {
                id: "global".into(),
                name: "Global".into(),
                patterns: vec![".*".into()],
                actions: vec![glint_core::ActionSpec {
                    id: "right".into(),
                    name: "Right".into(),
                    gesture: "right".into(),
                    action: glint_core::ActionKind::Keys {
                        keys: "CTRL+C".into(),
                    },
                }],
            }],
            ..Default::default()
        };
        let matcher = glint_core::Matcher::new(&config).unwrap();
        let mut capture = Capture::default();
        capture.begin(
            MouseButton::Right,
            Point::default(),
            GestureContext::default(),
            recording,
        );
        capture.track_prefix(matcher.prefix_tracker("test.exe", 24.));
        capture
    }

    #[test]
    fn invalid_prefix_stays_invalid_through_correction_and_release() {
        let mut capture = guarded_capture(false);
        capture.move_to(Point { x: -80., y: 0. }, 24.);
        assert!(capture.active.as_ref().unwrap().invalid());
        capture.move_to(Point { x: 2000., y: 0. }, 24.);
        let stroke = capture.finish(Point { x: 2100., y: 0. }, 24.).unwrap();
        assert!(stroke.invalid());
        assert!(capture.release(MouseButton::Right));
        capture.begin(
            MouseButton::Right,
            Point::default(),
            GestureContext::default(),
            false,
        );
        assert!(!capture.active.as_ref().unwrap().invalid());
    }

    #[test]
    fn invalid_prefix_blocks_secondary_buttons_and_wheel() {
        for wheel in [false, true] {
            let mut capture = guarded_capture(false);
            let wrong = Point { x: -80., y: 0. };
            // The event endpoint is checked even without a preceding mouse move.
            if wheel {
                assert!(capture.wheel(wrong, 24.).is_none());
            } else {
                assert!(
                    capture
                        .secondary_press(MouseButton::Left, wrong, 24.)
                        .is_none()
                );
                assert!(capture.release(MouseButton::Left));
            }
            let stroke = capture.finish(wrong, 24.).unwrap();
            assert!(stroke.special && stroke.invalid());
            assert!(capture.release(MouseButton::Right));
        }
        for wheel in [false, true] {
            let mut capture = guarded_capture(false);
            capture.move_to(Point { x: -80., y: 0. }, 24.);
            let corrected = Point { x: 2000., y: 0. };
            capture.move_to(corrected, 24.);
            if wheel {
                assert!(capture.wheel(corrected, 24.).is_none());
            } else {
                assert!(
                    capture
                        .secondary_press(MouseButton::Left, corrected, 24.)
                        .is_none()
                );
            }
            capture.cancel();
            assert!(capture.active.is_none());
            assert!(capture.release(MouseButton::Right));
            assert_eq!(capture.release(MouseButton::Left), !wheel);
        }
    }

    #[test]
    fn recording_and_short_clicks_are_not_invalidated() {
        let mut recording = guarded_capture(true);
        let stroke = recording.finish(Point { x: -100., y: 0. }, 24.).unwrap();
        assert!(stroke.recording && !stroke.invalid());
        let mut click = guarded_capture(false);
        let stroke = click.finish(Point { x: -3., y: 0. }, 24.).unwrap();
        assert!(!stroke.moved && !stroke.invalid());
    }

    #[test]
    fn secondary_press_captures_prefix_immediately_and_consumes_plain_stroke() {
        let mut c = Capture::default();
        let start = Point::default();
        c.begin(
            MouseButton::Right,
            start,
            GestureContext {
                window: 42,
                ..Default::default()
            },
            false,
        );
        let (points, context) = c.secondary_press(MouseButton::Left, start, 24.).unwrap();
        assert!(points.is_empty());
        assert_eq!(context.window, 42);
        assert!(c.active.as_ref().unwrap().special);
        assert!(c.release(MouseButton::Left));
        let finished = c.finish(start, 24.).unwrap();
        assert!(finished.special); // Trigger release must not replay a right click.
        assert!(c.release(MouseButton::Right));

        c.begin(MouseButton::Right, start, GestureContext::default(), false);
        c.move_to(Point { x: 100., y: 0. }, 24.);
        let end = Point { x: 100., y: 100. };
        let (points, _) = c.secondary_press(MouseButton::Left, end, 24.).unwrap();
        assert_eq!(points, vec![start, Point { x: 100., y: 0. }, end]);
        // Even if the trigger is released before the secondary button, both ups
        // remain owned and the plain stroke stays consumed.
        assert!(c.release(MouseButton::Right));
        assert!(c.finish(end, 24.).unwrap().special);
        assert!(c.release(MouseButton::Left));
        assert!(!c.release(MouseButton::Left));
    }
    #[test]
    fn recording_does_not_execute_secondary_buttons() {
        let mut c = Capture::default();
        c.begin(
            MouseButton::Right,
            Point::default(),
            GestureContext::default(),
            true,
        );
        assert!(
            c.secondary_press(MouseButton::Left, Point { x: 100., y: 0. }, 24.)
                .is_none()
        );
        assert!(!c.active.as_ref().unwrap().special);
        assert!(c.release(MouseButton::Left));
    }
    #[test]
    fn cancel_keeps_owned_up_and_never_owns_foreign_buttons() {
        let mut c = Capture::default();
        c.begin(
            MouseButton::Right,
            Point::default(),
            GestureContext::default(),
            false,
        );
        c.suppress(MouseButton::Left);
        c.cancel();
        assert!(c.active.is_none());
        assert!(c.release(MouseButton::Right));
        assert!(c.release(MouseButton::Left));
        assert!(!c.release(MouseButton::Middle));
        assert!(!c.release(MouseButton::Right));
    }
    #[test]
    fn distance_uses_current_point_and_motion_can_return_to_start() {
        let mut c = Capture::default();
        c.begin(
            MouseButton::Right,
            Point::default(),
            GestureContext::default(),
            false,
        );
        c.move_to(Point { x: 30.0, y: 0.0 }, 20.0);
        c.move_to(Point::default(), 20.0);
        assert!(c.active.unwrap().moved);
    }
    #[test]
    fn release_without_motion_events_preserves_endpoint_and_recording() {
        let mut c = Capture::default();
        c.begin(
            MouseButton::Left,
            Point::default(),
            GestureContext::default(),
            true,
        );
        assert!(c.release(MouseButton::Left));
        let end = Point { x: 300.0, y: 0.0 };
        let stroke = c.finish(end, 24.0).unwrap();
        assert!(stroke.moved && stroke.recording);
        assert_eq!(stroke.points, vec![Point::default(), end]);
        assert!(c.active.is_none());
        assert!(!c.release(MouseButton::Left));
    }
    #[test]
    fn key_chords_are_ordered_and_invalid_chords_fail_before_input() {
        assert_eq!(parse_keys("Ctrl+Shift+c").unwrap(), vec![0x11, 0x10, 0x43]);
        assert_eq!(parse_keys("Win+F24").unwrap(), vec![0x5b, 0x87]);
        assert_eq!(parse_keys("PrintScreen").unwrap(), vec![0x2c]);
        assert_eq!(parse_keys("ALT+PRINTSCREEN").unwrap(), vec![0x12, 0x2c]);
        for s in ["", "Ctrl++", "Ctrl+Ctrl", "F25", "Ctrl+typo"] {
            assert!(parse_keys(s).is_err(), "{s}");
        }
    }
}
