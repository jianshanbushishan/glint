use super::*;
use gpui_component::{Icon, IconName};

impl SettingsView {
    fn page_icon(&self, page: &Page, size: f32, fallback: IconName) -> AnyElement {
        let image = if let Page::Scope(id) = page {
            self.scope_drafts
                .get(id)
                .and_then(|draft| draft.application.as_ref())
                .or_else(|| {
                    self.config
                        .as_ref()?
                        .applications
                        .iter()
                        .find(|app| &app.id == id)
                })
                .and_then(|app| crate::application_icon::load(&app.process_path))
        } else {
            None
        };
        if let Some(image) = image {
            gpui::img(image)
                .size(px(size))
                .flex_shrink_0()
                .into_any_element()
        } else {
            Icon::new(fallback)
                .size(px(size))
                .flex_shrink_0()
                .into_any_element()
        }
    }

    pub(super) fn modal_open(&self) -> bool {
        self.app_dialog.is_some() || self.recording.is_some() || self.editor_blocked.is_some()
    }

    fn controls_blocked(&self) -> bool {
        !self.connected || self.pending || self.modal_open()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut global_navigation = v_flex()
            .gap_1()
            .py_1()
            .border_t_1()
            .border_b_1()
            .border_color(rgb(self.palette.border));
        for (id, label, page) in [
            (
                "nav-general".to_owned(),
                "常规设置".to_owned(),
                Page::General,
            ),
            (
                "nav-global".to_owned(),
                "全局手势".to_owned(),
                Page::Scope("global".into()),
            ),
        ] {
            global_navigation = global_navigation.child(self.navigation_item(id, label, page, cx));
        }
        let mut navigation = v_flex().gap_1().child(global_navigation);
        navigation = navigation.child(
            div()
                .px_3()
                .pt_4()
                .pb_1()
                .text_xs()
                .text_color(rgb(self.palette.muted))
                .child("应用专属"),
        );
        for (id, name) in self.scope_tabs() {
            navigation = navigation.child(self.navigation_item(
                format!("nav-{id}"),
                name,
                Page::Scope(id),
                cx,
            ));
        }
        navigation = navigation.child(
            Button::new("add-application")
                .ghost()
                .small()
                .label("＋ 添加应用")
                .w_full()
                .disabled(self.controls_blocked())
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_app_dialog(true, window, cx)),
                ),
        );
        v_flex()
            .w(px(144.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(self.palette.bg))
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .px_2()
            .py_3()
            .child(
                h_flex()
                    .px_3()
                    .pb_4()
                    .gap_2()
                    .child(
                        div()
                            .size(px(28.))
                            .rounded_md()
                            .bg(rgb(self.palette.accent))
                            .text_color(rgb(self.palette.panel))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::Redo2).size(px(16.))),
                    )
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Glint"),
                    ),
            )
            .child(
                div()
                    .id("settings-navigation")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(navigation),
            )
            .child(
                div()
                    .px_3()
                    .pt_4()
                    .text_xs()
                    .text_center()
                    .text_color(rgb(self.palette.muted))
                    .child(concat!("Glint · v", env!("CARGO_PKG_VERSION")))
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(11.))
                            .child(env!("GLINT_BUILD_TIME")),
                    ),
            )
            .into_any_element()
    }

    fn navigation_item(
        &self,
        id: String,
        label: String,
        page: Page,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = match (&self.page, &page) {
            (Page::General, Page::General) => true,
            (Page::Scope(a), Page::Scope(b)) => a == b,
            _ => false,
        };
        let icon = match &page {
            Page::General => IconName::Settings2,
            Page::Scope(scope) if scope == "global" => IconName::Globe,
            Page::Scope(_) => IconName::LayoutDashboard,
        };
        let dirty = match &page {
            Page::General => self.general_dirty,
            Page::Scope(scope) => {
                self.scope_drafts
                    .get(scope)
                    .is_some_and(|draft| draft.dirty)
                    || (selected && self.editor_dirty)
            }
        };
        let icon = self.page_icon(&page, 16., icon);
        div()
            .id(SharedString::from(id))
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .text_color(rgb(if selected {
                self.palette.accent
            } else {
                self.palette.muted
            }))
            .when(selected, |el| {
                el.bg(rgb(self.palette.selected))
                    .font_weight(FontWeight::MEDIUM)
            })
            .hover(|el| el.bg(rgb(self.palette.hover)))
            .on_click(
                cx.listener(move |this, _, window, cx| this.select_page(page.clone(), window, cx)),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(icon)
                            .child(div().min_w_0().truncate().child(label)),
                    )
                    .when(dirty, |el| el.child(div().text_xs().child("· 未应用"))),
            )
            .into_any_element()
    }

    fn render_heading(&self, cx: &mut Context<Self>) -> AnyElement {
        let application = self.application();
        let icon = match &self.page {
            Page::General => IconName::Settings2,
            Page::Scope(id) if id == "global" => IconName::Globe,
            _ => IconName::LayoutDashboard,
        };
        let mut heading = v_flex().gap_3().px_5().pt_4().pb_4().flex_shrink_0().child(
            h_flex()
                .gap_3()
                .justify_between()
                .child(
                    h_flex()
                        .gap_3()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .size(px(40.))
                                .flex_shrink_0()
                                .rounded_lg()
                                .bg(rgb(self.palette.bg))
                                .border_1()
                                .border_color(rgb(self.palette.border))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgb(self.palette.accent))
                                .child(self.page_icon(&self.page, 21., icon)),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_xl()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(self.page_title()),
                                )
                                .child(self.muted(if application.is_some() {
                                    "应用专属手势".to_owned()
                                } else {
                                    self.page_description()
                                })),
                        ),
                )
                .when(
                    self.current_scope().is_some_and(|id| id != "global"),
                    |el| {
                        el.child(
                            Button::new("delete-application-gestures")
                                .ghost()
                                .size(px(36.))
                                .child(Icon::default().path("icons/glint-trash.svg").size(px(21.)))
                                .tooltip("删除应用手势")
                                .text_color(rgb(self.palette.error))
                                .disabled(self.controls_blocked())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.remove_application(window, cx)
                                })),
                        )
                    },
                ),
        );
        if let Some(application) = application {
            heading = heading.child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(rgb(self.palette.muted))
                            .child(application.process_path.clone()),
                    )
                    .child(
                        Button::new("change-application-program")
                            .outline()
                            .small()
                            .icon(IconName::FolderOpen)
                            .label("更改应用程序")
                            .disabled(self.controls_blocked())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_application_program(window, cx)
                            })),
                    ),
            );
            let mut settings = v_flex()
                .rounded_lg()
                .border_1()
                .border_color(rgb(self.palette.border))
                .child(
                    h_flex()
                        .p_3()
                        .gap_3()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap_1()
                                .flex_1()
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("在此应用中启用手势"),
                                )
                                .child(self.muted(if application.disabled {
                                    "已停用 · 不识别或拦截手势，已有配置保留"
                                } else {
                                    "已启用 · 在此应用中识别手势"
                                })),
                        )
                        .child(
                            Switch::new("enable-application")
                                .checked(!application.disabled)
                                .disabled(self.controls_blocked())
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.set_application_disabled(!*checked, cx)
                                })),
                        ),
                );
            if !application.disabled {
                let mut options = h_flex().gap_2().flex_wrap();
                for (id, title, description, inherit) in [
                    (
                        "fallback-global",
                        "使用全局配置",
                        "未单独配置时，执行对应的全局动作",
                        true,
                    ),
                    (
                        "fallback-ignore",
                        "不执行",
                        "只使用此应用单独配置的手势",
                        false,
                    ),
                ] {
                    let active = application.inherit_global == inherit;
                    options =
                        options.child(
                            Button::new(id)
                                .outline()
                                .h_auto()
                                .flex_1()
                                .min_w(px(180.))
                                .p_3()
                                .disabled(self.controls_blocked())
                                .when(active, |el| {
                                    el.bg(rgb(self.palette.selected))
                                        .border_color(rgb(self.palette.accent))
                                })
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_start()
                                        .child(
                                            Icon::new(if active {
                                                IconName::CircleCheck
                                            } else {
                                                IconName::Minus
                                            })
                                            .size(px(16.))
                                            .text_color(rgb(if active {
                                                self.palette.accent
                                            } else {
                                                self.palette.muted
                                            })),
                                        )
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .items_start()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .child(title),
                                                )
                                                .child(self.muted(description)),
                                        ),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_application_fallback(inherit, window, cx)
                                })),
                        );
                }
                settings = settings.child(
                    v_flex()
                        .p_3()
                        .gap_2()
                        .bg(rgb(self.palette.bg))
                        .border_t_1()
                        .border_color(rgb(self.palette.border))
                        .child(options),
                );
            }
            heading = heading.child(settings);
        }
        heading.into_any_element()
    }

    fn muted(&self, text: impl Into<SharedString>) -> AnyElement {
        div()
            .text_xs()
            .text_color(rgb(self.palette.muted))
            .child(text.into())
            .into_any_element()
    }

    fn settings_card(&self) -> Div {
        v_flex()
            .w_full()
            .min_w_0()
            .p_3()
            .gap_3()
            .rounded_lg()
            .bg(rgb(self.palette.panel))
            .border_1()
            .border_color(rgb(self.palette.border))
    }

    fn settings_section(&self, title: &'static str, content: impl IntoElement) -> Div {
        v_flex()
            .w_full()
            .gap_2()
            .child(self.muted(title))
            .child(content)
    }

    fn render_general(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.controls_blocked();
        let narrow = window.viewport_size().width < px(740.);
        let mut themes = h_flex()
            .gap_1()
            .p_1()
            .flex_shrink_0()
            .bg(rgb(self.palette.bg))
            .rounded_md();
        for (id, label, appearance) in [
            ("theme-system", "跟随系统", Appearance::System),
            ("theme-light", "浅色", Appearance::Light),
            ("theme-dark", "深色", Appearance::Dark),
        ] {
            themes = themes.child(
                Button::new(id)
                    .disabled(self.modal_open())
                    .small()
                    .ghost()
                    .label(label)
                    .when(self.ui_settings.appearance == appearance, |button| {
                        button
                            .bg(rgb(self.palette.panel))
                            .text_color(rgb(self.palette.accent))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_appearance(appearance, window, cx)
                    })),
            );
        }
        let mut colors = h_flex().gap_4().flex_wrap();
        for (id, label, state) in [
            ("disabled-pen-color", "正常颜色", &self.pen_color),
            (
                "disabled-pen-invalid-color",
                "无效颜色",
                &self.pen_invalid_color,
            ),
        ] {
            colors = colors.child(
                h_flex()
                    .gap_2()
                    .child(if disabled {
                        div()
                            .id(id)
                            .size(px(24.))
                            .rounded_md()
                            .bg(state
                                .read(cx)
                                .value()
                                .unwrap_or_else(|| rgb(self.palette.muted).into()))
                            .opacity(0.5)
                            .into_any_element()
                    } else {
                        ColorPicker::new(state).small().into_any_element()
                    })
                    .child(self.muted(label)),
            );
        }
        let mut parameters = v_flex().flex_1().min_w(px(220.)).gap_3().child(colors);
        for (label, state, value) in [
            (
                "线宽",
                &self.pen_width,
                format!("{:.0} px", self.pen_width.read(cx).value().start()),
            ),
            (
                "不透明度",
                &self.pen_opacity,
                format!("{:.0}%", self.pen_opacity.read(cx).value().start()),
            ),
        ] {
            parameters = parameters.child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().text_xs().child(label))
                            .child(self.muted(value)),
                    )
                    .child(Slider::new(state).w_full().disabled(disabled)),
            );
        }
        let mut preview_modes = h_flex().justify_center().gap_1();
        for (id, label, invalid) in [
            ("preview-normal", "正常轨迹", false),
            ("preview-invalid", "无效轨迹", true),
        ] {
            preview_modes = preview_modes.child(
                Button::new(id)
                    .ghost()
                    .small()
                    .label(label)
                    .disabled(self.modal_open())
                    .when(self.preview_invalid == invalid, |button| {
                        button.text_color(rgb(self.palette.accent))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.preview_invalid = invalid;
                        cx.notify();
                    })),
            );
        }
        let trail = self
            .settings_card()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().font_weight(FontWeight::MEDIUM).child("轨迹样式"))
                    .child(self.muted("调整绘制效果，同步预览")),
            )
            .child(
                h_flex()
                    .items_start()
                    .flex_wrap()
                    .gap_4()
                    .child(parameters)
                    .child(
                        v_flex()
                            .w(px(200.))
                            .when(narrow, |el| el.w_full())
                            .flex_shrink_0()
                            .p_3()
                            .gap_2()
                            .rounded_md()
                            .bg(rgb(self.palette.bg))
                            .border_1()
                            .border_color(rgb(self.palette.border))
                            .child(self.muted("实时预览"))
                            .child(self.pen_preview(cx))
                            .child(preview_modes),
                    ),
            );

        let logs = self
            .settings_card()
            .child(
                Button::new("toggle-logs")
                    .ghost()
                    .small()
                    .disabled(self.modal_open())
                    .label(if self.logs_expanded {
                        "▾ 日志"
                    } else {
                        "▸ 日志"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.logs_expanded = !this.logs_expanded;
                        cx.notify();
                    })),
            )
            .when(self.logs_expanded, |el| {
                el.child(
                    h_flex()
                        .justify_between()
                        .flex_wrap()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .flex_1()
                                .min_w(px(180.))
                                .child("日志级别")
                                .child(self.muted(
                                    "默认 info；debug 记录执行的动作，日志保存到配置目录。",
                                )),
                        )
                        .child(
                            div().w(px(150.)).h(px(32.)).flex_shrink_0().child(
                                Select::new(&self.log_level)
                                    .cleanable(false)
                                    .w_full()
                                    .disabled(disabled),
                            ),
                        ),
                )
                .child(
                    h_flex().child(
                        Button::new("open-log")
                            .outline()
                            .small()
                            .disabled(self.modal_open())
                            .label("打开日志")
                            .on_click(cx.listener(|this, _, _, cx| this.open_log(cx))),
                    ),
                )
            });

        div()
            .id("general-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_5()
            .pb_5()
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        self.settings_section(
                            "操作",
                            self.settings_card().child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .flex_wrap()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child("手势触发键"),
                                            )
                                            .child(self.muted("按住绘制，松开执行")),
                                    )
                                    .child(
                                        div().w(px(160.)).h(px(32.)).flex_shrink_0().child(
                                            Select::new(&self.trigger)
                                                .cleanable(false)
                                                .w_full()
                                                .disabled(disabled),
                                        ),
                                    ),
                            ),
                        ),
                    )
                    .child(
                        self.settings_section(
                            "外观",
                            self.settings_card().child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .flex_wrap()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child("界面主题"),
                                            )
                                            .child(self.muted("选择你习惯的明暗风格")),
                                    )
                                    .child(themes),
                            ),
                        ),
                    )
                    .child(
                        self.settings_section(
                            "启动",
                            self.settings_card().child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child("开机自动启动"),
                                            )
                                            .child(
                                                self.muted(
                                                    "登录 Windows 后在后台运行，切换立即保存",
                                                ),
                                            ),
                                    )
                                    .child(
                                        Switch::new("launch-at-startup")
                                            .checked(self.autostart_enabled)
                                            .disabled(
                                                !self.autostart_available || self.modal_open(),
                                            )
                                            .on_click(cx.listener(|this, checked, _, cx| {
                                                this.set_autostart(*checked, cx)
                                            })),
                                    ),
                            ),
                        ),
                    )
                    .child(self.settings_section("手势轨迹", trail))
                    .child(logs)
                    .child(
                        v_flex()
                            .gap_3()
                            .child(
                                h_flex().child(
                                    Button::new("advanced-settings")
                                        .disabled(self.modal_open())
                                        .ghost()
                                        .small()
                                        .label(if self.preference_tools {
                                            "▾ 高级"
                                        } else {
                                            "▸ 高级"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.preference_tools = !this.preference_tools;
                                            cx.notify();
                                        })),
                                ),
                            )
                            .when(self.preference_tools, |el| {
                                el.child(self.muted("识别参数和过滤规则可在 config.json 中调整。"))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .flex_wrap()
                                            .child(
                                                Button::new("open-config")
                                                    .outline()
                                                    .small()
                                                    .disabled(self.modal_open())
                                                    .label("打开配置目录")
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.open_config(cx)
                                                    })),
                                            )
                                            .child(
                                                Button::new("reload-config")
                                                    .outline()
                                                    .small()
                                                    .disabled(disabled)
                                                    .label("重新加载配置")
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.reload(cx)
                                                    })),
                                            ),
                                    )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_gesture_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled =
            self.controls_blocked() || self.application().is_some_and(|app| app.disabled);
        let rows = self.rows(cx);
        let mut list = v_flex().gap_1().p_2();
        if rows.is_empty() {
            list = list.child(div().p_3().child(self.muted(
                if self.value("search", cx).is_empty() {
                    "暂无手势，点击“新建”添加。"
                } else {
                    "没有找到匹配的手势。"
                },
            )));
        }
        for row in rows {
            let selected = self.selected.as_ref().is_some_and(|current| {
                current.source_package == row.source_package && current.action.id == row.action.id
            });
            let entry = row.clone();
            let removal = row.clone();
            let restores_global = !row.inherited
                && row.source_package != "global"
                && self.effective_config().packages.iter().any(|package| {
                    package.id == row.source_package
                        && package
                            .actions
                            .iter()
                            .any(|action| action.id == row.action.id)
                })
                && self.application().is_some_and(|app| app.inherit_global)
                && self.effective_config().packages.iter().any(|package| {
                    package.id == "global"
                        && package
                            .actions
                            .iter()
                            .any(|action| action.gesture == row.action.gesture)
                });
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "gesture-{}-{}",
                        row.source_package, row.action.id
                    )))
                    .rounded_md()
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(rgb(self.palette.selected)))
                    .hover(|el| el.bg(rgb(self.palette.hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_row(entry.clone(), window, cx)
                    }))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(42.))
                                    .h(px(44.))
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .rounded_md()
                                    .bg(rgb(self.palette.panel))
                                    .border_1()
                                    .border_color(rgb(self.palette.border))
                                    .child(self.gesture_preview(&row.action.gesture)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(row.action.name.clone()),
                                            )
                                            .when(!row.inherited, |el| {
                                                el.child(
                                                    Button::new(SharedString::from(format!(
                                                        "delete-binding-{}-{}",
                                                        row.source_package, row.action.id
                                                    )))
                                                    .ghost()
                                                    .size(px(28.))
                                                    .flex_shrink_0()
                                                    .child(
                                                        Icon::default()
                                                            .path("icons/glint-trash.svg")
                                                            .size(px(17.)),
                                                    )
                                                    .text_color(rgb(self.palette.muted))
                                                    .tooltip(if restores_global {
                                                        "删除专属绑定，恢复全局动作"
                                                    } else {
                                                        "删除绑定"
                                                    })
                                                    .disabled(disabled)
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            cx.stop_propagation();
                                                            this.request_remove_action(
                                                                removal.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                                )
                                            })
                                            .when(row.inherited, |el| {
                                                el.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(self.palette.muted))
                                                        .child("全局"),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(rgb(self.palette.muted))
                                            .child(settings_model::action_summary(
                                                &row.action.action,
                                            )),
                                    ),
                            ),
                    ),
            );
        }
        v_flex()
            .w(px(220.))
            .max_w(relative(0.42))
            .min_w(px(180.))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .flex_shrink_0()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .child(format!("手势 · {}", self.rows(cx).len())),
                            )
                            .child(
                                Button::new("new-gesture")
                                    .outline()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label("新建")
                                    .disabled(disabled)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_action(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        Input::new(&self.inputs["search"])
                            .disabled(self.modal_open())
                            .w_full()
                            .min_w_0(),
                    )
                    .when(self.application().is_some(), |el| {
                        el.child(self.muted(
                            if self.application().is_some_and(|app| app.inherit_global) {
                                "全局手势与应用专属手势"
                            } else {
                                "仅显示应用专属手势"
                            },
                        ))
                    }),
            )
            .child(
                div()
                    .id("gesture-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
            .into_any_element()
    }

    fn editable_field(&self, key: &'static str, label: &'static str, disabled: bool) -> AnyElement {
        v_flex()
            .gap_2()
            .child(self.muted(label))
            .child(Input::new(&self.inputs[key]).disabled(disabled))
            .into_any_element()
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut editor = v_flex().gap_3().p_4();
        let Some(selected) = &self.selected else {
            return div()
                .id("gesture-editor-scroll")
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .child(editor.child(self.muted("选择一个手势查看，或新建手势。")))
                .into_any_element();
        };
        let editable = self.is_editable() && !self.modal_open();
        editor = editor
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.value("action_name", cx)),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(self.palette.bg))
                            .text_xs()
                            .text_color(rgb(self.palette.muted))
                            .child(if selected.inherited {
                                "继承全局"
                            } else if self.current_scope() == Some("global") {
                                "全局"
                            } else {
                                "应用专属"
                            }),
                    ),
            )
            .child(self.muted(if self.current_scope() == Some("global") {
                "作为所有应用的默认配置".to_owned()
            } else {
                format!("仅在 {} 中生效", self.page_title())
            }));
        if selected.inherited {
            // Inherited bindings are a summary; customization opens the same editor as a new binding.
            editor = editor.child(
                v_flex()
                    .gap_3()
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(self.palette.bg))
                    .child(self.muted("当前使用全局动作。为此应用自定义后，可独立编辑。"))
                    .child(
                        h_flex().child(
                            Button::new("customize-gesture")
                                .outline()
                                .small()
                                .label("为此应用自定义")
                                .disabled(self.controls_blocked())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.customize_action(window, cx)
                                })),
                        ),
                    ),
            );
        }
        let new_binding = !self.config.as_ref().is_some_and(|config| {
            config.packages.iter().any(|p| {
                p.id == selected.source_package
                    && p.actions.iter().any(|a| a.id == selected.action.id)
            })
        });
        editor = editor.child(
            v_flex()
                .gap_3()
                .p_3()
                .rounded_lg()
                .bg(rgb(self.palette.bg))
                .border_1()
                .border_color(rgb(self.palette.border))
                .child(
                    h_flex()
                        .gap_3()
                        .child(
                            div()
                                .w(px(76.))
                                .h(px(80.))
                                .flex_shrink_0()
                                .rounded_md()
                                .bg(rgb(self.palette.panel))
                                .overflow_hidden()
                                .child(self.gesture_preview(&self.editor_gesture)),
                        )
                        .child(
                            v_flex()
                                .gap_2()
                                .child(self.muted(if selected.inherited {
                                    "执行动作"
                                } else {
                                    "手势轨迹"
                                }))
                                .when(selected.inherited, |el| {
                                    el.child(div().child(action_summary(&selected.action.action)))
                                })
                                .when(!selected.inherited, |el| {
                                    el.child(
                                        Button::new("record-gesture")
                                            .outline()
                                            .small()
                                            .label("更换手势")
                                            .disabled(!editable)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.begin_recording(cx)
                                            })),
                                    )
                                }),
                        ),
                )
                .when(new_binding && !selected.inherited, |el| {
                    el.child(
                        v_flex()
                            .gap_2()
                            .child(self.muted("选择已有手势，或录制新的轨迹"))
                            .child(
                                Select::new(&self.template)
                                    .cleanable(false)
                                    .w_full()
                                    .disabled(!editable),
                            ),
                    )
                }),
        );
        if selected.inherited {
            editor = editor.child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(self.muted("附加输入"))
                    .child(self.muted(chosen(&self.extra, cx))),
            );
        } else {
            editor = editor
                .child(self.editable_field("action_name", "名称", !editable))
                .child(
                    v_flex().gap_2().child(self.muted("动作类型")).child(
                        Select::new(&self.action_type)
                            .cleanable(false)
                            .w_full()
                            .disabled(!editable),
                    ),
                );
            match self.selected_type(cx) {
                "keys" => {
                    editor = editor.child(
                        v_flex()
                            .gap_2()
                            .child(self.muted("快捷键"))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(
                                        Input::new(&self.inputs["action_value"])
                                            .flex_1()
                                            .min_w(px(140.))
                                            .disabled(!editable || self.recording_shortcut),
                                    )
                                    .child(
                                        Button::new("record-shortcut")
                                            .outline()
                                            .small()
                                            .label(if self.recording_shortcut {
                                                "录入中…"
                                            } else {
                                                "录入快捷键"
                                            })
                                            .disabled(!editable || self.recording_shortcut)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.begin_shortcut(cx)
                                            })),
                                    ),
                            )
                            .when(self.recording_shortcut, |el| {
                                el.child(
                                    v_flex()
                                        .gap_3()
                                        .p_3()
                                        .rounded_md()
                                        .bg(rgb(self.palette.selected))
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(self.palette.accent))
                                                .child(
                                                    self.shortcut_candidate
                                                        .as_ref()
                                                        .map(|value| {
                                                            format!(
                                                                "已录入：{value} · 可再次按键替换"
                                                            )
                                                        })
                                                        .unwrap_or_else(|| {
                                                            "请按下组合键，Esc 取消".into()
                                                        }),
                                                ),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    Button::new("accept-shortcut")
                                                        .primary()
                                                        .small()
                                                        .label("使用此快捷键")
                                                        .disabled(self.shortcut_candidate.is_none())
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.accept_shortcut(window, cx)
                                                            },
                                                        )),
                                                )
                                                .child(
                                                    Button::new("cancel-shortcut")
                                                        .outline()
                                                        .small()
                                                        .label("取消")
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.cancel_shortcut(cx)
                                                        })),
                                                ),
                                        ),
                                )
                            }),
                    );
                }
                "window" => {
                    editor = editor.child(
                        v_flex().gap_2().child(self.muted("窗口动作")).child(
                            Select::new(&self.window_operation)
                                .cleanable(false)
                                .w_full()
                                .disabled(!editable),
                        ),
                    );
                }
                "launch" => {
                    editor = editor
                        .child(
                            v_flex()
                                .gap_2()
                                .child(self.muted("程序路径"))
                                .child(Input::new(&self.inputs["action_value"]).disabled(!editable))
                                .child(
                                    h_flex().child(
                                        Button::new("pick-action-program")
                                            .outline()
                                            .small()
                                            .icon(IconName::FolderOpen)
                                            .label("选择程序…")
                                            .disabled(!editable)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.pick_program(false, window, cx)
                                            })),
                                    ),
                                ),
                        )
                        .child(self.editable_field("action_args", "启动参数（可选）", !editable));
                }
                _ => {}
            }
            editor = editor.child(
                v_flex()
                    .gap_3()
                    .pt_3()
                    .border_t_1()
                    .border_color(rgb(self.palette.border))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                Button::new("toggle-extra")
                                    .ghost()
                                    .small()
                                    .icon(if self.extra_expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .label("附加输入")
                                    .disabled(self.modal_open())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.extra_expanded = !this.extra_expanded;
                                        cx.notify();
                                    })),
                            )
                            .child(self.muted(chosen(&self.extra, cx))),
                    )
                    .when(self.extra_expanded, |el| {
                        el.child(Select::new(&self.extra).cleanable(false).w_full().disabled(
                            !editable
                                || glint_core::SPECIAL_GESTURES.contains(
                                    &self.editor_gesture.split('+').next().unwrap_or_default(),
                                ),
                        ))
                    }),
            );
        }
        let validation = if self.editor_dirty && !selected.inherited {
            self.editor_action(cx).err()
        } else {
            None
        };
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .when_some(validation, |el, reason| {
                el.child(
                    v_flex()
                        .px_4()
                        .py_3()
                        .gap_2()
                        .flex_shrink_0()
                        .bg(rgb(self.palette.bg))
                        .border_b_1()
                        .border_color(rgb(self.palette.border))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .size(px(16.))
                                        .text_color(rgb(self.palette.error)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(self.palette.error))
                                        .child(format!("手势尚未完成：{reason}")),
                                ),
                        )
                        .child(self.muted("补全配置后才能保存或切换；也可以放弃此次编辑。"))
                        .child(
                            h_flex().child(
                                Button::new("discard-incomplete-editor")
                                    .outline()
                                    .small()
                                    .label("放弃此次编辑")
                                    .disabled(self.controls_blocked())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.discard_editor_changes(window, cx)
                                    })),
                            ),
                        ),
                )
            })
            .child(
                div()
                    .id("gesture-editor-scroll")
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(editor),
            )
            .into_any_element()
    }

    fn render_editor_blocked_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let reason = self.editor_blocked.as_ref()?;
        Some(self.modal_frame().child(
            v_flex().gap_3().p_5().w(px(440.)).max_w_full().rounded_lg()
                .bg(rgb(self.palette.panel)).border_1().border_color(rgb(self.palette.border))
                .child(h_flex().gap_2()
                    .child(Icon::new(IconName::TriangleAlert).size(px(22.)).text_color(rgb(self.palette.error)))
                    .child(div().text_lg().font_weight(FontWeight::MEDIUM).child("当前手势尚未完成")))
                .child(div().text_color(rgb(self.palette.error)).child(reason.clone()))
                .child(self.muted("此次操作尚未执行。继续编辑会保留所有输入；放弃只撤销当前手势此次编辑，其他已暂存配置不受影响。"))
                .child(h_flex().gap_2().justify_end().flex_wrap()
                    .child(Button::new("discard-blocked-editor").outline().small().label("放弃此次编辑")
                        .on_click(cx.listener(|this, _, window, cx| this.discard_editor_changes(window, cx))))
                    .child(Button::new("continue-blocked-editor").primary().small().label("继续编辑")
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss_editor_blocked(cx)))))
        ).into_any_element())
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let changed = self.current_dirty();
        h_flex()
            .gap_3()
            .justify_between()
            .px_5()
            .py_3()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(self.palette.border))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .when(!self.connected, |el| el.child(self.muted("后台未连接")))
                    .when(!self.notice.is_empty() && self.is_error, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(if self.is_error {
                                    self.palette.error
                                } else {
                                    self.palette.muted
                                }))
                                .child(self.notice.clone()),
                        )
                    }),
            )
            .child(
                h_flex().gap_2().flex_shrink_0().child(
                    Button::new("apply-settings")
                        .small()
                        .primary()
                        .label("应用")
                        .disabled(self.controls_blocked() || !changed)
                        .on_click(cx.listener(|this, _, _, cx| this.save_current(cx))),
                ),
            )
            .into_any_element()
    }

    fn render_app_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let draft = self.app_dialog.as_ref()?;
        let creating = draft.id.is_none();
        Some(
            self.modal_frame()
                .child(
                    v_flex()
                        .gap_3()
                        .p_5()
                        .w(px(460.))
                        .max_w_full()
                        .rounded_lg()
                        .bg(rgb(self.palette.panel))
                        .border_1()
                        .border_color(rgb(self.palette.border))
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::MEDIUM)
                                .child(if creating {
                                    "添加应用"
                                } else {
                                    "管理应用"
                                }),
                        )
                        .child(self.editable_field("app_name", "显示名称", self.pending))
                        .child(self.editable_field("app_path", "可执行程序路径", self.pending))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("pick-application-program")
                                        .outline()
                                        .disabled(self.pending)
                                        .small()
                                        .label("选择程序…")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.pick_program(true, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("capture-application")
                                        .outline()
                                        .disabled(self.pending)
                                        .small()
                                        .label("拾取窗口")
                                        .tooltip("隐藏设置后，左键点击目标窗口；按 Esc 取消")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.capture_application(window, cx)
                                        })),
                                ),
                        )
                        .when(self.is_error && !self.notice.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(self.palette.error))
                                    .child(self.notice.clone()),
                            )
                        })
                        .child(
                            h_flex()
                                .justify_between()
                                .gap_2()
                                .child(h_flex().when(!creating, |el| {
                                    el.child(
                                        Button::new("remove-application")
                                            .disabled(self.pending)
                                            .small()
                                            .outline()
                                            .label("删除应用")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.remove_application(window, cx)
                                            })),
                                    )
                                }))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new("cancel-application")
                                                .outline()
                                                .small()
                                                .label("取消")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.cancel_app_dialog(cx)
                                                })),
                                        )
                                        .child(
                                            Button::new("commit-application")
                                                .disabled(self.pending)
                                                .small()
                                                .primary()
                                                .label("确定")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.commit_app_dialog(window, cx)
                                                })),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_recording_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let recording = self.recording.as_ref()?;
        Some(
            self.modal_frame()
                .child(
                    v_flex()
                        .gap_3()
                        .p_5()
                        .w(px(440.))
                        .max_w_full()
                        .rounded_lg()
                        .bg(rgb(self.palette.panel))
                        .border_1()
                        .border_color(rgb(self.palette.border))
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::MEDIUM)
                                .child("记录新手势"),
                        )
                        .child(self.muted(if recording.ready {
                            "录制完成，确认轨迹后使用新手势。"
                        } else {
                            "按住手势触发键绘制，松开结束录制。"
                        }))
                        .child(
                            div()
                                .h(px(140.))
                                .rounded_md()
                                .bg(rgb(self.palette.bg))
                                .overflow_hidden()
                                .child(if recording.ready {
                                    self.gesture_preview(&recording.id)
                                } else {
                                    div()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(self.muted("等待绘制手势"))
                                        .into_any_element()
                                }),
                        )
                        .when(!self.notice.is_empty(), |el| {
                            el.child(self.muted(self.notice.clone()))
                        })
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    Button::new("cancel-recording")
                                        .outline()
                                        .small()
                                        .label("取消")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.cancel_recording(cx)),
                                        ),
                                )
                                .when(
                                    recording.ready
                                        || (!self.pending && self.status.recording.is_none()),
                                    |el| {
                                        el.child(
                                            Button::new("repeat-recording")
                                                .outline()
                                                .small()
                                                .label("重新录制")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.begin_recording(cx)
                                                })),
                                        )
                                    },
                                )
                                .child(
                                    Button::new("use-recording")
                                        .small()
                                        .primary()
                                        .label("使用新手势")
                                        .disabled(!recording.ready)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.use_recording(window, cx)
                                        })),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn modal_frame(&self) -> Div {
        div()
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(64.))
            .px_4()
            .bg(rgba(0x00000066))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if matches!(self.page, Page::General) {
            self.render_general(window, cx)
        } else if self.application().is_some_and(|app| app.disabled) {
            div().flex_1().into_any_element()
        } else {
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .border_t_1()
                .border_color(rgb(self.palette.border))
                .child(self.render_gesture_list(cx))
                .child(self.render_editor(cx))
                .into_any_element()
        };
        div()
            .relative()
            .size_full()
            .bg(rgb(self.palette.panel))
            .text_color(rgb(self.palette.text))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_sidebar(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .when(matches!(self.page, Page::General), |el| {
                                el.bg(rgb(self.palette.bg))
                            })
                            .child(self.render_heading(cx))
                            .child(content)
                            .child(self.render_footer(cx)),
                    ),
            )
            .children(self.render_app_modal(cx))
            .children(self.render_recording_modal(cx))
            .children(self.render_editor_blocked_modal(cx))
            .children(Root::render_dialog_layer(window, cx))
    }
}
