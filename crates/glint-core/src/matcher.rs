use crate::recognizer::{PrefixDescriptors, PrefixTracker, direction_score, directions};
use crate::{ActionSpec, Config, Point};
use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder, RegexSet, RegexSetBuilder};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub struct Matcher {
    excluded: RegexSet,
    packages: Vec<(String, RegexSet, Vec<ActionSpec>)>,
    applications: Vec<(Regex, String, bool, bool)>,
    templates: Vec<(String, Vec<(f64, f64)>)>,
    threshold: f32,
    prefix_templates: HashMap<String, Arc<PrefixDescriptors>>,
}

impl Matcher {
    pub fn new(config: &Config) -> Result<Self> {
        Self::build(config, true)
    }

    /// Validate rule compilation without building discarded geometry caches.
    pub(crate) fn validate_patterns(config: &Config) -> Result<()> {
        Self::build(config, false).map(|_| ())
    }

    fn build(config: &Config, cache_geometry: bool) -> Result<Self> {
        for (i, expression) in config.excluded.iter().enumerate() {
            RegexBuilder::new(expression)
                .case_insensitive(true)
                .build()
                .with_context(|| format!("excluded[{i}]: invalid regex {expression:?}"))?;
        }
        let excluded = RegexSetBuilder::new(&config.excluded)
            .case_insensitive(true)
            .build()?;
        let mut packages = Vec::new();
        for package in &config.packages {
            for expression in &package.patterns {
                RegexBuilder::new(expression)
                    .case_insensitive(true)
                    .build()
                    .with_context(|| {
                        format!(
                            "package {:?}: invalid match regex {expression:?}",
                            package.id
                        )
                    })?;
            }
            let patterns = RegexSetBuilder::new(&package.patterns)
                .case_insensitive(true)
                .build()?;
            packages.push((package.id.clone(), patterns, package.actions.clone()));
        }
        let applications = config
            .applications
            .iter()
            .map(|app| {
                Ok((
                    RegexBuilder::new(&app.pattern()?)
                        .case_insensitive(true)
                        .build()?,
                    app.id.clone(),
                    app.disabled,
                    app.inherit_global,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            excluded,
            packages,
            applications,
            templates: config
                .gestures
                .iter()
                .filter(|_| cache_geometry)
                .filter_map(|template| {
                    directions(&template.points).map(|descriptor| (template.id.clone(), descriptor))
                })
                .collect(),
            threshold: config.threshold,
            prefix_templates: config
                .gestures
                .iter()
                .filter(|_| cache_geometry)
                .map(|template| {
                    (
                        template.id.clone(),
                        Arc::new(PrefixDescriptors::new(&template.points)),
                    )
                })
                .collect(),
        })
    }
    pub fn is_excluded(&self, process: &str) -> bool {
        self.excluded.is_match(process)
            || self
                .applications
                .iter()
                .any(|(pattern, _, disabled, _)| *disabled && pattern.is_match(process))
    }
    /// Applicable packages in action priority order, shared by recognition and dispatch.
    fn matching_packages(&self, process: &str) -> Vec<&[ActionSpec]> {
        if self.is_excluded(process) {
            return Vec::new();
        }
        if let Some((_, id, _, inherit_global)) = self
            .applications
            .iter()
            .rev()
            .find(|(pattern, _, _, _)| pattern.is_match(process))
        {
            let mut packages = Vec::new();
            for wanted in [Some(id.as_str()), inherit_global.then_some("global")]
                .into_iter()
                .flatten()
            {
                if let Some((_, _, actions)) = self
                    .packages
                    .iter()
                    .find(|(package_id, _, _)| package_id == wanted)
                {
                    packages.push(actions.as_slice());
                }
            }
            return packages;
        }
        self.packages
            .iter()
            .rev()
            .filter(|(id, patterns, _)| {
                !self
                    .applications
                    .iter()
                    .any(|(_, app_id, _, _)| app_id == id)
                    && patterns.is_match(process)
            })
            .map(|(_, _, actions)| actions.as_slice())
            .collect()
    }

    pub fn match_action(&self, process: &str, gesture: &str) -> Option<ActionSpec> {
        self.matching_packages(process)
            .into_iter()
            .flat_map(|actions| actions.iter().rev())
            .find(|action| action.gesture == gesture)
            .cloned()
    }

    /// Snapshot effective plain and compound base templates for this process.
    /// Normalized geometry is computed only when constructing the matcher and shared.
    pub fn prefix_tracker(&self, process: &str, min_distance: f64) -> PrefixTracker {
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for actions in self.matching_packages(process) {
            for action in actions {
                let base = match action.gesture.split_once('+') {
                    Some((base, event)) if crate::SPECIAL_GESTURES.contains(&event) => base,
                    Some(_) => continue,
                    None if crate::SPECIAL_GESTURES.contains(&action.gesture.as_str()) => continue,
                    None => action.gesture.as_str(),
                };
                if seen.insert(base)
                    && let Some(template) = self.prefix_templates.get(base)
                {
                    candidates.push(Arc::clone(template));
                }
            }
        }
        PrefixTracker::new(candidates, self.threshold, min_distance)
    }

    /// Recognize only shapes with an effective binding in this process and event family.
    /// An optional suffix selects compound gestures; empty points select a direct event.
    pub fn recognize_binding(
        &self,
        process: &str,
        points: &[Point],
        suffix: Option<&str>,
    ) -> Option<(String, f32)> {
        if points.is_empty() {
            return suffix.and_then(|gesture| {
                self.match_action(process, gesture)
                    .map(|_| (gesture.to_owned(), 1.0))
            });
        }
        if !self.threshold.is_finite() || !(0.0..=1.0).contains(&self.threshold) {
            return None;
        }
        let sample = directions(points)?;
        let mut seen = std::collections::HashSet::new();
        let mut best: Option<(&str, f32)> = None;
        for actions in self.matching_packages(process) {
            let bound: std::collections::HashSet<&str> = actions
                .iter()
                .filter_map(|action| match (action.gesture.split_once('+'), suffix) {
                    (None, None) => Some(action.gesture.as_str()),
                    (Some((base, button)), Some(wanted)) if button == wanted => Some(base),
                    _ => None,
                })
                .collect();
            // The recognizer keeps the first equal score. Package order gives application
            // bindings priority, while template order within each package remains stable.
            for (id, reference) in &self.templates {
                if bound.contains(id.as_str()) && seen.insert(id.as_str()) {
                    let score = direction_score(&sample, reference);
                    if score >= self.threshold && best.is_none_or(|(_, previous)| score > previous)
                    {
                        best = Some((id.as_str(), score));
                    }
                }
            }
        }
        best.map(|(id, score)| {
            (
                suffix.map_or_else(|| id.to_owned(), |button| format!("{id}+{button}")),
                score,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionKind, GestureTemplate, Package};
    #[test]
    fn cached_recognition_matches_reference_scores_and_rejects_invalid_inputs() {
        let mut config = Config::default();
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
        for threshold in [0.0, 0.8, 1.0, f32::NAN, 1.1] {
            config.threshold = threshold;
            let matcher = Matcher::new(&config).unwrap();
            let mut samples = vec![
                vec![],
                vec![Point::default(); 4],
                vec![Point { x: f64::NAN, y: 0. }],
            ];
            for template in &config.gestures {
                samples.push(template.points.clone());
                samples.push(
                    template
                        .points
                        .iter()
                        .enumerate()
                        .map(|(i, p)| Point {
                            x: p.x * 2.7 + 123.0,
                            y: p.y * 2.7 - 50.0 + (i % 2) as f64 * 0.3,
                        })
                        .collect(),
                );
                samples.push(template.points.iter().rev().copied().collect());
            }
            for points in samples {
                assert_eq!(
                    matcher.recognize_binding("test.exe", &points, None),
                    crate::recognize(&points, &config.gestures, threshold)
                );
            }
        }
    }

    #[test]
    fn prefix_scope_includes_compound_only_and_honors_inheritance_and_exclusion() {
        let mut config = Config {
            applications: vec![app("editor")],
            packages: vec![
                bound_package("global", "left"),
                bound_package("editor", "right+middle_click"),
            ],
            ..Config::default()
        };
        let accepts = |config: &Config, process: &str, x| {
            let matcher = Matcher::new(config).unwrap();
            let mut guard = matcher.prefix_tracker(process, 16.0);
            guard.push(Point::default());
            guard.push(Point { x, y: 0. })
        };
        assert!(accepts(&config, r"C:\Apps\editor.exe", 100.));
        assert!(accepts(&config, r"C:\Apps\editor.exe", -100.));
        assert!(!accepts(&config, "other.exe", 100.));
        config.applications[0].inherit_global = false;
        assert!(!accepts(&config, r"C:\Apps\editor.exe", -100.));
        config.applications[0].disabled = true;
        assert!(!accepts(&config, r"C:\Apps\editor.exe", 100.));
        config.excluded = vec!["other".into()];
        assert!(!accepts(&config, "other.exe", -100.));
        config.excluded.clear();
        config.packages = vec![bound_package("global", "middle_click")];
        // Even a malformed template named for a direct button cannot keep a stroke alive.
        config.gestures.push(GestureTemplate {
            id: "middle_click".into(),
            name: "direct".into(),
            points: vec![Point::default(), Point { x: 100., y: 0. }],
        });
        assert!(!accepts(&config, "other.exe", 100.));
    }
    fn package(id: &str, pattern: &str) -> Package {
        Package {
            id: id.into(),
            name: id.into(),
            patterns: vec![pattern.into()],
            actions: vec![ActionSpec {
                id: id.into(),
                name: id.into(),
                gesture: "left".into(),
                action: ActionKind::Keys {
                    keys: "CTRL+C".into(),
                },
            }],
        }
    }
    #[test]
    fn later_packages_take_priority_and_case_is_insensitive() {
        let c = Config {
            packages: vec![
                package("global", ".*"),
                package("browser", r"(^|[\\/])chrome\.exe$"),
            ],
            ..Config::default()
        };
        let matcher = Matcher::new(&c).unwrap();
        assert_eq!(
            matcher
                .match_action(r"C:\Chrome\CHROME.EXE", "left")
                .unwrap()
                .id,
            "browser"
        );
        assert_eq!(
            matcher.match_action("notepad.exe", "left").unwrap().id,
            "global"
        );
        assert!(matcher.match_action("chrome.exe", "unbound").is_none());
    }
    #[test]
    fn excludes_win_and_invalid_regex_has_context() {
        let mut c = Config {
            packages: vec![package("global", ".*")],
            excluded: vec!["game.exe$".into()],
            ..Config::default()
        };
        let m = Matcher::new(&c).unwrap();
        assert!(m.is_excluded("GAME.EXE"));
        assert!(m.match_action("GAME.EXE", "left").is_none());
        c.packages[0].patterns = vec!["[".into()];
        let err = Matcher::new(&c).err().unwrap().to_string();
        assert!(err.contains("global"));
    }
    #[test]
    fn application_scope_is_exact_and_inheritance_is_explicit() {
        let app = crate::ApplicationProfile {
            id: "app".into(),
            name: "App".into(),
            process_path: r"C:\Apps (x64)\editor[1].exe".into(),
            disabled: false,
            inherit_global: true,
        };
        let mut own = package("app", ".*");
        own.actions[0].gesture = "right".into();
        let mut config = Config {
            applications: vec![app.clone()],
            packages: vec![own, package("global", ".*"), package("legacy", "editor")],
            ..Config::default()
        };
        let matcher = Matcher::new(&config).unwrap();
        assert_eq!(
            matcher
                .match_action(&app.process_path.to_uppercase(), "right")
                .unwrap()
                .id,
            "app"
        );
        assert_eq!(
            matcher.match_action(&app.process_path, "left").unwrap().id,
            "global"
        );
        assert!(
            matcher
                .match_action(r"D:\Apps (x64)\editor[1].exe", "right")
                .is_none()
        );
        assert!(
            matcher
                .match_action(&format!("{}.backup", app.process_path), "right")
                .is_none()
        );
        config.applications[0].inherit_global = false;
        let matcher = Matcher::new(&config).unwrap();
        assert!(matcher.match_action(&app.process_path, "left").is_none());
        assert_eq!(
            matcher.match_action(&app.process_path, "right").unwrap().id,
            "app"
        );
        config.applications[0].disabled = true;
        let matcher = Matcher::new(&config).unwrap();
        assert!(matcher.is_excluded(&app.process_path.to_uppercase()));
        assert!(matcher.match_action(&app.process_path, "right").is_none());
        assert!(!matcher.is_excluded(r"D:\Apps (x64)\editor[1].exe"));
    }

    #[test]
    fn application_paths_normalize_before_matching_and_exclusion() {
        let app = crate::ApplicationProfile {
            id: "editor".into(),
            name: "Editor".into(),
            process_path: "c:/Apps/./Old/../editor.exe".into(),
            disabled: false,
            inherit_global: false,
        };
        let mut config = Config {
            applications: vec![app],
            packages: vec![package("editor", ".*")],
            ..Config::default()
        };
        assert_eq!(
            Matcher::new(&config)
                .unwrap()
                .match_action(r"C:\APPS\EDITOR.EXE", "left")
                .unwrap()
                .id,
            "editor"
        );
        config.applications[0].disabled = true;
        assert!(
            Matcher::new(&config)
                .unwrap()
                .is_excluded(r"C:\APPS\EDITOR.EXE")
        );
    }

    fn app(id: &str) -> crate::ApplicationProfile {
        crate::ApplicationProfile {
            id: id.into(),
            name: id.into(),
            process_path: format!(r"C:\Apps\{id}.exe"),
            disabled: false,
            inherit_global: true,
        }
    }

    fn bound_package(id: &str, gesture: &str) -> Package {
        let mut result = package(id, ".*");
        result.actions[0].gesture = gesture.into();
        result
    }

    #[test]
    fn duplicate_recording_wins_only_in_its_application_scope() {
        let mut config = Config {
            applications: vec![app("editor")],
            packages: vec![
                bound_package("editor", "recorded_left"),
                package("global", ".*"),
            ],
            ..Config::default()
        };
        let left = config
            .gestures
            .iter()
            .find(|t| t.id == "left")
            .unwrap()
            .clone();
        config.gestures.push(GestureTemplate {
            id: "recorded_left".into(),
            ..left.clone()
        });
        let matcher = Matcher::new(&config).unwrap();
        assert_eq!(
            matcher
                .recognize_binding(r"C:\Apps\editor.exe", &left.points, None)
                .unwrap()
                .0,
            "recorded_left"
        );
        assert_eq!(
            matcher
                .recognize_binding(r"C:\Apps\other.exe", &left.points, None)
                .unwrap()
                .0,
            "left"
        );
        config.applications[0].inherit_global = false;
        assert_eq!(
            Matcher::new(&config)
                .unwrap()
                .recognize_binding(r"C:\Apps\editor.exe", &left.points, None)
                .unwrap()
                .0,
            "recorded_left"
        );
        config.applications[0].disabled = true;
        assert!(
            Matcher::new(&config)
                .unwrap()
                .recognize_binding(r"C:\Apps\editor.exe", &left.points, None)
                .is_none()
        );
    }

    #[test]
    fn unbound_and_other_application_templates_cannot_steal_recognition() {
        let mut config = Config {
            applications: vec![app("other")],
            packages: vec![
                package("global", ".*"),
                bound_package("other", "other_recording"),
            ],
            ..Config::default()
        };
        let points = vec![Point { x: 100., y: 0. }, Point { x: 0., y: 20. }];
        // These fit exactly, better than the bound horizontal left template.
        for id in ["cancelled_recording", "other_recording"] {
            config.gestures.push(GestureTemplate {
                id: id.into(),
                name: id.into(),
                points: points.clone(),
            });
        }
        let matcher = Matcher::new(&config).unwrap();
        assert_eq!(
            matcher
                .recognize_binding(r"C:\Apps\editor.exe", &points, None)
                .unwrap()
                .0,
            "left"
        );
        assert_eq!(
            matcher
                .recognize_binding(r"C:\Apps\other.exe", &points, None)
                .unwrap()
                .0,
            "other_recording"
        );
        config.packages.clear();
        assert!(
            Matcher::new(&config)
                .unwrap()
                .recognize_binding("unbound.exe", &points, None)
                .is_none()
        );
    }

    #[test]
    fn compound_candidates_are_isolated_by_button_and_never_fall_back_to_plain() {
        let mut config = Config {
            packages: vec![package("global", ".*")],
            ..Config::default()
        };
        let left = config
            .gestures
            .iter()
            .find(|t| t.id == "left")
            .unwrap()
            .clone();
        for (id, binding) in [
            ("compound_left", "compound_left+left_click"),
            ("compound_middle", "compound_middle+middle_click"),
        ] {
            config.gestures.push(GestureTemplate {
                id: id.into(),
                ..left.clone()
            });
            config.packages[0]
                .actions
                .push(bound_package(id, binding).actions.remove(0));
        }
        config.packages[0]
            .actions
            .push(bound_package("direct", "x1_click").actions.remove(0));
        let matcher = Matcher::new(&config).unwrap();
        assert_eq!(
            matcher
                .recognize_binding("app.exe", &left.points, None)
                .unwrap()
                .0,
            "left"
        );
        assert_eq!(
            matcher
                .recognize_binding("app.exe", &left.points, Some("left_click"))
                .unwrap()
                .0,
            "compound_left+left_click"
        );
        assert_eq!(
            matcher
                .recognize_binding("app.exe", &left.points, Some("middle_click"))
                .unwrap()
                .0,
            "compound_middle+middle_click"
        );
        assert!(
            matcher
                .recognize_binding("app.exe", &left.points, Some("x1_click"))
                .is_none()
        );
        assert_eq!(
            matcher
                .recognize_binding("app.exe", &[], Some("x1_click"))
                .unwrap()
                .0,
            "x1_click"
        );
        assert!(
            matcher
                .recognize_binding("app.exe", &[], Some("left_click"))
                .is_none()
        );
    }

    #[test]
    fn legacy_package_priority_also_breaks_recognition_ties() {
        let mut config = Config {
            packages: vec![
                package("global", ".*"),
                bound_package("legacy", "legacy_left"),
            ],
            ..Config::default()
        };
        config.packages[1].patterns = vec!["editor.exe$".into()];
        let left = config
            .gestures
            .iter()
            .find(|t| t.id == "left")
            .unwrap()
            .clone();
        config.gestures.push(GestureTemplate {
            id: "legacy_left".into(),
            ..left.clone()
        });
        let matcher = Matcher::new(&config).unwrap();
        assert_eq!(
            matcher
                .recognize_binding("editor.exe", &left.points, None)
                .unwrap()
                .0,
            "legacy_left"
        );
        assert_eq!(
            matcher
                .recognize_binding("other.exe", &left.points, None)
                .unwrap()
                .0,
            "left"
        );
    }
}
