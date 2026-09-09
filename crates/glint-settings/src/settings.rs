mod view;

use super::*;
use crate::appearance::AppearanceExt;
use crate::settings_model::{
    GestureRow, action_summary, format_launch_args, gesture_label, gesture_rows, join_gesture,
    parse_launch_args, split_gesture,
};
use glint_core::{ApplicationProfile, LogLevel, LoggingConfig, Package};
use gpui_component::{
    IndexPath, WindowExt,
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    dialog::DialogButtonProps,
    select::{Select, SelectEvent, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
};
use std::time::{SystemTime, UNIX_EPOCH};

type Choice = Entity<SelectState<Vec<&'static str>>>;
const TRIGGERS: [&str; 5] = ["鼠标右键", "鼠标中键", "侧键 X1", "侧键 X2", "鼠标左键"];
const ACTION_TYPES: [&str; 3] = ["快捷键", "窗口操作", "启动程序"];
const LOG_LEVELS: [&str; 6] = ["off", "error", "warn", "info", "debug", "trace"];
const OPERATIONS: [&str; 7] = [
    "最小化",
    "最大化",
    "切换最大化 / 还原",
    "还原",
    "关闭窗口",
    "强制关闭窗口",
    "切换置顶",
];
const OPERATION_IDS: [&str; 7] = [
    "minimize",
    "maximize",
    "toggle_maximize",
    "restore",
    "close",
    "force_close",
    "toggle_topmost",
];
const EXTRAS: [&str; 8] = [
    "无",
    "点击左键",
    "点击中键",
    "点击右键",
    "点击侧键 X1",
    "点击侧键 X2",
    "滚轮向上",
    "滚轮向下",
];
const EXTRA_IDS: [&str; 8] = [
    "",
    "left_click",
    "middle_click",
    "right_click",
    "x1_click",
    "x2_click",
    "wheel_up",
    "wheel_down",
];
const FALLBACKS: [&str; 2] = ["未配置的手势：继承全局", "仅使用应用专属手势"];

#[derive(Clone, PartialEq, Eq)]
enum Page {
    General,
    Scope(String),
}

#[derive(Default, Clone)]
struct ScopeDraft {
    application: Option<ApplicationProfile>,
    upserts: Vec<ActionSpec>,
    removed: Vec<String>,
    dirty: bool,
}

struct AppDialog {
    id: Option<String>,
    disabled: bool,
    inherit_global: bool,
    generation: String,
}
struct RecordingDraft {
    id: String,
    ready: bool,
}

pub(super) struct SettingsView {
    ui_settings: UiSettings,
    palette: Palette,
    page: Page,
    dir: PathBuf,
    tx: mpsc::Sender<Job>,
    rx: mpsc::Receiver<Reply>,
    connected: bool,
    #[cfg(windows)]
    engine_process: Option<engine_process::EngineProcess>,
    config: Option<Config>,
    status: Status,
    notice: String,
    is_error: bool,
    inputs: HashMap<&'static str, Entity<InputState>>,
    input_values: HashMap<&'static str, SharedString>,
    trigger: Choice,
    log_level: Choice,
    action_type: Choice,
    window_operation: Choice,
    extra: Choice,
    fallback: Choice,
    template: Entity<SelectState<Vec<String>>>,
    template_ids: Vec<String>,
    pen_width: Entity<SliderState>,
    pen_opacity: Entity<SliderState>,
    pen_color: Entity<ColorPickerState>,
    pen_invalid_color: Entity<ColorPickerState>,
    selected: Option<GestureRow>,
    editor_gesture: String,
    scope_drafts: HashMap<String, ScopeDraft>,
    general_dirty: bool,
    editor_dirty: bool,
    loading: bool,
    prefs_loaded: bool,
    pending: bool,
    refresh_selection: bool,
    preference_tools: bool,
    app_dialog: Option<AppDialog>,
    recording: Option<RecordingDraft>,
    recording_shortcut: bool,
    capture_target: Option<String>,
    cancel_recording_queued: bool,
    _subscriptions: Vec<Subscription>,
}

fn choice(values: &[&'static str], window: &mut Window, cx: &mut Context<SettingsView>) -> Choice {
    cx.new(|cx| SelectState::new(values.to_vec(), Some(IndexPath::new(0)), window, cx))
}
fn choose(entity: &Choice, value: &'static str, window: &mut Window, cx: &mut App) {
    entity.update(cx, |state, cx| state.set_selected_value(&value, window, cx));
}
fn chosen(entity: &Choice, cx: &App) -> &'static str {
    entity
        .read(cx)
        .selected_value()
        .copied()
        .unwrap_or_default()
}
fn fresh_id(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

impl SettingsView {
    pub(super) fn new(
        dir: PathBuf,
        smoke_test: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (ui_settings, settings_error) = match UiSettings::load(&dir) {
            Ok(settings) => (settings, None),
            Err(error) => (UiSettings::default(), Some(error)),
        };
        let palette = ui_settings.appearance.apply(window, cx);
        let (tx, rx) = start_worker(dir.clone(), smoke_test);
        if !smoke_test {
            let _ = tx.send(Job::Connect);
        }
        let mut inputs = HashMap::new();
        for (key, placeholder) in [
            ("search", "搜索手势或动作"),
            ("action_name", "动作名称"),
            ("action_value", "例如 Ctrl+Shift+T"),
            ("action_args", "可选，例如 --new-window \"文件路径\""),
            ("app_name", "应用名称"),
            ("app_path", "选择应用程序或拾取窗口"),
        ] {
            inputs.insert(
                key,
                cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)),
            );
        }
        let trigger = choice(&TRIGGERS, window, cx);
        let log_level = choice(&LOG_LEVELS, window, cx);
        choose(&log_level, "info", window, cx);
        let action_type = choice(&ACTION_TYPES, window, cx);
        let window_operation = choice(&OPERATIONS, window, cx);
        let extra = choice(&EXTRAS, window, cx);
        let fallback = choice(&FALLBACKS, window, cx);
        let template = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx));
        let pen_width = cx.new(|_| {
            SliderState::new()
                .min(1.)
                .max(32.)
                .step(1.)
                .default_value(4.)
        });
        let pen_opacity = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(85.)
        });
        let pen_color = cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(0x69E0C3)));
        let pen_invalid_color =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(0x9CA3AF)));
        let mut subscriptions = Vec::new();
        for key in ["search", "action_name", "action_value", "action_args"] {
            subscriptions.push(cx.subscribe_in(
                &inputs[key],
                window,
                move |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = this.inputs[key].read(cx).value();
                        let previous = this.input_values.insert(key, value.clone());
                        // set_value emits a deferred Change event, after loading ends.
                        // Compare values so programmatic refreshes never dirty the editor.
                        if previous.as_ref() != Some(&value) {
                            if key != "search" && !this.loading {
                                this.editor_dirty = true;
                            }
                            cx.notify();
                        }
                    }
                },
            ));
        }
        for entity in [&trigger, &log_level] {
            subscriptions.push(cx.subscribe_in(
                entity,
                window,
                |this, _, event: &SelectEvent<Vec<&'static str>>, _, cx| {
                    if matches!(event, SelectEvent::Confirm(_)) && !this.loading {
                        this.general_dirty = true;
                        cx.notify();
                    }
                },
            ));
        }
        for entity in [&action_type, &window_operation, &extra] {
            subscriptions.push(cx.subscribe_in(
                entity,
                window,
                |this, _, event: &SelectEvent<Vec<&'static str>>, _, cx| {
                    if matches!(event, SelectEvent::Confirm(_)) && !this.loading {
                        this.editor_dirty = true;
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &template,
            window,
            |this, _, event: &SelectEvent<Vec<String>>, window, cx| {
                if let SelectEvent::Confirm(Some(_)) = event
                    && !this.loading
                    && let Some(index) = this.template.read(cx).selected_index(cx)
                    && let Some(id) = this.template_ids.get(index.row)
                {
                    this.editor_gesture = id.clone();
                    if glint_core::SPECIAL_GESTURES.contains(&this.editor_gesture.as_str()) {
                        this.loading = true;
                        choose(&this.extra, EXTRAS[0], window, cx);
                        this.loading = false;
                    }
                    this.editor_dirty = true;
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &fallback,
            window,
            |this, _, event: &SelectEvent<Vec<&'static str>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event
                    && !this.loading
                {
                    if !this.stash_editor(cx) {
                        this.sync_fallback(window, cx);
                        return;
                    }
                    if let Some(mut application) = this.application() {
                        application.inherit_global = *value == FALLBACKS[0];
                        let draft = this.scope_drafts.entry(application.id.clone()).or_default();
                        draft.application = Some(application);
                        draft.dirty = true;
                        this.select_first(window, cx);
                        cx.notify();
                    }
                }
            },
        ));
        for entity in [&pen_width, &pen_opacity] {
            subscriptions.push(cx.subscribe_in(
                entity,
                window,
                |this, _, _: &SliderEvent, _, cx| {
                    if !this.loading {
                        this.general_dirty = true;
                        cx.notify();
                    }
                },
            ));
        }
        for entity in [&pen_color, &pen_invalid_color] {
            subscriptions.push(cx.subscribe_in(
                entity,
                window,
                |this, _, _: &ColorPickerEvent, _, cx| {
                    if !this.loading {
                        this.general_dirty = true;
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            this.palette = this.ui_settings.appearance.apply(window, cx);
            cx.notify();
        }));
        let weak = cx.entity().downgrade();
        let handle = window.window_handle();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let _ = weak.update(cx, |this, cx| {
                if !this.recording_shortcut {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                let key = &event.keystroke;
                if key.key == "escape"
                    && !key.modifiers.control
                    && !key.modifiers.alt
                    && !key.modifiers.shift
                    && !key.modifiers.platform
                {
                    this.recording_shortcut = false;
                    this.notice = "已取消快捷键录入".into();
                    cx.notify();
                    return;
                }
                match shortcut_string(key) {
                    Ok(value) => {
                        this.set("action_value", value, window, cx);
                        this.editor_dirty = true;
                        this.recording_shortcut = false;
                        this.notice = "快捷键已录入，点击右下角应用设置生效".into();
                    }
                    Err(error) => this.error(error, cx),
                }
                cx.notify();
            });
        }));
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
                if this.pending {
                    this.error("正在保存，请稍候再关闭设置", cx);
                    return false;
                }
                if !this.has_pending_changes() {
                    if this.recording.is_some() {
                        let _ = this.tx.send(Job::Command(Command::CancelRecording));
                    }
                    return true;
                }
                if !window.has_active_dialog(cx) {
                    let tx = this.tx.clone();
                    window.open_dialog(cx, move |dialog, _, _| {
                        let tx = tx.clone();
                        dialog
                            .title("尚未应用的修改")
                            .child("关闭后将放弃尚未应用的设置。")
                            .confirm()
                            .button_props(
                                DialogButtonProps::default()
                                    .ok_text("放弃并关闭")
                                    .cancel_text("继续编辑"),
                            )
                            .on_ok(move |_, window, cx| {
                                let _ = tx.send(Job::Command(Command::CancelRecording));
                                window.defer(cx, |window, _| window.remove_window());
                                true
                            })
                    });
                }
                false
            })
            .unwrap_or(true)
        });
        cx.spawn_in(window, async move |view, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(100)).await;
                if cx
                    .update(|window, cx| view.update(cx, |this, cx| this.drain(window, cx)))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            ui_settings,
            palette,
            page: Page::General,
            dir,
            tx,
            rx,
            connected: false,
            #[cfg(windows)]
            engine_process: None,
            config: None,
            status: Status::default(),
            notice: settings_error.clone().unwrap_or_else(|| {
                if smoke_test {
                    "界面自检 · 未启动后台"
                } else {
                    "正在连接后台…"
                }
                .into()
            }),
            is_error: settings_error.is_some(),
            inputs,
            input_values: HashMap::new(),
            trigger,
            log_level,
            action_type,
            window_operation,
            extra,
            fallback,
            template,
            template_ids: Vec::new(),
            pen_width,
            pen_opacity,
            pen_color,
            pen_invalid_color,
            selected: None,
            editor_gesture: String::new(),
            scope_drafts: HashMap::new(),
            general_dirty: false,
            editor_dirty: false,
            loading: false,
            prefs_loaded: false,
            pending: false,
            refresh_selection: false,
            preference_tools: false,
            app_dialog: None,
            recording: None,
            recording_shortcut: false,
            capture_target: None,
            cancel_recording_queued: false,
            _subscriptions: subscriptions,
        }
    }

    fn value(&self, key: &'static str, cx: &App) -> String {
        self.inputs[key].read(cx).value().to_string()
    }
    fn set(
        &mut self,
        key: &'static str,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let value = value.into();
        self.input_values.insert(key, value.clone());
        self.inputs[key].update(cx, |input, cx| input.set_value(value, window, cx));
    }
    fn error(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = text.into();
        log::error!("{}", self.notice);
        self.is_error = true;
        cx.notify();
    }
    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.notice = match &command {
            Command::CaptureWindow { .. } => "请在 3 秒内将鼠标移到目标窗口…",
            Command::Record { .. } => "请按住触发键绘制新手势…",
            _ => "正在处理…",
        }
        .into();
        self.is_error = false;
        self.pending = true;
        if self.tx.send(Job::Command(command)).is_err() {
            self.pending = false;
            self.error("连接工作线程已停止，请重新打开设置", cx);
        }
        cx.notify();
    }
    fn current_scope(&self) -> Option<&str> {
        match &self.page {
            Page::General => None,
            Page::Scope(id) => Some(id),
        }
    }
    fn application(&self) -> Option<ApplicationProfile> {
        let id = self.current_scope()?;
        self.scope_drafts
            .get(id)
            .and_then(|d| d.application.clone())
            .or_else(|| {
                self.config
                    .as_ref()?
                    .applications
                    .iter()
                    .find(|a| a.id == id)
                    .cloned()
            })
    }
    fn scope_tabs(&self) -> Vec<(String, String)> {
        let mut tabs: Vec<_> = self
            .config
            .as_ref()
            .map(|c| {
                c.packages
                    .iter()
                    .filter(|p| p.id != "global")
                    .map(|p| (p.id.clone(), p.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for (id, draft) in &self.scope_drafts {
            if let Some(app) = &draft.application {
                if let Some(tab) = tabs.iter_mut().find(|tab| tab.0 == *id) {
                    tab.1 = app.name.clone();
                } else {
                    tabs.push((id.clone(), app.name.clone()));
                }
            }
        }
        tabs
    }
    fn page_title(&self) -> String {
        match &self.page {
            Page::General => "常规配置".into(),
            Page::Scope(id) if id == "global" => "全局手势".into(),
            Page::Scope(id) => self
                .scope_tabs()
                .into_iter()
                .find(|t| &t.0 == id)
                .map(|t| format!("{} 手势", t.1))
                .unwrap_or_else(|| "应用手势".into()),
        }
    }
    fn page_description(&self) -> String {
        if matches!(self.page, Page::General) {
            "调整操作方式与外观".into()
        } else if let Some(app) = self.application() {
            app.process_path
        } else if self.current_scope() == Some("global") {
            "所有应用默认使用，应用专属配置优先".into()
        } else {
            "配置文件应用规则 · 修改绑定在当前作用域生效".into()
        }
    }
    fn effective_config(&self) -> Config {
        let mut config = self.config.clone().unwrap_or_default();
        for (id, draft) in &self.scope_drafts {
            if !config.packages.iter().any(|p| p.id == *id)
                && let Some(app) = &draft.application
                && let Ok(pattern) = app.pattern()
            {
                config.packages.push(Package {
                    id: id.clone(),
                    name: app.name.clone(),
                    patterns: vec![pattern],
                    actions: Vec::new(),
                });
            }
            if let Some(package) = config.packages.iter_mut().find(|p| p.id == *id) {
                package.actions.retain(|a| !draft.removed.contains(&a.id));
                for action in &draft.upserts {
                    if let Some(current) = package.actions.iter_mut().find(|a| a.id == action.id) {
                        *current = action.clone();
                    } else {
                        package.actions.push(action.clone());
                    }
                }
            }
        }
        config
    }
    fn all_rows(&self) -> Vec<GestureRow> {
        let Some(scope) = self.current_scope() else {
            return Vec::new();
        };
        gesture_rows(
            &self.effective_config(),
            scope,
            self.application().is_none_or(|a| a.inherit_global),
        )
    }
    fn rows(&self, cx: &App) -> Vec<GestureRow> {
        let search = self.value("search", cx).to_lowercase();
        self.all_rows()
            .into_iter()
            .filter(|r| {
                search.is_empty()
                    || format!(
                        "{} {} {}",
                        r.action.name,
                        gesture_label(&r.action.gesture),
                        action_summary(&r.action.action)
                    )
                    .to_lowercase()
                    .contains(&search)
            })
            .collect()
    }
    fn is_editable(&self) -> bool {
        self.connected
            && !self.pending
            && !self.application().is_some_and(|a| a.disabled)
            && self.selected.as_ref().is_some_and(|r| !r.inherited)
    }
    fn has_pending_changes(&self) -> bool {
        self.general_dirty
            || self.editor_dirty
            || self.scope_drafts.values().any(|d| d.dirty)
            || self.app_dialog.is_some()
    }
    fn current_dirty(&self) -> bool {
        match self.current_scope() {
            Some(id) => self.editor_dirty || self.scope_drafts.get(id).is_some_and(|d| d.dirty),
            None => self.general_dirty,
        }
    }
    fn selected_type(&self, cx: &App) -> &'static str {
        match chosen(&self.action_type, cx) {
            "窗口操作" => "window",
            "启动程序" => "launch",
            _ => "keys",
        }
    }
    fn sync_fallback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        choose(
            &self.fallback,
            FALLBACKS[usize::from(self.application().is_some_and(|a| !a.inherit_global))],
            window,
            cx,
        );
        self.loading = false;
    }
    fn select_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.recording.is_some() || !self.stash_editor(cx) {
            return;
        }
        self.recording_shortcut = false;
        self.page = page;
        self.set("search", "", window, cx);
        self.sync_fallback(window, cx);
        self.select_first(window, cx);
        cx.notify();
    }
    fn select_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_dirty = false;
        if let Some(row) = self.all_rows().into_iter().next() {
            self.load_row(row, window, cx);
        } else {
            self.selected = None;
            self.editor_gesture.clear();
        }
    }
    fn select_row(&mut self, row: GestureRow, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.stash_editor(cx) {
            return;
        }
        let row = self
            .all_rows()
            .into_iter()
            .find(|r| r.action.gesture == row.action.gesture)
            .unwrap_or(row);
        self.load_row(row, window, cx);
        cx.notify();
    }
    fn load_row(&mut self, row: GestureRow, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.recording_shortcut = false;
        self.set("action_name", row.action.name.clone(), window, cx);
        let (base, extra) = split_gesture(&row.action.gesture);
        self.editor_gesture = base;
        choose(
            &self.extra,
            EXTRAS[EXTRA_IDS.iter().position(|id| *id == extra).unwrap_or(0)],
            window,
            cx,
        );
        let (kind, value, args) = match &row.action.action {
            ActionKind::Keys { keys } => (0, keys.clone(), String::new()),
            ActionKind::Window { operation } => {
                choose(
                    &self.window_operation,
                    OPERATIONS[OPERATION_IDS
                        .iter()
                        .position(|id| id == operation)
                        .unwrap_or(2)],
                    window,
                    cx,
                );
                (1, operation.clone(), String::new())
            }
            ActionKind::Launch { program, args } => (2, program.clone(), format_launch_args(args)),
        };
        self.action_type.update(cx, |state, cx| {
            state.set_items(ACTION_TYPES.to_vec(), window, cx);
        });
        choose(&self.action_type, ACTION_TYPES[kind], window, cx);
        self.set("action_value", value, window, cx);
        self.set("action_args", args, window, cx);
        self.selected = Some(row);
        self.sync_template(window, cx);
        self.editor_dirty = false;
        self.loading = false;
    }
    fn sync_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.config.clone().unwrap_or_default();
        self.template_ids = config
            .gestures
            .iter()
            .map(|g| g.id.clone())
            .chain(glint_core::SPECIAL_GESTURES.iter().map(|s| s.to_string()))
            .collect();
        let items: Vec<String> = config
            .gestures
            .iter()
            .map(|g| g.name.clone())
            .chain(
                glint_core::SPECIAL_GESTURES
                    .iter()
                    .map(|id| gesture_label(id)),
            )
            .collect();
        let selected = self
            .template_ids
            .iter()
            .position(|id| id == &self.editor_gesture)
            .map(IndexPath::new);
        self.template.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(selected, window, cx);
        });
    }
    fn editor_action(&self, cx: &App) -> Result<ActionSpec, String> {
        let row = self.selected.as_ref().ok_or("请先选择手势")?;
        let name = self.value("action_name", cx).trim().to_owned();
        if name.is_empty() {
            return Err("请填写动作名称".into());
        }
        let extra = EXTRA_IDS[EXTRAS
            .iter()
            .position(|v| *v == chosen(&self.extra, cx))
            .unwrap_or(0)];
        if self.editor_gesture.is_empty() {
            return Err("请选择或记录一个手势".into());
        }
        let gesture = join_gesture(&self.editor_gesture, extra);
        let value = self.value("action_value", cx).trim().to_owned();
        let action = match self.selected_type(cx) {
            "window" => ActionKind::Window {
                operation: OPERATION_IDS[OPERATIONS
                    .iter()
                    .position(|v| *v == chosen(&self.window_operation, cx))
                    .unwrap_or(2)]
                .into(),
            },
            "launch" => {
                if value.is_empty() {
                    return Err("请选择要启动的程序".into());
                }
                ActionKind::Launch {
                    program: value,
                    args: parse_launch_args(&self.value("action_args", cx))?,
                }
            }
            _ => {
                if value.is_empty() {
                    return Err("请录入快捷键".into());
                }
                ActionKind::Keys { keys: value }
            }
        };
        let scope = self.current_scope().ok_or("请选择配置范围")?;
        if let Some(package) = self
            .effective_config()
            .packages
            .iter()
            .find(|p| p.id == scope)
            && let Some(conflict) = package
                .actions
                .iter()
                .find(|a| a.id != row.action.id && a.gesture == gesture)
        {
            return Err(format!(
                "此手势已绑定“{}”，请更换手势或附加输入",
                conflict.name
            ));
        }
        Ok(ActionSpec {
            id: row.action.id.clone(),
            name,
            gesture,
            action,
        })
    }
    fn stash_editor(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.editor_dirty {
            return true;
        }
        let Some(row) = &self.selected else {
            self.editor_dirty = false;
            return true;
        };
        if row.inherited {
            self.editor_dirty = false;
            return true;
        }
        match self.editor_action(cx) {
            Err(error) => {
                self.error(error, cx);
                false
            }
            Ok(action) => {
                let id = self.current_scope().unwrap().to_owned();
                let draft = self.scope_drafts.entry(id).or_default();
                draft.removed.retain(|id| id != &action.id);
                draft.upserts.retain(|a| a.id != action.id);
                draft.upserts.push(action.clone());
                draft.dirty = true;
                self.selected.as_mut().unwrap().action = action;
                self.editor_dirty = false;
                true
            }
        }
    }
    fn new_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connected
            || self.pending
            || self.application().is_some_and(|a| a.disabled)
            || !self.stash_editor(cx)
        {
            return;
        }
        let Some(scope) = self.current_scope().map(str::to_owned) else {
            return;
        };
        let config = self.effective_config();
        let available = config
            .gestures
            .iter()
            .find(|g| {
                !config
                    .packages
                    .iter()
                    .filter(|p| p.id == scope)
                    .any(|p| p.actions.iter().any(|a| a.gesture == g.id))
            })
            .map(|g| g.id.clone())
            .unwrap_or_else(|| "left".into());
        self.load_row(
            GestureRow {
                source_package: scope,
                inherited: false,
                action: ActionSpec {
                    id: fresh_id("action"),
                    name: "新手势".into(),
                    gesture: available,
                    action: ActionKind::Keys {
                        keys: String::new(),
                    },
                },
            },
            window,
            cx,
        );
        self.editor_dirty = true;
        self.notice = "选择或记录手势，配置动作后点击右下角应用设置".into();
        cx.notify();
    }
    fn customize_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut row) = self.selected.clone() else {
            return;
        };
        if !row.inherited {
            return;
        }
        row.action.id = fresh_id("action");
        row.inherited = false;
        row.source_package = self.current_scope().unwrap_or("global").to_owned();
        self.load_row(row, window, cx);
        self.editor_dirty = true;
        cx.notify();
    }
    fn remove_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected.clone() else {
            return;
        };
        if row.inherited || self.pending {
            return;
        }
        let scope = self.current_scope().unwrap_or("global").to_owned();
        let weak = cx.entity().downgrade();
        let restoring = scope != "global"
            && self.config.as_ref().is_some_and(|c| {
                c.packages.iter().any(|p| {
                    p.id == "global" && p.actions.iter().any(|a| a.gesture == row.action.gesture)
                })
            });
        let label = if restoring {
            "恢复全局"
        } else {
            "删除绑定"
        };
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            let scope = scope.clone();
            let row = row.clone();
            dialog
                .title(label)
                .child(if restoring {
                    "移除当前应用的覆盖后，将恢复继承全局动作。"
                } else {
                    "删除动作绑定，保留轨迹模板。"
                })
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(label)
                        .cancel_text("取消"),
                )
                .on_ok(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let draft = this.scope_drafts.entry(scope.clone()).or_default();
                        draft.upserts.retain(|a| a.id != row.action.id);
                        if this.config.as_ref().is_some_and(|c| {
                            c.packages.iter().any(|p| {
                                p.id == scope && p.actions.iter().any(|a| a.id == row.action.id)
                            })
                        }) && !draft.removed.contains(&row.action.id)
                        {
                            draft.removed.push(row.action.id.clone());
                        }
                        draft.dirty = true;
                        this.editor_dirty = false;
                        this.select_first(window, cx);
                        this.notice = "修改已暂存，点击右下角应用设置生效".into();
                        cx.notify();
                    });
                    true
                })
        });
    }
    fn begin_shortcut(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }
        self.recording_shortcut = !self.recording_shortcut;
        self.notice = if self.recording_shortcut {
            "请按下快捷键；Esc 取消录入"
        } else {
            "已取消快捷键录入"
        }
        .into();
        cx.notify();
    }
    fn begin_recording(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }
        let id = fresh_id("gesture");
        self.recording = Some(RecordingDraft {
            id: id.clone(),
            ready: false,
        });
        self.recording_shortcut = false;
        self.send(
            Command::Record {
                id,
                name: self.value("action_name", cx),
            },
            cx,
        );
    }
    fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        self.recording = None;
        if self.pending {
            self.cancel_recording_queued = true;
            cx.notify();
        } else {
            self.send(Command::CancelRecording, cx);
        }
    }
    fn use_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(recording) = self.recording.take() {
            if !recording.ready {
                self.recording = Some(recording);
                return;
            }
            self.editor_gesture = recording.id;
            self.loading = true;
            self.sync_template(window, cx);
            self.loading = false;
            self.editor_dirty = true;
            self.notice = "新手势已记录，应用设置后生效".into();
            cx.notify();
        }
    }
    fn open_app_dialog(&mut self, is_new: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.stash_editor(cx) {
            return;
        }
        let application = if is_new { None } else { self.application() };
        if !is_new && application.is_none() {
            self.open_config(cx);
            return;
        }
        self.set(
            "app_name",
            application
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_default(),
            window,
            cx,
        );
        self.set(
            "app_path",
            application
                .as_ref()
                .map(|a| a.process_path.clone())
                .unwrap_or_default(),
            window,
            cx,
        );
        self.app_dialog = Some(AppDialog {
            id: application.as_ref().map(|a| a.id.clone()),
            disabled: application.as_ref().is_some_and(|a| a.disabled),
            inherit_global: application.as_ref().is_none_or(|a| a.inherit_global),
            generation: fresh_id("dialog"),
        });
        self.inputs["app_name"].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    fn cancel_app_dialog(&mut self, cx: &mut Context<Self>) {
        self.app_dialog = None;
        cx.notify();
    }
    fn commit_app_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Some(dialog) = &self.app_dialog else {
            return;
        };
        let name = self.value("app_name", cx).trim().to_owned();
        if name.is_empty() {
            self.error("请填写应用名称", cx);
            return;
        }
        let path = match glint_core::normalize_application_path(self.value("app_path", cx).trim()) {
            Ok(path) => path,
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        };
        let id = dialog.id.clone().unwrap_or_else(|| fresh_id("app"));
        let all_apps = self
            .config
            .as_ref()
            .map(|c| c.applications.clone())
            .unwrap_or_default()
            .into_iter()
            .chain(
                self.scope_drafts
                    .values()
                    .filter_map(|d| d.application.clone()),
            );
        if all_apps
            .into_iter()
            .any(|a| a.id != id && a.process_path.eq_ignore_ascii_case(&path))
        {
            self.error("此程序已有专属配置，请选择已有的应用标签", cx);
            return;
        }
        let app = ApplicationProfile {
            id: id.clone(),
            name,
            process_path: path,
            disabled: dialog.disabled,
            inherit_global: dialog.inherit_global,
        };
        let draft = self.scope_drafts.entry(id.clone()).or_default();
        draft.application = Some(app);
        draft.dirty = true;
        self.app_dialog = None;
        self.page = Page::Scope(id);
        self.set("search", "", window, cx);
        self.sync_fallback(window, cx);
        self.select_first(window, cx);
        self.notice = "应用配置已暂存，点击右下角应用设置生效".into();
        cx.notify();
    }
    fn pick_program(&mut self, for_application: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let dialog_generation = self.app_dialog.as_ref().map(|d| d.generation.clone());
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择可执行程序".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = pick.await
                && let Some(path) = paths.first()
            {
                let path = path.clone();
                let _ = this.update_in(cx, |this, window, cx| {
                    if !path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                    {
                        this.error("请选择 .exe 程序", cx);
                        return;
                    }
                    if for_application {
                        if this.app_dialog.as_ref().map(|d| &d.generation)
                            != dialog_generation.as_ref()
                        {
                            return;
                        }
                        this.set("app_path", path.display().to_string(), window, cx);
                        if this.value("app_name", cx).is_empty() {
                            this.set(
                                "app_name",
                                path.file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                window,
                                cx,
                            );
                        }
                    } else {
                        this.set("action_value", path.display().to_string(), window, cx);
                        this.editor_dirty = true;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn capture_application(&mut self, cx: &mut Context<Self>) {
        if self.pending || self.app_dialog.is_none() {
            return;
        }
        self.capture_target = self.app_dialog.as_ref().map(|d| d.generation.clone());
        self.send(Command::CaptureWindow { delay_ms: 3000 }, cx);
    }
    fn remove_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(application) = self.application() else {
            return;
        };
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            let id = application.id.clone();
            dialog
                .title("移除应用配置")
                .child("移除此应用的专属绑定和禁用规则，恢复使用全局手势。")
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("移除")
                        .cancel_text("取消"),
                )
                .on_ok(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.app_dialog = None;
                        if this
                            .config
                            .as_ref()
                            .is_some_and(|c| c.applications.iter().any(|a| a.id == id))
                        {
                            this.send(
                                Command::RemoveApplication {
                                    package_id: id.clone(),
                                },
                                cx,
                            );
                        } else {
                            this.scope_drafts.remove(&id);
                            this.editor_dirty = false;
                            this.page = Page::Scope("global".into());
                            this.select_first(window, cx);
                            cx.notify();
                        }
                    });
                    true
                })
        });
    }
    fn set_application_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.pending || !self.stash_editor(cx) {
            return;
        }
        if let Some(mut application) = self.application() {
            application.disabled = disabled;
            let draft = self.scope_drafts.entry(application.id.clone()).or_default();
            draft.application = Some(application);
            draft.dirty = true;
            self.notice = "应用状态已暂存，点击右下角应用设置生效".into();
            cx.notify();
        }
    }
    fn pen(&self, cx: &App) -> PenConfig {
        let rgba = self
            .pen_color
            .read(cx)
            .value()
            .unwrap_or_else(|| rgb(0x69E0C3).into())
            .to_rgb();
        let invalid_rgba = self
            .pen_invalid_color
            .read(cx)
            .value()
            .unwrap_or_else(|| rgb(0x9CA3AF).into())
            .to_rgb();
        PenConfig {
            color: format!(
                "#{:02X}{:02X}{:02X}",
                (rgba.r * 255.).round() as u8,
                (rgba.g * 255.).round() as u8,
                (rgba.b * 255.).round() as u8
            ),
            invalid_color: format!(
                "#{:02X}{:02X}{:02X}",
                (invalid_rgba.r * 255.).round() as u8,
                (invalid_rgba.g * 255.).round() as u8,
                (invalid_rgba.b * 255.).round() as u8
            ),
            width: self.pen_width.read(cx).value().start(),
            opacity: self.pen_opacity.read(cx).value().start() / 100.,
        }
    }
    fn save_current(&mut self, cx: &mut Context<Self>) {
        if !self.connected || self.pending {
            return;
        }
        if matches!(self.page, Page::General) {
            let button = match chosen(&self.trigger, cx) {
                "鼠标中键" => MouseButton::Middle,
                "侧键 X1" => MouseButton::X1,
                "侧键 X2" => MouseButton::X2,
                "鼠标左键" => MouseButton::Left,
                _ => MouseButton::Right,
            };
            self.send(
                Command::SetPreferences {
                    pen: self.pen(cx),
                    stroke_button: button,
                    logging: LoggingConfig {
                        level: match chosen(&self.log_level, cx) {
                            "off" => LogLevel::Off,
                            "error" => LogLevel::Error,
                            "warn" => LogLevel::Warn,
                            "debug" => LogLevel::Debug,
                            "trace" => LogLevel::Trace,
                            _ => LogLevel::Info,
                        },
                    },
                },
                cx,
            );
        } else if self.stash_editor(cx) {
            let id = self.current_scope().unwrap().to_owned();
            let draft = self.scope_drafts.get(&id).cloned().unwrap_or_default();
            if !draft.dirty {
                self.notice = "当前配置没有待应用的修改".into();
                cx.notify();
                return;
            }
            self.send(
                Command::ApplyScope {
                    package_id: id,
                    application: draft.application,
                    upsert_actions: draft.upserts,
                    remove_actions: draft.removed,
                },
                cx,
            );
        }
    }
    fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui_settings.appearance = appearance;
        self.palette = appearance.apply(window, cx);
        match self.ui_settings.save(&self.dir) {
            Ok(()) => {
                self.notice = "主题已保存".into();
                self.is_error = false;
            }
            Err(error) => self.error(error, cx),
        };
        cx.notify();
    }
    fn open_log(&mut self, cx: &mut Context<Self>) {
        let path = self.dir.join("glint.log");
        if path.is_file() {
            cx.open_with_system(&path);
        } else {
            self.error("日志文件尚未生成，请先启动后台", cx);
        }
    }
    fn open_config(&mut self, cx: &mut Context<Self>) {
        cx.reveal_path(&self.dir);
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.send(Command::Reload, cx);
    }
    fn gesture_preview(&self, gesture: &str) -> AnyElement {
        let (base, _) = split_gesture(gesture);
        if let Some(template) = self
            .config
            .as_ref()
            .and_then(|c| c.gestures.iter().find(|g| g.id == base))
        {
            div()
                .w_full()
                .h_full()
                .child(preview(template.clone(), self.palette.accent, 2.5, false))
                .into_any_element()
        } else {
            div()
                .flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_xs()
                .child(gesture_label(&base))
                .into_any_element()
        }
    }
    fn pen_preview(&self, cx: &App) -> AnyElement {
        let pen = self.pen(cx);
        div()
            .w_full()
            .h(px((pen.width * 3. + 4.).max(76.)))
            .opacity(pen.opacity)
            .child(preview(
                GestureTemplate {
                    id: String::new(),
                    name: String::new(),
                    points: vec![
                        glint_core::Point { x: 0., y: 0. },
                        glint_core::Point { x: 100., y: 0. },
                    ],
                },
                u32::from_str_radix(pen.color.trim_start_matches('#'), 16)
                    .unwrap_or(self.palette.accent),
                pen.width,
                true,
            ))
            .into_any_element()
    }
    fn load_preferences(&mut self, config: &Config, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        let index = match config.stroke_button {
            MouseButton::Right => 0,
            MouseButton::Middle => 1,
            MouseButton::X1 => 2,
            MouseButton::X2 => 3,
            MouseButton::Left => 4,
        };
        choose(&self.trigger, TRIGGERS[index], window, cx);
        let level = match config.logging.level {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        };
        choose(&self.log_level, level, window, cx);
        self.pen_width.update(cx, |state, cx| {
            state.set_value(config.pen.width, window, cx)
        });
        self.pen_opacity.update(cx, |state, cx| {
            state.set_value(config.pen.opacity * 100., window, cx)
        });
        if let Ok(color) = u32::from_str_radix(config.pen.color.trim_start_matches('#'), 16) {
            self.pen_color
                .update(cx, |state, cx| state.set_value(rgb(color), window, cx));
        }
        if let Ok(color) = u32::from_str_radix(config.pen.invalid_color.trim_start_matches('#'), 16)
        {
            self.pen_invalid_color
                .update(cx, |state, cx| state.set_value(rgb(color), window, cx));
        }
        self.loading = false;
        self.prefs_loaded = true;
    }
    fn drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(windows)]
        if self
            .engine_process
            .as_ref()
            .is_some_and(|process| process.has_exited())
        {
            cx.quit();
            return;
        }
        let mut changed = false;
        while let Ok(reply) = self.rx.try_recv() {
            changed = true;
            if !reply.passive {
                self.pending = false;
            }
            match reply.result {
                Err(error) => {
                    self.connected = false;
                    if !reply.passive || !self.is_error {
                        self.error(format!("后台未连接 · {error}"), cx);
                    }
                }
                Ok(response) => {
                    self.connected = true;
                    if !response.ok {
                        self.error(response.message, cx);
                        continue;
                    }
                    if !reply.passive {
                        self.is_error = false;
                        self.notice = response.message;
                        match &reply.command {
                            Some(Command::SetPreferences { .. }) => {
                                self.general_dirty = false;
                                self.prefs_loaded = false;
                            }
                            Some(Command::ApplyScope { package_id, .. }) => {
                                self.scope_drafts.remove(package_id);
                                self.editor_dirty = false;
                                self.refresh_selection = true;
                            }
                            Some(Command::RemoveApplication { package_id }) => {
                                self.scope_drafts.remove(package_id);
                                self.editor_dirty = false;
                                self.page = Page::Scope("global".into());
                                self.selected = None;
                                self.refresh_selection = true;
                            }
                            _ => {}
                        }
                    }
                    if let Some(status) = response.status {
                        #[cfg(windows)]
                        if self.engine_process.is_none()
                            && let Some(pid) = status.engine_pid
                        {
                            match engine_process::EngineProcess::open(pid) {
                                Ok(process) => self.engine_process = Some(process),
                                Err(error) if error.raw_os_error() == Some(87) => {
                                    cx.quit();
                                    return;
                                }
                                Err(error) => self.error(format!("无法监测后台退出：{error}"), cx),
                            }
                        }
                        if status.last_error != self.status.last_error
                            && let Some(error) = &status.last_error
                        {
                            self.error(error.clone(), cx);
                        }
                        self.status = status;
                    }
                    if let Some(config) = response.config {
                        glint_core::logging::set_level(config.logging.level);
                        let preferences_changed = self.config.as_ref().is_none_or(|previous| {
                            previous.logging.level != config.logging.level
                                || previous.stroke_button != config.stroke_button
                                || previous.pen.color != config.pen.color
                                || previous.pen.invalid_color != config.pen.invalid_color
                                || previous.pen.width != config.pen.width
                                || previous.pen.opacity != config.pen.opacity
                        });
                        if !self.prefs_loaded || (!self.general_dirty && preferences_changed) {
                            self.load_preferences(&config, window, cx);
                        }
                        if let Some(recording) = &mut self.recording {
                            recording.ready = config.gestures.iter().any(|g| g.id == recording.id);
                        }
                        self.config = Some(config);
                        if self.selected.is_none() || self.refresh_selection {
                            let id = self.selected.as_ref().map(|r| r.action.id.clone());
                            if let Some(row) = self
                                .all_rows()
                                .into_iter()
                                .find(|r| Some(&r.action.id) == id.as_ref())
                            {
                                self.load_row(row, window, cx);
                            } else {
                                self.select_first(window, cx);
                            }
                            self.refresh_selection = false;
                            self.sync_fallback(window, cx);
                        }
                    }
                    if let Some(context) = response.context
                        && self.app_dialog.as_ref().map(|d| &d.generation)
                            == self.capture_target.as_ref()
                        && self.capture_target.take().is_some()
                    {
                        self.set(
                            "app_name",
                            Path::new(&context.process)
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            window,
                            cx,
                        );
                        self.set("app_path", context.process, window, cx);
                    }
                }
            }
        }
        if !self.pending && self.cancel_recording_queued {
            self.cancel_recording_queued = false;
            self.send(Command::CancelRecording, cx);
        }
        if changed {
            cx.notify();
        }
    }
}

fn shortcut_string(key: &Keystroke) -> Result<String, String> {
    let raw = key.key.to_ascii_lowercase();
    let normalized = match raw.as_str() {
        "+" | "=" => "Plus",
        "-" | "_" => "Minus",
        "," | "<" => "Comma",
        "." | ">" => "Period",
        "enter" | "return" => "Enter",
        "tab" => "Tab",
        "escape" => "Esc",
        "space" => "Space",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "left" => "Left",
        "right" => "Right",
        "up" => "Up",
        "down" => "Down",
        "printscreen" => "PrintScreen",
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric() => value,
        value
            if value
                .strip_prefix('f')
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| (1..=24).contains(&n)) =>
        {
            value
        }
        _ => return Err("暂不支持此按键，请使用字母、数字、功能键或常用导航键".into()),
    };
    let mut parts = Vec::new();
    if key.modifiers.control {
        parts.push("Ctrl".into());
    }
    if key.modifiers.alt {
        parts.push("Alt".into());
    }
    if key.modifiers.shift {
        parts.push("Shift".into());
    }
    if key.modifiers.platform {
        parts.push("Win".into());
    }
    parts.push(if normalized == raw {
        normalized.to_uppercase()
    } else {
        normalized.into()
    });
    Ok(parts.join("+"))
}
