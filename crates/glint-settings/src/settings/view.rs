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

    pub(super) fn controls_blocked(&self) -> bool {
        !self.connected || self.pending || self.modal_open()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut global_navigation = v_flex()
            .gap_1()
            .pb_3()
            .border_b_1()
            .border_color(rgb(self.palette.border_subtle));
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
        let scope_tabs = self.scope_tabs();
        let mut navigation = v_flex().gap_1().child(global_navigation);
        navigation = navigation.child(
            h_flex()
                .justify_between()
                .items_center()
                .px_3()
                .pt_3()
                .pb_1()
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(self.palette.muted))
                        .child("应用专属"),
                )
                .when(!scope_tabs.is_empty(), |el| {
                    el.child(
                        div()
                            .px(px(6.))
                            .py(px(1.))
                            .rounded_full()
                            .bg(rgb(self.palette.elevated))
                            .text_size(px(10.))
                            .text_color(rgb(self.palette.muted))
                            .child(format!("{}", scope_tabs.len())),
                    )
                }),
        );
        for (id, name) in scope_tabs {
            navigation = navigation.child(self.navigation_item(
                format!("nav-{id}"),
                name,
                Page::Scope(id),
                cx,
            ));
        }
        navigation = navigation.child(
            div().pt_1().child(
                Button::new("add-application")
                    .ghost()
                    .small()
                    .w_full()
                    .justify_start()
                    .icon(IconName::Plus)
                    .label("添加应用")
                    .disabled(self.controls_blocked())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_app_dialog(true, window, cx)
                    })),
            ),
        );

        let (status_dot, status_label, status_tooltip) = if !self.connected {
            (self.palette.muted, "后台未连接", "正在等待后台服务连接")
        } else if self.status.paused {
            (
                self.palette.warning,
                "手势已暂停",
                "点击恢复全局鼠标手势识别",
            )
        } else {
            (
                self.palette.accent,
                "后台运行中",
                "点击临时暂停鼠标手势识别",
            )
        };
        let can_toggle_pause = self.connected && !self.pending && !self.modal_open();

        v_flex()
            .w(px(204.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(self.palette.sidebar))
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .px_3()
            .py_4()
            .child(
                h_flex()
                    .px_2()
                    .pb_4()
                    .gap(px(10.))
                    .items_center()
                    .child(
                        div()
                            .size(px(34.))
                            .flex_shrink_0()
                            .rounded(px(10.))
                            .bg(rgb(self.palette.accent))
                            .text_color(rgb(if self.palette.is_dark {
                                0x11231A
                            } else {
                                0xFFFFFF
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::Redo2).size(px(19.))),
                    )
                    .child(
                        v_flex()
                            .gap(px(1.))
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .line_height(px(20.))
                                    .font_weight(FontWeight::BOLD)
                                    .child("Glint"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(self.palette.muted))
                                    .child("掠影 · 鼠标手势"),
                            ),
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
                v_flex()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(rgb(self.palette.border_subtle))
                    .child(
                        div()
                            .id("sidebar-engine-status")
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .bg(rgb(self.palette.elevated))
                            .border_1()
                            .border_color(rgb(self.palette.border_subtle))
                            .when(can_toggle_pause, |el| {
                                el.cursor_pointer()
                                    .hover(|el| el.bg(rgb(self.palette.hover)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.send(
                                            Command::Pause {
                                                paused: !this.status.paused,
                                            },
                                            cx,
                                        );
                                    }))
                            })
                            .tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new(status_tooltip)
                                    .build(window, cx)
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .size(px(7.))
                                            .flex_shrink_0()
                                            .rounded_full()
                                            .bg(rgb(status_dot)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(self.palette.text))
                                            .child(status_label),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .px_2()
                            .justify_between()
                            .text_size(px(10.5))
                            .text_color(rgb(self.palette.muted))
                            .child(concat!("v", env!("CARGO_PKG_VERSION")))
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
        let app_disabled = match &page {
            Page::Scope(scope) if scope != "global" => self
                .application_by_id(scope)
                .is_some_and(|app| app.disabled),
            _ => false,
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
            .px(px(10.))
            .py(px(9.))
            .rounded(px(9.))
            .border_1()
            .border_color(rgb(if selected {
                self.palette.accent_border
            } else {
                self.palette.sidebar
            }))
            .text_size(px(13.5))
            .cursor_pointer()
            .text_color(rgb(if selected {
                self.palette.accent
            } else if app_disabled {
                self.palette.muted
            } else {
                self.palette.text
            }))
            .when(selected, |el| {
                el.bg(rgb(self.palette.selected))
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .hover(|el| {
                el.bg(rgb(if selected {
                    self.palette.selected
                } else {
                    self.palette.hover
                }))
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.select_page(page.clone(), window, cx)),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(3.))
                            .h(px(15.))
                            .flex_shrink_0()
                            .rounded_full()
                            .when(selected, |el| el.bg(rgb(self.palette.accent))),
                    )
                    .child(div().when(app_disabled, |el| el.opacity(0.55)).child(icon))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .when(app_disabled, |el| {
                        el.child(
                            div()
                                .px(px(5.))
                                .py(px(1.))
                                .flex_shrink_0()
                                .rounded(px(4.))
                                .bg(rgb(self.palette.elevated))
                                .text_size(px(10.))
                                .text_color(rgb(self.palette.muted))
                                .child("停用"),
                        )
                    })
                    .when(dirty, |el| {
                        el.child(
                            div()
                                .size(px(6.))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(rgb(self.palette.accent)),
                        )
                    }),
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
        if let Some(application) = application {
            let mut strategy = h_flex()
                .gap(px(2.))
                .p(px(3.))
                .flex_shrink_0()
                .rounded(px(8.))
                .bg(rgb(self.palette.elevated))
                .border_1()
                .border_color(rgb(self.palette.border_subtle));
            for (id, label, tooltip_text, inherit) in [
                (
                    "fallback-global",
                    "继承全局",
                    "未单独配置的手势执行全局动作",
                    true,
                ),
                (
                    "fallback-ignore",
                    "仅应用专属",
                    "只使用此应用单独配置的手势",
                    false,
                ),
            ] {
                let active = application.inherit_global == inherit;
                strategy = strategy.child(
                    Button::new(id)
                        .ghost()
                        .small()
                        .h(px(26.))
                        .label(label)
                        .tooltip(tooltip_text)
                        .disabled(self.controls_blocked())
                        .when(active, |button| {
                            button
                                .bg(rgb(self.palette.panel))
                                .border_1()
                                .border_color(rgb(self.palette.border))
                                .text_color(rgb(self.palette.accent))
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_application_fallback(inherit, window, cx)
                        })),
                );
            }

            return v_flex()
                .gap_3()
                .px_6()
                .pt_4()
                .pb_3()
                .flex_shrink_0()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .size(px(40.))
                                        .flex_shrink_0()
                                        .rounded(px(11.))
                                        .bg(rgb(self.palette.elevated))
                                        .border_1()
                                        .border_color(rgb(self.palette.border_subtle))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_color(rgb(self.palette.accent))
                                        .child(self.page_icon(&self.page, 22., icon)),
                                )
                                .child(
                                    v_flex()
                                        .gap(px(2.))
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_center()
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(19.))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .child(self.page_title()),
                                                )
                                                .child(
                                                    div()
                                                        .px(px(7.))
                                                        .py(px(2.))
                                                        .flex_shrink_0()
                                                        .rounded_full()
                                                        .bg(rgb(if application.disabled {
                                                            self.palette.elevated
                                                        } else {
                                                            self.palette.selected
                                                        }))
                                                        .text_size(px(11.))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(rgb(if application.disabled {
                                                            self.palette.muted
                                                        } else {
                                                            self.palette.accent
                                                        }))
                                                        .child(if application.disabled {
                                                            "已停用"
                                                        } else {
                                                            "已启用"
                                                        }),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(px(11.5))
                                                .text_color(rgb(self.palette.muted))
                                                .child(application.process_path.clone()),
                                        ),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_shrink_0()
                                .child(
                                    Button::new("change-application-program")
                                        .outline()
                                        .small()
                                        .icon(IconName::FolderOpen)
                                        .label("更改程序")
                                        .disabled(self.controls_blocked())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.change_application_program(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("delete-application-gestures")
                                        .ghost()
                                        .size(px(32.))
                                        .child(
                                            Icon::default()
                                                .path("icons/glint-trash.svg")
                                                .size(px(18.)),
                                        )
                                        .tooltip("删除应用手势")
                                        .text_color(rgb(self.palette.error))
                                        .disabled(self.controls_blocked())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.remove_application(window, cx)
                                        })),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .px_3()
                        .py(px(8.))
                        .gap_3()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .rounded(px(10.))
                        .bg(rgb(self.palette.bg))
                        .border_1()
                        .border_color(rgb(self.palette.border_subtle))
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(
                                    Switch::new("enable-application")
                                        .checked(!application.disabled)
                                        .disabled(self.controls_blocked())
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.set_application_disabled(!*checked, cx)
                                        })),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("在此应用中启用手势"),
                                ),
                        )
                        .when(!application.disabled, |el| {
                            el.child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(self.muted("未配置手势"))
                                    .child(strategy),
                            )
                        }),
                )
                .into_any_element();
        }

        v_flex()
            .px_6()
            .pt_5()
            .pb_4()
            .flex_shrink_0()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size(px(40.))
                            .flex_shrink_0()
                            .rounded(px(11.))
                            .bg(rgb(self.palette.selected))
                            .border_1()
                            .border_color(rgb(self.palette.accent_border))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(self.palette.accent))
                            .child(self.page_icon(&self.page, 20., icon)),
                    )
                    .child(
                        v_flex()
                            .gap(px(2.))
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.page_title()),
                            )
                            .child(self.muted(self.page_description())),
                    ),
            )
            .into_any_element()
    }

    fn muted(&self, text: impl Into<SharedString>) -> AnyElement {
        div()
            .text_size(px(12.))
            .text_color(rgb(self.palette.muted))
            .child(text.into())
            .into_any_element()
    }

    fn settings_card(&self) -> Div {
        v_flex()
            .w_full()
            .min_w_0()
            .p_4()
            .gap_4()
            .rounded(px(12.))
            .bg(rgb(self.palette.panel))
            .border_1()
            .border_color(rgb(self.palette.border))
    }

    fn settings_section(&self, title: &'static str, content: impl IntoElement) -> Div {
        v_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .px_1()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(self.palette.muted))
                    .child(title),
            )
            .child(content)
    }

    fn settings_row(
        &self,
        title: &'static str,
        description: &'static str,
        control: impl IntoElement,
    ) -> Div {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(220.))
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(self.muted(description)),
            )
            .child(control)
    }

    fn render_general(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.controls_blocked();
        let narrow = window.viewport_size().width < px(860.);
        let pen = self.pen(cx);
        let mut themes = h_flex()
            .gap(px(3.))
            .p(px(3.))
            .flex_shrink_0()
            .bg(rgb(self.palette.elevated))
            .border_1()
            .border_color(rgb(self.palette.border_subtle))
            .rounded(px(9.));
        for (id, label, appearance) in [
            ("theme-system", "跟随系统", Appearance::System),
            ("theme-light", "浅色", Appearance::Light),
            ("theme-dark", "深色", Appearance::Dark),
        ] {
            let active = self.ui_settings.appearance == appearance;
            themes = themes.child(
                Button::new(id)
                    .disabled(self.modal_open())
                    .small()
                    .ghost()
                    .h(px(28.))
                    .label(label)
                    .when(active, |button| {
                        button
                            .bg(rgb(self.palette.panel))
                            .border_1()
                            .border_color(rgb(self.palette.border))
                            .text_color(rgb(self.palette.accent))
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_appearance(appearance, window, cx)
                    })),
            );
        }
        let mut colors = h_flex().gap_3().flex_wrap();
        for (id, label, hex, state) in [
            ("disabled-pen-color", "正常颜色", &pen.color, &self.pen_color),
            (
                "disabled-pen-invalid-color",
                "无效颜色",
                &pen.invalid_color,
                &self.pen_invalid_color,
            ),
        ] {
            colors = colors.child(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(9.))
                    .bg(rgb(self.palette.bg))
                    .border_1()
                    .border_color(rgb(self.palette.border_subtle))
                    .child(if disabled {
                        div()
                            .id(id)
                            .size(px(22.))
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
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .font_family("Consolas")
                                    .text_color(rgb(self.palette.muted))
                                    .child(hex.clone()),
                            ),
                    ),
            );
        }
        let mut parameters = v_flex().flex_1().min_w(px(220.)).gap_4().child(colors);
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
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(label),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py(px(2.))
                                    .rounded(px(6.))
                                    .bg(rgb(self.palette.elevated))
                                    .border_1()
                                    .border_color(rgb(self.palette.border_subtle))
                                    .font_family("Consolas")
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(value),
                            ),
                    )
                    .child(Slider::new(state).w_full().disabled(disabled)),
            );
        }
        let mut preview_modes = h_flex()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(8.))
            .bg(rgb(self.palette.panel))
            .border_1()
            .border_color(rgb(self.palette.border_subtle));
        for (id, label, invalid) in [
            ("preview-normal", "正常轨迹", false),
            ("preview-invalid", "无效轨迹", true),
        ] {
            let active = self.preview_invalid == invalid;
            preview_modes = preview_modes.child(
                Button::new(id)
                    .ghost()
                    .small()
                    .flex_1()
                    .h(px(26.))
                    .label(label)
                    .disabled(self.modal_open())
                    .when(active, |button| {
                        button
                            .bg(rgb(self.palette.selected))
                            .text_color(rgb(self.palette.accent))
                            .font_weight(FontWeight::SEMIBOLD)
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
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child("轨迹样式"),
                    )
                    .child(self.muted("调整绘制颜色、线宽与不透明度，右侧实时预览")),
            )
            .child(
                h_flex()
                    .items_start()
                    .flex_wrap()
                    .gap_4()
                    .child(parameters)
                    .child(
                        v_flex()
                            .w(px(228.))
                            .when(narrow, |el| el.w_full())
                            .flex_shrink_0()
                            .p_3()
                            .gap_2()
                            .rounded(px(10.))
                            .bg(rgb(self.palette.elevated))
                            .border_1()
                            .border_color(rgb(self.palette.border_subtle))
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(self.muted("实时预览"))
                                    .child(
                                        div()
                                            .font_family("Consolas")
                                            .text_size(px(10.5))
                                            .text_color(rgb(self.palette.muted))
                                            .child(format!(
                                                "{:.0}px · {:.0}%",
                                                pen.width,
                                                pen.opacity * 100.
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .rounded(px(8.))
                                    .bg(rgb(self.palette.panel))
                                    .border_1()
                                    .border_color(rgb(self.palette.border_subtle))
                                    .child(self.pen_preview(cx)),
                            )
                            .child(preview_modes),
                    ),
            );

        let advanced = self
            .settings_card()
            .child(
                div()
                    .id("advanced-settings")
                    .cursor_pointer()
                    .rounded(px(8.))
                    .when(!self.modal_open(), |el| {
                        el.on_click(cx.listener(|this, _, _, cx| {
                            this.advanced_expanded = !this.advanced_expanded;
                            cx.notify();
                        }))
                    })
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("高级与日志"),
                                    )
                                    .child(self.muted("日志级别、配置目录与热重载")),
                            )
                            .child(
                                Icon::new(if self.advanced_expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(16.))
                                .text_color(rgb(self.palette.muted)),
                            ),
                    ),
            )
            .when(self.advanced_expanded, |el| {
                el.child(
                    v_flex()
                        .pt_4()
                        .gap_4()
                        .border_t_1()
                        .border_color(rgb(self.palette.border_subtle))
                        .child(
                            self.settings_row(
                                "日志级别",
                                "默认 info；debug 记录执行的动作，日志保存到配置目录。",
                                div().w(px(150.)).flex_shrink_0().child(
                                    Select::new(&self.log_level)
                                        .cleanable(false)
                                        .w_full()
                                        .disabled(disabled),
                                ),
                            ),
                        )
                        .child(self.muted("识别参数和过滤规则可在 config.json 中调整。"))
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    Button::new("open-log")
                                        .outline()
                                        .small()
                                        .disabled(self.modal_open())
                                        .label("打开日志")
                                        .on_click(cx.listener(|this, _, _, cx| this.open_log(cx))),
                                )
                                .child(
                                    Button::new("open-config")
                                        .outline()
                                        .small()
                                        .disabled(self.modal_open())
                                        .icon(IconName::FolderOpen)
                                        .label("打开配置目录")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.open_config(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("reload-config")
                                        .outline()
                                        .small()
                                        .disabled(disabled)
                                        .label("重新加载配置")
                                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                                ),
                        ),
                )
            });

        div()
            .id("general-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_6()
            .pb_6()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(1040.))
                    .gap_4()
                    .child(
                        self.settings_section(
                            "基础偏好",
                            self.settings_card()
                                .child(
                                    self.settings_row(
                                        "手势触发键",
                                        "按住绘制轨迹，松开执行动作",
                                        div().w(px(168.)).flex_shrink_0().child(
                                            Select::new(&self.trigger)
                                                .cleanable(false)
                                                .w_full()
                                                .disabled(disabled),
                                        ),
                                    )
                                    .pb_4()
                                    .border_b_1()
                                    .border_color(rgb(self.palette.border_subtle)),
                                )
                                .child(
                                    self.settings_row(
                                        "界面主题",
                                        "选择你习惯的明暗风格，切换立即保存",
                                        themes,
                                    )
                                    .pb_4()
                                    .border_b_1()
                                    .border_color(rgb(self.palette.border_subtle)),
                                )
                                .child(
                                    self.settings_row(
                                        "开机自动启动",
                                        "登录 Windows 后在后台静默运行，切换立即保存",
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
                    .child(advanced),
            )
            .into_any_element()
    }

    fn render_disabled_application(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .border_t_1()
            .border_color(rgb(self.palette.border_subtle))
            .bg(rgb(self.palette.bg))
            .child(
                v_flex()
                    .w(px(360.))
                    .max_w_full()
                    .items_center()
                    .gap_3()
                    .p_6()
                    .rounded(px(14.))
                    .bg(rgb(self.palette.panel))
                    .border_1()
                    .border_color(rgb(self.palette.border))
                    .child(
                        div()
                            .size(px(44.))
                            .rounded_full()
                            .bg(rgb(self.palette.elevated))
                            .border_1()
                            .border_color(rgb(self.palette.border_subtle))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(self.palette.muted))
                            .child(Icon::new(IconName::Minus).size(px(20.))),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("已在此应用中停用手势"),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(px(12.5))
                            .text_color(rgb(self.palette.muted))
                            .child("Glint 不会识别或拦截此应用的鼠标输入，已配置的专属手势会安全保留。"),
                    )
                    .child(
                        div().pt_1().child(
                            Button::new("reenable-application")
                                .outline()
                                .small()
                                .label("重新启用手势")
                                .disabled(self.controls_blocked())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_application_disabled(false, cx)
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_gesture_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled =
            self.controls_blocked() || self.application().is_some_and(|app| app.disabled);
        let is_app_inherit = self.application().is_some_and(|app| app.inherit_global);
        let rows = self.rows(cx);
        let row_count = rows.len();
        let mut list = v_flex().gap(px(5.)).px_3().pb_3();
        if rows.is_empty() {
            list = list.child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .py_6()
                    .px_3()
                    .rounded(px(10.))
                    .bg(rgb(self.palette.bg))
                    .child(self.muted(if self.value("search", cx).is_empty() {
                        "暂无手势，点击上方“新建”添加。"
                    } else {
                        "没有找到匹配的手势。"
                    })),
            );
        }
        let effective = self.effective_config();
        let current_scope = self.current_scope();
        let scope_action_ids: std::collections::HashSet<&str> = current_scope
            .and_then(|scope| effective.packages.iter().find(|p| p.id == scope))
            .map(|p| p.actions.iter().map(|a| a.id.as_str()).collect())
            .unwrap_or_default();
        let global_gestures: std::collections::HashSet<&str> = if is_app_inherit {
            effective
                .packages
                .iter()
                .find(|p| p.id == "global")
                .map(|p| p.actions.iter().map(|a| a.gesture.as_str()).collect())
                .unwrap_or_default()
        } else {
            std::collections::HashSet::new()
        };
        for row in rows {
            let selected = self.selected.as_ref().is_some_and(|current| {
                current.source_package == row.source_package && current.action.id == row.action.id
            });
            let entry = row.clone();
            let removal = row.clone();
            let (_, extra_id) = split_gesture(&row.action.gesture);
            let extra_badge = if extra_id.is_empty() {
                None
            } else {
                Some(format!("+ {}", self.gesture_label(&extra_id)))
            };
            let restores_global = !row.inherited
                && row.source_package != "global"
                && is_app_inherit
                && scope_action_ids.contains(row.action.id.as_str())
                && global_gestures.contains(row.action.gesture.as_str());
            let group_name = SharedString::from(format!(
                "gesture-row-{}-{}",
                row.source_package, row.action.id
            ));
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "gesture-{}-{}",
                        row.source_package, row.action.id
                    )))
                    .group(group_name.clone())
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(if selected {
                        self.palette.accent_border
                    } else {
                        self.palette.panel
                    }))
                    .p(px(8.))
                    .cursor_pointer()
                    .when(selected, |el| el.bg(rgb(self.palette.selected)))
                    .hover(|el| {
                        el.bg(rgb(if selected {
                            self.palette.selected
                        } else {
                            self.palette.hover
                        }))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_row(entry.clone(), window, cx)
                    }))
                    .child(
                        h_flex()
                            .gap(px(10.))
                            .items_center()
                            .child(
                                div()
                                    .size(px(42.))
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .rounded(px(8.))
                                    .bg(rgb(self.palette.panel))
                                    .border_1()
                                    .border_color(rgb(if selected {
                                        self.palette.accent_border
                                    } else {
                                        self.palette.border_subtle
                                    }))
                                    .child(self.gesture_preview(&row.action.gesture)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(3.))
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(13.5))
                                                    .font_weight(if selected {
                                                        FontWeight::SEMIBOLD
                                                    } else {
                                                        FontWeight::MEDIUM
                                                    })
                                                    .when(selected, |el| {
                                                        el.text_color(rgb(self.palette.accent))
                                                    })
                                                    .child(row.action.name.clone()),
                                            )
                                            .when(!row.inherited && is_app_inherit, |el| {
                                                el.child(
                                                    div()
                                                        .px(px(5.))
                                                        .py(px(1.))
                                                        .flex_shrink_0()
                                                        .rounded(px(4.))
                                                        .bg(rgb(self.palette.selected))
                                                        .text_size(px(10.))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(rgb(self.palette.accent))
                                                        .child("专属"),
                                                )
                                            })
                                            .when(!row.inherited, |el| {
                                                el.child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .when(!selected, |el| {
                                                            el.opacity(0.0)
                                                                .group_hover(group_name, |style| {
                                                                    style.opacity(1.0)
                                                                })
                                                        })
                                                        .child(
                                                            Button::new(SharedString::from(
                                                                format!(
                                                                    "delete-binding-{}-{}",
                                                                    row.source_package,
                                                                    row.action.id
                                                                ),
                                                            ))
                                                            .ghost()
                                                            .size(px(24.))
                                                            .child(
                                                                Icon::default()
                                                                    .path("icons/glint-trash.svg")
                                                                    .size(px(14.)),
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
                                                        ),
                                                )
                                            })
                                            .when(row.inherited, |el| {
                                                el.child(
                                                    div()
                                                        .text_size(px(10.))
                                                        .px(px(5.))
                                                        .py(px(1.))
                                                        .flex_shrink_0()
                                                        .rounded(px(4.))
                                                        .bg(rgb(self.palette.elevated))
                                                        .text_color(rgb(self.palette.muted))
                                                        .child("全局"),
                                                )
                                            }),
                                    )
                                    .child(
                                        h_flex()
                                            .gap(px(5.))
                                            .items_center()
                                            .min_w_0()
                                            .when_some(extra_badge, |el, badge| {
                                                el.child(
                                                    div()
                                                        .px(px(5.))
                                                        .py(px(1.))
                                                        .flex_shrink_0()
                                                        .rounded(px(4.))
                                                        .bg(rgb(if selected {
                                                            self.palette.panel
                                                        } else {
                                                            self.palette.elevated
                                                        }))
                                                        .border_1()
                                                        .border_color(rgb(if selected {
                                                            self.palette.accent_border
                                                        } else {
                                                            self.palette.border_subtle
                                                        }))
                                                        .text_size(px(10.))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(rgb(if selected {
                                                            self.palette.accent
                                                        } else {
                                                            self.palette.muted
                                                        }))
                                                        .child(badge),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(11.5))
                                                    .text_color(rgb(self.palette.muted))
                                                    .child(settings_model::action_summary(
                                                        &row.action.action,
                                                    )),
                                            ),
                                    ),
                            ),
                    ),
            );
        }
        v_flex()
            .w(px(272.))
            .max_w(relative(0.42))
            .min_w(px(216.))
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
                            .items_center()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("手势"),
                                    )
                                    .child(
                                        div()
                                            .px(px(6.))
                                            .py(px(1.))
                                            .rounded_full()
                                            .bg(rgb(self.palette.elevated))
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(self.palette.muted))
                                            .child(format!("{row_count}")),
                                    ),
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
                            .cleanable(true)
                            .disabled(self.modal_open())
                            .w_full()
                            .min_w_0(),
                    )
                    .when(self.application().is_some(), |el| {
                        el.child(self.muted(if is_app_inherit {
                            "显示全局手势与应用专属手势"
                        } else {
                            "仅显示应用专属手势"
                        }))
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
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(self.palette.muted))
                    .child(label),
            )
            .child(Input::new(&self.inputs[key]).disabled(disabled))
            .into_any_element()
    }

    fn render_keycaps(&self, shortcut: &str) -> AnyElement {
        let mut caps = h_flex().gap(px(4.)).items_center().flex_wrap();
        let parts: Vec<&str> = shortcut.split('+').filter(|p| !p.is_empty()).collect();
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                caps = caps.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(self.palette.muted))
                        .child("+"),
                );
            }
            caps = caps.child(
                div()
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .bg(rgb(self.palette.panel))
                    .border_1()
                    .border_color(rgb(self.palette.accent_border))
                    .font_family("Consolas")
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(self.palette.text))
                    .child((*part).to_owned()),
            );
        }
        caps.into_any_element()
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut editor = v_flex().gap_4().p_5();
        let Some(selected) = &self.selected else {
            let can_create =
                !self.controls_blocked() && !self.application().is_some_and(|app| app.disabled);
            return div()
                .id("gesture-editor-scroll")
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .child(
                    v_flex()
                        .items_center()
                        .gap_3()
                        .max_w(px(300.))
                        .child(
                            div()
                                .size(px(44.))
                                .rounded_full()
                                .bg(rgb(self.palette.elevated))
                                .border_1()
                                .border_color(rgb(self.palette.border_subtle))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgb(self.palette.muted))
                                .child(Icon::new(IconName::Redo2).size(px(20.))),
                        )
                        .child(
                            div()
                                .text_size(px(14.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("尚未选择手势"),
                        )
                        .child(
                            div()
                                .text_center()
                                .text_size(px(12.))
                                .text_color(rgb(self.palette.muted))
                                .child("从左侧列表选择一条手势查看详情，或新建一条手势绑定。"),
                        )
                        .child(
                            Button::new("empty-new-gesture")
                                .primary()
                                .small()
                                .icon(IconName::Plus)
                                .label("新建手势")
                                .disabled(!can_create)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.new_action(window, cx)
                                })),
                        ),
                )
                .into_any_element();
        };
        let editable = self.is_editable() && !self.modal_open();
        let extra_id = EXTRA_IDS[EXTRAS
            .iter()
            .position(|v| *v == chosen(&self.extra, cx))
            .unwrap_or(0)];
        let current_gesture_display = if selected.inherited {
            self.gesture_label(&selected.action.gesture)
        } else {
            self.gesture_label(&join_gesture(&self.editor_gesture, extra_id))
        };
        let action_title = self.value("action_name", cx);
        let action_title = if action_title.trim().is_empty() {
            "未命名手势".to_owned()
        } else {
            action_title
        };

        editor = editor
            .child(
                v_flex()
                    .gap(px(3.))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(action_title),
                            )
                            .child(
                                div()
                                    .px(px(8.))
                                    .py(px(3.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(rgb(if selected.inherited {
                                        self.palette.elevated
                                    } else {
                                        self.palette.selected
                                    }))
                                    .border_1()
                                    .border_color(rgb(if selected.inherited {
                                        self.palette.border_subtle
                                    } else {
                                        self.palette.accent_border
                                    }))
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(if selected.inherited {
                                        self.palette.muted
                                    } else {
                                        self.palette.accent
                                    }))
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
                    } else if selected.inherited {
                        format!("在 {} 中沿用全局动作", self.page_title())
                    } else {
                        format!("仅在 {} 中生效", self.page_title())
                    })),
            );
        if selected.inherited {
            editor = editor.child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .flex_wrap()
                    .p(px(12.))
                    .rounded(px(10.))
                    .bg(rgb(self.palette.selected))
                    .border_1()
                    .border_color(rgb(self.palette.accent_border))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(180.))
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(self.palette.text))
                                    .child("当前沿用全局配置"),
                            )
                            .child(self.muted("为此应用自定义后，可独立修改而不影响其他应用。")),
                    )
                    .child(
                        Button::new("customize-gesture")
                            .primary()
                            .small()
                            .label("为此应用自定义")
                            .disabled(self.controls_blocked())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.customize_action(window, cx)
                            })),
                    ),
            );
        }
        editor = editor.child(
            v_flex()
                .gap_3()
                .p_3()
                .rounded(px(10.))
                .bg(rgb(self.palette.bg))
                .border_1()
                .border_color(rgb(self.palette.border_subtle))
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .size(px(80.))
                                .flex_shrink_0()
                                .rounded(px(9.))
                                .bg(rgb(self.palette.panel))
                                .border_1()
                                .border_color(rgb(self.palette.border))
                                .overflow_hidden()
                                .child(self.gesture_preview(&self.editor_gesture)),
                        )
                        .child(
                            v_flex()
                                .gap(px(6.))
                                .flex_1()
                                .min_w_0()
                                .child(self.muted(if selected.inherited {
                                    "触发轨迹与动作"
                                } else {
                                    "手势轨迹"
                                }))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(current_gesture_display),
                                )
                                .when(selected.inherited, |el| {
                                    el.child(
                                        div()
                                            .text_size(px(12.5))
                                            .text_color(rgb(self.palette.muted))
                                            .child(action_summary(&selected.action.action)),
                                    )
                                })
                                .when(!selected.inherited, |el| {
                                    el.child(
                                        h_flex().gap_2().child(
                                            Button::new("record-gesture")
                                                .outline()
                                                .small()
                                                .label("录制新手势")
                                                .disabled(!editable)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.begin_recording(cx)
                                                })),
                                        ),
                                    )
                                }),
                        ),
                )
                .when(!selected.inherited, |el| {
                    el.child(
                        v_flex()
                            .gap(px(6.))
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(self.palette.border_subtle))
                            .child(self.muted("或从内置 / 已录制模板中选择"))
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
                    .items_center()
                    .px_3()
                    .py(px(8.))
                    .rounded(px(8.))
                    .bg(rgb(self.palette.bg))
                    .child(self.muted("附加输入"))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(chosen(&self.extra, cx)),
                    ),
            );
        } else {
            editor = editor
                .child(self.editable_field("action_name", "名称", !editable))
                .child(
                    v_flex()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(self.palette.muted))
                                .child("动作类型"),
                        )
                        .child(
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
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(self.palette.muted))
                                    .child("快捷键"),
                            )
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
                                        .rounded(px(10.))
                                        .bg(rgb(self.palette.selected))
                                        .border_1()
                                        .border_color(rgb(self.palette.accent_border))
                                        .child(
                                            if let Some(candidate) = &self.shortcut_candidate {
                                                v_flex()
                                                    .gap_2()
                                                    .child(self.muted("已录入组合键（可继续按键替换）："))
                                                    .child(self.render_keycaps(candidate))
                                                    .into_any_element()
                                            } else {
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(self.palette.accent))
                                                    .child("请直接按下目标组合键，按 Esc 取消")
                                                    .into_any_element()
                                            },
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
                        v_flex()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(self.palette.muted))
                                    .child("窗口动作"),
                            )
                            .child(
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
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(self.palette.muted))
                                        .child("程序路径"),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Input::new(&self.inputs["action_value"])
                                                .flex_1()
                                                .min_w(px(140.))
                                                .disabled(!editable),
                                        )
                                        .child(
                                            Button::new("pick-action-program")
                                                .outline()
                                                .small()
                                                .icon(IconName::FolderOpen)
                                                .label("浏览…")
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
            let extra_value = chosen(&self.extra, cx);
            editor = editor.child(
                v_flex()
                    .gap_3()
                    .pt_3()
                    .border_t_1()
                    .border_color(rgb(self.palette.border_subtle))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
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
                            .child(
                                div()
                                    .px(px(7.))
                                    .py(px(2.))
                                    .rounded(px(6.))
                                    .bg(rgb(if extra_value == "无" {
                                        self.palette.elevated
                                    } else {
                                        self.palette.selected
                                    }))
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(if extra_value == "无" {
                                        self.palette.muted
                                    } else {
                                        self.palette.accent
                                    }))
                                    .child(extra_value),
                            ),
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
                        .px_5()
                        .py_3()
                        .gap_2()
                        .flex_shrink_0()
                        .bg(rgb(self.palette.error_bg))
                        .border_b_1()
                        .border_color(rgb(self.palette.error_border))
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .size(px(15.))
                                        .text_color(rgb(self.palette.error)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(self.palette.error))
                                        .child(format!("手势尚未完成：{reason}")),
                                ),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .child(self.muted("补全配置后才能保存或切换。"))
                                .child(
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
        Some(
            self.modal_frame()
                .child(
                    v_flex()
                        .gap_3()
                        .p_5()
                        .w(px(440.))
                        .max_w_full()
                        .flex_shrink_0()
                        .rounded(px(14.))
                        .bg(rgb(self.palette.panel))
                        .border_1()
                        .border_color(rgb(self.palette.border))
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .size(px(20.))
                                        .text_color(rgb(self.palette.error)),
                                )
                                .child(
                                    div()
                                        .text_size(px(16.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("当前手势尚未完成"),
                                ),
                        )
                        .child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded(px(8.))
                                .bg(rgb(self.palette.error_bg))
                                .border_1()
                                .border_color(rgb(self.palette.error_border))
                                .text_sm()
                                .text_color(rgb(self.palette.error))
                                .child(reason.clone()),
                        )
                        .child(self.muted(
                            "此次操作尚未执行。继续编辑会保留所有输入；放弃只撤销当前手势此次编辑，其他已暂存配置不受影响。",
                        ))
                        .child(
                            h_flex()
                                .pt_1()
                                .gap_2()
                                .justify_end()
                                .flex_wrap()
                                .child(
                                    Button::new("discard-blocked-editor")
                                        .outline()
                                        .small()
                                        .label("放弃此次编辑")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.discard_editor_changes(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("continue-blocked-editor")
                                        .primary()
                                        .small()
                                        .label("继续编辑")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.dismiss_editor_blocked(cx)
                                        })),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let changed = self.current_dirty();
        let status_text = if self.pending {
            "正在处理…".to_owned()
        } else if changed {
            "有未应用的更改".to_owned()
        } else if !self.notice.is_empty() {
            self.notice.clone()
        } else {
            "当前页面已保存".to_owned()
        };
        h_flex()
            .gap_3()
            .items_center()
            .justify_between()
            .px_6()
            .py(px(11.))
            .flex_shrink_0()
            .bg(rgb(self.palette.panel))
            .border_t_1()
            .border_color(rgb(self.palette.border))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .when(!self.connected, |el| {
                        el.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .size(px(7.))
                                        .rounded_full()
                                        .bg(rgb(self.palette.muted)),
                                )
                                .child(self.muted("后台未连接")),
                        )
                    })
                    .when(self.connected && !self.is_error, |el| {
                        el.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Icon::new(if changed {
                                        IconName::Info
                                    } else {
                                        IconName::CircleCheck
                                    })
                                    .size(px(14.))
                                    .text_color(rgb(if changed {
                                        self.palette.accent
                                    } else {
                                        self.palette.muted
                                    })),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(rgb(if changed {
                                            self.palette.text
                                        } else {
                                            self.palette.muted
                                        }))
                                        .child(status_text),
                                )
                                .when(changed && !self.pending, |el| {
                                    el.child(
                                        div()
                                            .px(px(6.))
                                            .py(px(1.))
                                            .rounded(px(4.))
                                            .bg(rgb(self.palette.elevated))
                                            .border_1()
                                            .border_color(rgb(self.palette.border_subtle))
                                            .font_family("Consolas")
                                            .text_size(px(10.5))
                                            .text_color(rgb(self.palette.muted))
                                            .child("Ctrl+S"),
                                    )
                                }),
                        )
                    })
                    .when(!self.notice.is_empty() && self.is_error, |el| {
                        el.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .size(px(14.))
                                        .text_color(rgb(self.palette.error)),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(rgb(self.palette.error))
                                        .child(self.notice.clone()),
                                ),
                        )
                    }),
            )
            .child(
                h_flex().gap_2().flex_shrink_0().child(
                    Button::new("apply-settings")
                        .h(px(34.))
                        .w(px(88.))
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
                        .gap_4()
                        .p_5()
                        .w(px(460.))
                        .max_w_full()
                        .flex_shrink_0()
                        .rounded(px(14.))
                        .bg(rgb(self.palette.panel))
                        .border_1()
                        .border_color(rgb(self.palette.border))
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .text_size(px(16.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(if creating {
                                            "添加应用"
                                        } else {
                                            "管理应用"
                                        }),
                                )
                                .child(self.muted("通过选择可执行文件或拾取窗口来定位目标应用")),
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
                                        .icon(IconName::FolderOpen)
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
                                    .px_3()
                                    .py_2()
                                    .rounded(px(8.))
                                    .bg(rgb(self.palette.error_bg))
                                    .border_1()
                                    .border_color(rgb(self.palette.error_border))
                                    .text_xs()
                                    .text_color(rgb(self.palette.error))
                                    .child(self.notice.clone()),
                            )
                        })
                        .child(
                            h_flex()
                                .pt_1()
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
                        .gap_4()
                        .p_5()
                        .w(px(440.))
                        .max_w_full()
                        .flex_shrink_0()
                        .rounded(px(14.))
                        .bg(rgb(self.palette.panel))
                        .border_1()
                        .border_color(rgb(self.palette.border))
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .text_size(px(16.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("记录新手势"),
                                )
                                .child(self.muted(if recording.ready {
                                    "录制完成，确认轨迹无误后点击“使用新手势”。"
                                } else {
                                    "按住手势触发键在屏幕任意位置绘制，松开结束录制。"
                                })),
                        )
                        .child(
                            div()
                                .h(px(152.))
                                .rounded(px(10.))
                                .bg(rgb(self.palette.elevated))
                                .border_1()
                                .border_color(rgb(if recording.ready {
                                    self.palette.accent_border
                                } else {
                                    self.palette.border_subtle
                                }))
                                .overflow_hidden()
                                .child(if recording.ready {
                                    self.gesture_preview(&recording.id)
                                } else {
                                    v_flex()
                                        .size_full()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(IconName::Redo2)
                                                .size(px(22.))
                                                .text_color(rgb(self.palette.accent)),
                                        )
                                        .child(self.muted("等待绘制手势…"))
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

    fn modal_frame(&self) -> Stateful<Div> {
        div()
            .id("settings-modal-scroll")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .overflow_y_scroll()
            .py_6()
            .px_4()
            .bg(rgba(0x00000078))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if matches!(self.page, Page::General) {
            self.render_general(window, cx)
        } else if self.application().is_some_and(|app| app.disabled) {
            self.render_disabled_application(cx)
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
            .font(Font {
                fallbacks: Some(FontFallbacks::from_fonts(vec![
                    "Microsoft YaHei UI".into(),
                    "Segoe UI Symbol".into(),
                ])),
                ..font("Segoe UI")
            })
            .text_size(px(14.))
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
