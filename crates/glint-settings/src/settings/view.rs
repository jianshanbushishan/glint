use super::*;
use gpui_component::{Icon, IconName};

impl SettingsView {
    fn modal_open(&self) -> bool {
        self.app_dialog.is_some() || self.recording.is_some()
    }

    fn controls_blocked(&self) -> bool {
        !self.connected || self.pending || self.modal_open()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut navigation = v_flex().gap_1();
        for (id, label, page) in [
            (
                "nav-general".to_owned(),
                "常规配置".to_owned(),
                Page::General,
            ),
            (
                "nav-global".to_owned(),
                "全局手势".to_owned(),
                Page::Scope("global".into()),
            ),
        ] {
            navigation = navigation.child(self.navigation_item(id, label, page, cx));
        }
        navigation = navigation.child(
            div()
                .px_3()
                .pt_5()
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
                .outline()
                .label("＋ 添加应用")
                .w_full()
                .disabled(self.controls_blocked())
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_app_dialog(true, window, cx)),
                ),
        );
        v_flex()
            .w(px(164.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(self.palette.bg))
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .px_2()
            .py_4()
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
                    .pt_5()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
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
            Page::General => IconName::Settings,
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
                    .items_center()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_center()
                            .gap_2()
                            .child(Icon::new(icon).size(px(16.)).flex_shrink_0())
                            .child(div().min_w_0().truncate().child(label)),
                    )
                    .when(dirty, |el| el.child(div().text_xs().child("· 未应用"))),
            )
            .into_any_element()
    }

    fn render_heading(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut heading = v_flex()
            .gap_1()
            .px_6()
            .pt_5()
            .pb_4()
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(self.palette.border))
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.page_title()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .truncate()
                    .child(self.page_description()),
            );
        if let Some(application) = self.application() {
            heading = heading.child(
                h_flex()
                    .mt_3()
                    .gap_4()
                    .flex_wrap()
                    .child(
                        Switch::new("disable-application")
                            .label("在此应用中禁用手势")
                            .checked(application.disabled)
                            .disabled(self.controls_blocked())
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.set_application_disabled(*checked, cx)
                            })),
                    )
                    .child(
                        Select::new(&self.fallback)
                            .cleanable(false)
                            .w(px(225.))
                            .disabled(self.controls_blocked() || application.disabled),
                    ),
            );
            if application.disabled {
                heading = heading.child(
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(rgb(self.palette.error))
                        .child(if self.current_scope().and_then(|scope| self.scope_drafts.get(scope)).is_some_and(|draft| draft.dirty) {
                            "应用设置后生效：此应用内将不再识别或拦截手势，保留全部配置以便恢复。"
                        } else {
                            "此应用内不识别或拦截手势，保留全部配置以便恢复。"
                        }),
                );
            }
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

    fn section(&self, title: &'static str) -> Div {
        v_flex()
            .gap_4()
            .py_5()
            .border_b_1()
            .border_color(rgb(self.palette.border))
            .child(div().font_weight(FontWeight::MEDIUM).child(title))
    }

    fn render_general(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.controls_blocked();
        let mut themes = h_flex()
            .gap_1()
            .p_1()
            .border_1()
            .border_color(rgb(self.palette.border))
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
                    .outline()
                    .label(label)
                    .when(self.ui_settings.appearance == appearance, |button| {
                        button.primary()
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_appearance(appearance, window, cx)
                    })),
            );
        }
        let width = self.pen_width.read(cx).value().start();
        let opacity = self.pen_opacity.read(cx).value().start();
        let trail = h_flex()
            .items_start()
            .gap_6()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_4()
                    .child(
                        h_flex()
                            .gap_3()
                            .child(div().w(px(70.)).flex_shrink_0().child("正常颜色"))
                            .child(if disabled {
                                Button::new("disabled-pen-color")
                                    .outline()
                                    .label("选择颜色")
                                    .disabled(true)
                                    .into_any_element()
                            } else {
                                ColorPicker::new(&self.pen_color)
                                    .label("选择颜色")
                                    .into_any_element()
                            })
                            .child(div().flex_shrink_0().child("无效颜色"))
                            .child(if disabled {
                                Button::new("disabled-pen-invalid-color")
                                    .outline()
                                    .label("选择颜色")
                                    .disabled(true)
                                    .into_any_element()
                            } else {
                                ColorPicker::new(&self.pen_invalid_color)
                                    .label("选择颜色")
                                    .into_any_element()
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(div().w(px(70.)).flex_shrink_0().child("线宽"))
                            .child(Slider::new(&self.pen_width).flex_1().disabled(disabled))
                            .child(
                                div()
                                    .w(px(48.))
                                    .text_xs()
                                    .text_color(rgb(self.palette.muted))
                                    .child(format!("{width:.0} px")),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(div().w(px(70.)).flex_shrink_0().child("不透明度"))
                            .child(Slider::new(&self.pen_opacity).flex_1().disabled(disabled))
                            .child(
                                div()
                                    .w(px(48.))
                                    .text_xs()
                                    .text_color(rgb(self.palette.muted))
                                    .child(format!("{opacity:.0}%")),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .w(px(180.))
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(self.palette.bg))
                    .border_1()
                    .border_color(rgb(self.palette.border))
                    .child(self.pen_preview(cx))
                    .child(self.muted("实时预览")),
            );
        div()
            .id("general-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_6()
            .pb_5()
            .child(
                self.section("操作方式").child(
                    h_flex()
                        .justify_between()
                        .gap_5()
                        .child(
                            v_flex()
                                .gap_1()
                                .child("手势触发键")
                                .child(self.muted("按住绘制，松开执行")),
                        )
                        .child(
                            Select::new(&self.trigger)
                                .cleanable(false)
                                .w(px(200.))
                                .disabled(disabled),
                        ),
                ),
            )
            .child(
                self.section("外观").child(
                    h_flex()
                        .justify_between()
                        .gap_5()
                        .child("界面主题")
                        .child(themes),
                ),
            )
            .child(self.section("手势轨迹").child(trail))
            .child(
                self.section("日志").child(
                    h_flex()
                        .justify_between()
                        .gap_5()
                        .child(v_flex().gap_1().child("日志级别").child(
                            self.muted("默认 info；debug 记录执行的动作，日志保存到配置目录。"),
                        ))
                        .child(
                            Select::new(&self.log_level)
                                .cleanable(false)
                                .w(px(200.))
                                .disabled(disabled),
                        ),
                ),
            )
            .child(
                v_flex()
                    .gap_3()
                    .pt_3()
                    .child(
                        Button::new("advanced-settings")
                            .disabled(self.modal_open())
                            .outline()
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
                    )
                    .when(self.preference_tools, |el| {
                        el.child(self.muted("识别参数和过滤规则可在 config.json 中调整。"))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("open-config")
                                            .outline()
                                            .disabled(self.modal_open())
                                            .small()
                                            .label("打开配置目录")
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.open_config(cx)),
                                            ),
                                    )
                                    .child(
                                        Button::new("reload-config")
                                            .outline()
                                            .small()
                                            .label("重新加载配置")
                                            .disabled(disabled)
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.reload(cx)),
                                            ),
                                    ),
                            )
                    })
                    .child(
                        h_flex().child(
                            Button::new("open-log")
                                .outline()
                                .disabled(self.modal_open())
                                .small()
                                .label("打开日志")
                                .on_click(cx.listener(|this, _, _, cx| this.open_log(cx))),
                        ),
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
            list = list.child(div().p_4().child(self.muted(
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
                                    .w(px(58.))
                                    .h(px(48.))
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
                                            .child(div().truncate().child(row.action.name.clone()))
                                            .when(row.inherited, |el| {
                                                el.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(self.palette.accent))
                                                        .child("继承全局"),
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
            .w(relative(0.44))
            .min_w(px(210.))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .child(
                h_flex()
                    .gap_2()
                    .p_4()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(self.palette.border))
                    .child(
                        Input::new(&self.inputs["search"])
                            .disabled(self.modal_open())
                            .flex_1()
                            .min_w_0(),
                    )
                    .child(
                        Button::new("new-gesture")
                            .outline()
                            .small()
                            .label("＋ 新建")
                            .disabled(disabled)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_action(window, cx)),
                            ),
                    ),
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
            .child(label)
            .child(Input::new(&self.inputs[key]).disabled(disabled))
            .into_any_element()
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut editor = v_flex().gap_5().p_4();
        let Some(selected) = &self.selected else {
            return div()
                .id("gesture-editor-scroll")
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .child(editor.child(self.muted("选择一个手势查看和编辑，或新建手势。")))
                .into_any_element();
        };
        let editable = self.is_editable() && !self.modal_open();
        editor = editor.child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(div().text_lg().font_weight(FontWeight::MEDIUM).child(
                    if selected.inherited {
                        "全局手势"
                    } else {
                        "编辑手势"
                    },
                ))
                .when(!selected.inherited, |el| {
                    el.child(
                        Button::new("delete-gesture")
                            .outline()
                            .small()
                            .label(
                                if self.current_scope() != Some("global")
                                    && self.config.as_ref().is_some_and(|config| {
                                        config.packages.iter().any(|package| {
                                            package.id == "global"
                                                && package.actions.iter().any(|action| {
                                                    action.gesture == selected.action.gesture
                                                })
                                        })
                                    })
                                {
                                    "恢复全局"
                                } else {
                                    "删除绑定"
                                },
                            )
                            .disabled(!editable)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.remove_action(window, cx)),
                            ),
                    )
                }),
        );
        if selected.inherited {
            editor =
                editor.child(
                    v_flex()
                        .gap_3()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(self.palette.bg))
                        .child(self.muted("此手势继承全局配置。可为当前应用创建独立动作。"))
                        .child(
                            Button::new("customize-gesture")
                                .outline()
                                .small()
                                .label("为此应用自定义")
                                .disabled(
                                    self.controls_blocked()
                                        || self.application().is_some_and(|app| app.disabled),
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.customize_action(window, cx)
                                })),
                        ),
                );
        }
        editor =
            editor
                .child(self.editable_field("action_name", "名称", !editable))
                .child(
                    v_flex()
                        .gap_2()
                        .child("手势")
                        .child(
                            h_flex()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(70.))
                                        .h(px(52.))
                                        .flex_shrink_0()
                                        .rounded_md()
                                        .bg(rgb(self.palette.bg))
                                        .border_1()
                                        .border_color(rgb(self.palette.border))
                                        .overflow_hidden()
                                        .child(self.gesture_preview(&self.editor_gesture)),
                                )
                                .child(
                                    Button::new("record-gesture")
                                        .outline()
                                        .small()
                                        .label("更换手势")
                                        .disabled(!editable)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.begin_recording(cx)),
                                        ),
                                ),
                        )
                        .when(
                            !self.config.as_ref().is_some_and(|config| {
                                config.packages.iter().any(|package| {
                                    package.id == selected.source_package
                                        && package
                                            .actions
                                            .iter()
                                            .any(|action| action.id == selected.action.id)
                                })
                            }),
                            |el| {
                                el.child(
                                    Select::new(&self.template)
                                        .cleanable(false)
                                        .w_full()
                                        .disabled(!editable),
                                )
                            },
                        ),
                )
                .child(v_flex().gap_2().child("附加输入").child(
                    Select::new(&self.extra).cleanable(false).w_full().disabled(
                        !editable
                            || glint_core::SPECIAL_GESTURES.contains(
                                &self.editor_gesture.split('+').next().unwrap_or_default(),
                            ),
                    ),
                ))
                .child(div().h(px(1.)).bg(rgb(self.palette.border)))
                .child(
                    v_flex().gap_2().child("动作类型").child(
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
                        .child("快捷键")
                        .child(
                            Input::new(&self.inputs["action_value"])
                                .disabled(!editable || self.recording_shortcut),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("record-shortcut")
                                        .outline()
                                        .small()
                                        .label(if self.recording_shortcut {
                                            "请按下快捷键…"
                                        } else {
                                            "录入快捷键"
                                        })
                                        .disabled(!editable || self.recording_shortcut)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.begin_shortcut(cx)),
                                        ),
                                )
                                .when(self.recording_shortcut, |el| {
                                    el.child(self.muted("Esc 取消"))
                                }),
                        ),
                );
            }
            "window" => {
                editor = editor.child(
                    v_flex().gap_2().child("窗口操作").child(
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
                            .child("程序路径")
                            .child(Input::new(&self.inputs["action_value"]).disabled(!editable))
                            .child(
                                Button::new("pick-action-program")
                                    .outline()
                                    .small()
                                    .label("选择程序…")
                                    .disabled(!editable)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pick_program(false, window, cx)
                                    })),
                            ),
                    )
                    .child(self.editable_field("action_args", "启动参数（可选）", !editable));
            }
            _ => {}
        }
        div()
            .id("gesture-editor-scroll")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .child(editor)
            .into_any_element()
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let changed = self.current_dirty();
        let status = if self.pending {
            "正在应用…"
        } else if changed {
            "有未应用的修改"
        } else {
            "设置已保存"
        };
        h_flex()
            .gap_3()
            .justify_between()
            .px_6()
            .py_3()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(self.palette.border))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(self.muted(if self.connected {
                        status
                    } else {
                        "后台未连接"
                    }))
                    .when(!self.notice.is_empty(), |el| {
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
                h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .when(self.application().is_some(), |el| {
                        el.child(
                            Button::new("manage-application")
                                .outline()
                                .small()
                                .label("管理应用")
                                .disabled(self.controls_blocked())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_app_dialog(false, window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("apply-settings")
                            .small()
                            .primary()
                            .label("应用设置")
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
                        .p_6()
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
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.capture_application(cx)
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
                        .gap_4()
                        .p_6()
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
            .px_5()
            .bg(rgba(0x00000066))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if matches!(self.page, Page::General) {
            self.render_general(cx)
        } else {
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
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
                            .child(self.render_heading(cx))
                            .child(content)
                            .child(self.render_footer(cx)),
                    ),
            )
            .children(self.render_app_modal(cx))
            .children(self.render_recording_modal(cx))
            .children(Root::render_dialog_layer(window, cx))
    }
}
