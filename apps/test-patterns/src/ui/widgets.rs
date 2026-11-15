//! Side panel, overlays, pattern grid, and footer controls.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    ClickEvent, Context, FontWeight, InteractiveElement, MouseButton, MouseDownEvent, ObjectFit,
    RenderImage, SharedString, div, img, prelude::*, px, rgb,
};
use omt_media::SendStats;
use openmediatransport::Quality;
use pattern_generator::PatternKind;
use suite_core::{Language, SimdCapabilities, t};

use super::PatternsView;
use super::presets::*;

pub(crate) fn section_header<F>(
    cx: &mut Context<PatternsView>,
    id: &'static str,
    title: &str,
    open: bool,
    on_toggle: F,
) -> impl IntoElement
where
    F: Fn(&mut PatternsView, &mut Context<PatternsView>) + 'static + Clone,
{
    let title = SharedString::from(title.to_string());
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| on_toggle(this, cx)))
        .child(
            div()
                .w(px(12.0))
                .text_xs()
                .opacity(0.75)
                .child(if open { "▾" } else { "▸" }),
        )
        .child(div().font_weight(FontWeight::BOLD).child(title))
}

pub(crate) fn stats_block(
    language: Language,
    stats: &SendStats,
    error: Option<SharedString>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(stat_row(
            t(language, "patterns.clients"),
            format!("{}", stats.clients),
        ))
        .child(stat_row(
            t(language, "patterns.connections"),
            format!("{}", stats.connections),
        ))
        .child(stat_row(
            t(language, "patterns.video_subs"),
            format!("{}", stats.video_subscribers),
        ))
        .child(stat_row(
            t(language, "patterns.audio_subs"),
            format!("{}", stats.audio_subscribers),
        ))
        .child(stat_row("Audio TX", format!("{}", stats.audio_frames)))
        .child(stat_row(
            "Audio gap",
            format!("{:.1} ms", stats.audio_max_gap_ms),
        ))
        .child(stat_row("Audio skips", format!("{}", stats.audio_skips)))
        .child(stat_row("Port", format!("{}", stats.port)))
        .child(stat_row("Video FPS", format!("{:.1}", stats.video_fps)))
        .child(stat_row("Encode", format!("{:.1} ms", stats.encode_ms)))
        .child(stat_row("Frames", format!("{}", stats.frames)))
        .child(stat_row("Dropped", format!("{}", stats.dropped)))
        .child(stat_row("Bytes TX", format_bytes(stats.bytes_sent)))
        .child(stat_row(
            t(language, "simd"),
            SimdCapabilities::detect().summary(),
        ))
        .child(
            div()
                .mt_1()
                .text_xs()
                .text_color(rgb(0xf6c344))
                .opacity(if stats.behind { 1.0 } else { 0.0 })
                .child(t(language, "patterns.perf_warn")),
        )
        .children(error.map(|e| div().text_xs().text_color(rgb(0xff6b6b)).child(e)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_block(
    cx: &mut Context<PatternsView>,
    language: Language,
    name: &str,
    name_editing: bool,
    sending: bool,
    animate: bool,
    speed_h: i32,
    speed_v: i32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(name_field(cx, language, name, name_editing, sending))
        .children(sending.then(|| {
            div()
                .text_xs()
                .text_color(rgb(0xf6c344))
                .child(t(language, "patterns.restart_required"))
        }))
        .child(toggle_row(
            cx,
            "animate-toggle",
            t(language, "patterns.animate"),
            animate,
            |this, cx| this.toggle_animate(cx),
        ))
        .child(stepper_row(
            cx,
            "speed-h",
            t(language, "patterns.anim_speed_h"),
            format!("{speed_h}%"),
            |this, cx| this.nudge_anim_speed_h(-10, cx),
            |this, cx| this.nudge_anim_speed_h(10, cx),
        ))
        .child(stepper_row(
            cx,
            "speed-v",
            t(language, "patterns.anim_speed_v"),
            format!("{speed_v}%"),
            |this, cx| this.nudge_anim_speed_v(-10, cx),
            |this, cx| this.nudge_anim_speed_v(10, cx),
        ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn overlay_layer(
    cx: &mut Context<PatternsView>,
    language: Language,
    open_menu: Option<(MenuKind, f32)>,
    tone_hz: f32,
    frame_rate: FrameRate,
    level_dbfs: f32,
    width: i32,
    height: i32,
    custom_menu: Option<(usize, f32, f32)>,
) -> impl IntoElement {
    let mut layer = div()
        .id("overlay-root")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_overlays(cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, _, _, cx| {
                this.close_overlays(cx);
            }),
        );

    if let Some((menu, anchor_x)) = open_menu {
        let menu_width = match menu {
            MenuKind::Resolution => 168.0,
            MenuKind::Tone => 160.0,
            MenuKind::Fps => 100.0,
            MenuKind::Level => 120.0,
        };
        // Keep the menu near the control that opened it, even when the footer wraps.
        let left = (anchor_x - 8.0).max(8.0);
        let menu_div = div()
            .absolute()
            .w(px(menu_width))
            .bottom(px(52.0))
            .left(px(left))
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(0x2a3340))
            .bg(rgb(0x1b222c))
            .text_color(rgb(0xedf2f7))
            .shadow_md()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        layer = layer.child(
            menu_div.children(match menu {
                MenuKind::Tone => TonePreset::PRESETS
                    .iter()
                    .map(|preset| {
                        let preset = *preset;
                        let active = preset.matches(tone_hz);
                        dropdown_item(
                            cx,
                            SharedString::from(format!("tone-{}", preset.hz())),
                            preset.label(language),
                            active,
                            move |this, cx| this.set_tone(preset, cx),
                        )
                    })
                    .collect::<Vec<_>>(),
                MenuKind::Fps => FrameRate::PRESETS
                    .iter()
                    .map(|preset| {
                        let preset = *preset;
                        let active = frame_rate == preset;
                        dropdown_item(
                            cx,
                            SharedString::from(format!("fps-{}-{}", preset.n, preset.d)),
                            SharedString::from(preset.label()),
                            active,
                            move |this, cx| this.set_frame_rate(preset, cx),
                        )
                    })
                    .collect::<Vec<_>>(),
                MenuKind::Level => LevelPreset::PRESETS
                    .iter()
                    .map(|preset| {
                        let preset = *preset;
                        let active = preset.matches(level_dbfs);
                        dropdown_item(
                            cx,
                            SharedString::from(format!("level-{}", preset.dbfs() as i32)),
                            preset.label(),
                            active,
                            move |this, cx| this.set_level(preset, cx),
                        )
                    })
                    .collect::<Vec<_>>(),
                MenuKind::Resolution => {
                    let mut items: Vec<gpui::AnyElement> = Resolution::PRESETS
                        .iter()
                        .map(|preset| {
                            let preset = *preset;
                            let active = width == preset.width && height == preset.height;
                            dropdown_item(
                                cx,
                                SharedString::from(format!(
                                    "res-{}-{}",
                                    preset.width, preset.height
                                )),
                                SharedString::from(preset.label()),
                                active,
                                move |this, cx| this.set_resolution(preset, cx),
                            )
                        })
                        .collect();
                    items.push(
                        div()
                            .mt_1()
                            .pt_1()
                            .border_t_1()
                            .border_color(rgb(0x2a3340))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .px_1()
                                    .text_xs()
                                    .text_color(rgb(0xa0aec0))
                                    .child(format!("W {width}")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(step_btn(cx, "res-w-dec", "−", |this, cx| {
                                        this.nudge_width(-2, cx);
                                    }))
                                    .child(step_btn(cx, "res-w-inc", "+", |this, cx| {
                                        this.nudge_width(2, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .px_1()
                                    .text_xs()
                                    .text_color(rgb(0xa0aec0))
                                    .child(format!("H {height}")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(step_btn(cx, "res-h-dec", "−", |this, cx| {
                                        this.nudge_height(-2, cx);
                                    }))
                                    .child(step_btn(cx, "res-h-inc", "+", |this, cx| {
                                        this.nudge_height(2, cx);
                                    })),
                            )
                            .into_any_element(),
                    );
                    items
                }
            }),
        );
    }

    if let Some((index, x, y)) = custom_menu {
        layer = layer.child(
            div()
                .absolute()
                .top(px(y))
                .left(px(x))
                .min_w(px(168.0))
                .p_1()
                .rounded_md()
                .border_1()
                .border_color(rgb(0x2a3340))
                .bg(rgb(0x1b222c))
                .text_color(rgb(0xedf2f7))
                .shadow_md()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(dropdown_item(
                    cx,
                    SharedString::from("custom-image-reveal"),
                    SharedString::from(t(language, "patterns.image_reveal")),
                    false,
                    move |this, cx| this.reveal_custom_image(index, cx),
                ))
                .child(dropdown_item(
                    cx,
                    SharedString::from("custom-image-remove"),
                    SharedString::from(t(language, "patterns.image_remove")),
                    false,
                    move |this, cx| this.remove_custom_image(index, cx),
                )),
        );
    }

    layer
}

pub(crate) fn dropdown_item<F>(
    cx: &mut Context<PatternsView>,
    id: SharedString,
    label: SharedString,
    active: bool,
    on_click: F,
) -> gpui::AnyElement
where
    F: Fn(&mut PatternsView, &mut Context<PatternsView>) + 'static + Clone,
{
    let handler = on_click.clone();
    let hover_bg = if active { rgb(0x3d7cff) } else { rgb(0x2f3b4d) };
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_sm()
        .bg(if active { rgb(0x2f6fed) } else { rgb(0x1b222c) })
        .text_color(rgb(0xedf2f7))
        .hover(move |s| s.bg(hover_bg).text_color(rgb(0xedf2f7)).opacity(1.0))
        .cursor_pointer()
        .text_xs()
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| {
            handler(this, cx);
        }))
        .into_any_element()
}

pub(crate) fn pattern_grid(
    cx: &mut Context<PatternsView>,
    language: Language,
    selected: PatternKind,
    selected_custom: Option<usize>,
    thumbs: Vec<(PatternKind, Option<Arc<RenderImage>>)>,
    custom_images: Vec<(usize, PathBuf, Option<Arc<RenderImage>>)>,
) -> impl IntoElement {
    let pattern_tiles: Vec<_> = thumbs
        .into_iter()
        .map(|(kind, thumb)| {
            let is_selected = selected != PatternKind::Image && kind == selected;
            pattern_tile(
                cx,
                SharedString::from(kind.id()),
                SharedString::from(pattern_label(language, kind)),
                thumb,
                is_selected,
                move |this, cx| this.select_pattern(kind, cx),
                None::<fn(&mut PatternsView, &MouseDownEvent, &mut Context<PatternsView>)>,
            )
        })
        .collect();

    let mut custom_cells: Vec<gpui::AnyElement> = custom_images
        .into_iter()
        .map(|(index, path, thumb)| {
            let is_selected = selected == PatternKind::Image && selected_custom == Some(index);
            pattern_tile(
                cx,
                SharedString::from(format!("custom-image-{index}")),
                image_display_name(&path),
                thumb,
                is_selected,
                move |this, cx| this.select_custom_image(index, cx),
                Some(
                    move |this: &mut PatternsView,
                          event: &MouseDownEvent,
                          cx: &mut Context<PatternsView>| {
                        let x: f32 = event.position.x.into();
                        let y: f32 = event.position.y.into();
                        this.open_custom_menu(index, x, y, cx);
                    },
                ),
            )
        })
        .collect();

    custom_cells.push(add_image_tile(cx, language));

    div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_4()
                .children(pattern_tiles),
        )
        .child(div().mt_2().mb_3().h(px(1.0)).bg(rgb(0x2a3340)))
        .child(
            div()
                .mb_3()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .opacity(0.7)
                .child(t(language, "patterns.image")),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_4()
                .children(custom_cells),
        )
}

pub(crate) fn pattern_tile<FSelect, FMenu>(
    cx: &mut Context<PatternsView>,
    id: SharedString,
    label: SharedString,
    thumb: Option<Arc<RenderImage>>,
    is_selected: bool,
    on_select: FSelect,
    on_menu: Option<FMenu>,
) -> gpui::AnyElement
where
    FSelect: Fn(&mut PatternsView, &mut Context<PatternsView>) + 'static + Clone,
    FMenu: Fn(&mut PatternsView, &MouseDownEvent, &mut Context<PatternsView>) + 'static + Clone,
{
    let select = on_select.clone();
    let mut tile = div()
        .id(id)
        .w(px(TILE_W))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_1()
        .cursor_pointer()
        .child(
            div()
                .rounded_sm()
                .border_2()
                .border_color(if is_selected {
                    rgb(0x2f6fed)
                } else {
                    rgb(0x2a3340)
                })
                .bg(rgb(0x0c1016))
                .overflow_hidden()
                .child(if let Some(tex) = thumb {
                    img(tex)
                        .object_fit(ObjectFit::Fill)
                        .w(px(216.0))
                        .h(px(122.0))
                        .into_any_element()
                } else {
                    div()
                        .w(px(216.0))
                        .h(px(122.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .opacity(0.5)
                        .child("?")
                        .into_any_element()
                }),
        )
        .child(
            div()
                .text_sm()
                .text_center()
                .w_full()
                .truncate()
                .child(label),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            select(this, cx);
        }));

    if let Some(on_menu) = on_menu {
        tile = tile.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                on_menu(this, event, cx);
                cx.stop_propagation();
            }),
        );
    }

    tile.into_any_element()
}

pub(crate) fn add_image_tile(
    cx: &mut Context<PatternsView>,
    language: Language,
) -> gpui::AnyElement {
    div()
        .id("add-custom-image")
        .w(px(TILE_W))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_1()
        .cursor_pointer()
        .child(
            div()
                .rounded_sm()
                .border_2()
                .border_color(rgb(0x2a3340))
                .border_dashed()
                .bg(rgb(0x0c1016))
                .w(px(216.0))
                .h(px(122.0))
                .flex()
                .items_center()
                .justify_center()
                .text_2xl()
                .opacity(0.7)
                .child("+"),
        )
        .child(
            div()
                .text_sm()
                .text_center()
                .opacity(0.7)
                .child(t(language, "patterns.image_add")),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.request_pick_image(cx);
            cx.notify();
        }))
        .into_any_element()
}

pub(crate) fn tone_control(
    cx: &mut Context<PatternsView>,
    language: Language,
    tone_hz: f32,
    open: bool,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_shrink_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .child(t(language, "patterns.tone")),
        )
        .child(
            div()
                .id("tone-toggle")
                .w(px(140.0))
                .px_2()
                .py_1()
                .rounded_md()
                .bg(if open { rgb(0x2f6fed) } else { rgb(0x243041) })
                .cursor_pointer()
                .text_xs()
                .child(tone_label(language, tone_hz))
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    let x: f32 = event.position().x.into();
                    this.toggle_menu(MenuKind::Tone, x, cx);
                })),
        )
}

pub(crate) fn transport_controls(
    cx: &mut Context<PatternsView>,
    language: Language,
    sending: bool,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_shrink_0()
        .child(div().text_xs().opacity(0.65).child(" "))
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    div()
                        .id("patterns-start")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(if sending {
                            rgb(0x1a3a2a)
                        } else {
                            rgb(0x2f6fed)
                        })
                        .opacity(if sending { 0.55 } else { 1.0 })
                        .cursor_pointer()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t(language, "patterns.start"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !sending {
                                this.start_sending(cx);
                            }
                        })),
                )
                .child(
                    div()
                        .id("patterns-stop")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(if sending {
                            rgb(0xb33a3a)
                        } else {
                            rgb(0x243041)
                        })
                        .opacity(if sending { 1.0 } else { 0.55 })
                        .cursor_pointer()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t(language, "patterns.stop"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if sending {
                                this.stop_sending(cx);
                            }
                        })),
                ),
        )
}

pub(crate) fn name_field(
    cx: &mut Context<PatternsView>,
    language: Language,
    name: &str,
    editing: bool,
    locked: bool,
) -> impl IntoElement {
    let display = if editing {
        format!("{name}|")
    } else {
        name.to_string()
    };
    // Avoid `.truncate()`: without a definite width it replaces the name with an ellipsis.
    div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .min_w_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .child(t(language, "patterns.name")),
        )
        .child(
            div()
                .id("source-name")
                .w_full()
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(if editing {
                    rgb(0x2f6fed)
                } else {
                    rgb(0x2a3340)
                })
                .bg(rgb(0x0c1016))
                .opacity(if locked { 0.55 } else { 1.0 })
                .cursor_text()
                .text_xs()
                .child(display)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !locked {
                        this.begin_edit_name(window, cx);
                    }
                })),
        )
}

pub(crate) fn resolution_control(
    cx: &mut Context<PatternsView>,
    language: Language,
    width: i32,
    height: i32,
    open: bool,
    locked: bool,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_shrink_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .child(t(language, "patterns.resolution")),
        )
        .child(
            div()
                .id("resolution-toggle")
                .w(px(110.0))
                .px_2()
                .py_1()
                .rounded_md()
                .bg(if open { rgb(0x2f6fed) } else { rgb(0x243041) })
                .opacity(if locked { 0.55 } else { 1.0 })
                .cursor_pointer()
                .text_xs()
                .child(format!("{width}×{height}"))
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if !locked {
                        let x: f32 = event.position().x.into();
                        this.toggle_menu(MenuKind::Resolution, x, cx);
                    }
                })),
        )
}

pub(crate) fn level_control(
    cx: &mut Context<PatternsView>,
    language: Language,
    level_dbfs: f32,
    open: bool,
) -> impl IntoElement {
    let display = LevelPreset::nearest(level_dbfs).label();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_shrink_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .child(t(language, "patterns.tone_level")),
        )
        .child(
            div()
                .id("level-toggle")
                .w(px(100.0))
                .px_2()
                .py_1()
                .rounded_md()
                .bg(if open { rgb(0x2f6fed) } else { rgb(0x243041) })
                .cursor_pointer()
                .text_xs()
                .child(display)
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    let x: f32 = event.position().x.into();
                    this.toggle_menu(MenuKind::Level, x, cx);
                })),
        )
}

pub(crate) fn toggle_row<F>(
    cx: &mut Context<PatternsView>,
    id: &'static str,
    label: &str,
    active: bool,
    on_toggle: F,
) -> impl IntoElement
where
    F: Fn(&mut PatternsView, &mut Context<PatternsView>) + 'static + Clone,
{
    let toggle = on_toggle.clone();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .min_w_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .min_w_0()
                .flex_1()
                .truncate()
                .child(label.to_string()),
        )
        .child(
            div()
                .id(SharedString::from(id))
                .px_2()
                .py_1()
                .rounded_md()
                .bg(if active { rgb(0x2f6fed) } else { rgb(0x243041) })
                .cursor_pointer()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0()
                .child(if active { "ON" } else { "OFF" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    toggle(this, cx);
                })),
        )
}

pub(crate) fn stepper_row<FDec, FInc>(
    cx: &mut Context<PatternsView>,
    id: &'static str,
    label: &str,
    value: String,
    on_dec: FDec,
    on_inc: FInc,
) -> impl IntoElement
where
    FDec: Fn(&mut PatternsView, &mut Context<PatternsView>) + 'static + Clone,
    FInc: Fn(&mut PatternsView, &mut Context<PatternsView>) + 'static + Clone,
{
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .min_w_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .min_w_0()
                .flex_1()
                .truncate()
                .child(label.to_string()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .flex_shrink_0()
                .child(step_btn(
                    cx,
                    SharedString::from(format!("{id}-dec")),
                    "−",
                    on_dec,
                ))
                .child(
                    div()
                        .min_w(px(64.0))
                        .text_xs()
                        .text_center()
                        .font_weight(FontWeight::MEDIUM)
                        .child(value),
                )
                .child(step_btn(
                    cx,
                    SharedString::from(format!("{id}-inc")),
                    "+",
                    on_inc,
                )),
        )
}

pub(crate) fn step_btn<F>(
    cx: &mut Context<PatternsView>,
    id: impl Into<SharedString>,
    label: &'static str,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&mut PatternsView, &mut Context<PatternsView>) + 'static + Clone,
{
    let handler = on_click.clone();
    div()
        .id(id.into())
        .w(px(22.0))
        .h(px(22.0))
        .rounded_sm()
        .bg(rgb(0x243041))
        .hover(|s| s.bg(rgb(0x2f3b4d)))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| {
            handler(this, cx);
        }))
}

pub(crate) fn frame_buffer_control(
    cx: &mut Context<PatternsView>,
    language: Language,
    frames: u32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_shrink_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .child(t(language, "patterns.frame_buffer")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .h(px(24.0))
                .child(step_btn(cx, "frame-buffer-dec", "−", |this, cx| {
                    this.nudge_frame_buffer(-1, cx);
                }))
                .child(
                    div()
                        .w(px(24.0))
                        .text_xs()
                        .text_center()
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("{frames}")),
                )
                .child(step_btn(cx, "frame-buffer-inc", "+", |this, cx| {
                    this.nudge_frame_buffer(1, cx);
                })),
        )
}

pub(crate) fn fps_control(
    cx: &mut Context<PatternsView>,
    language: Language,
    selected: FrameRate,
    open: bool,
    locked: bool,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_shrink_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .child(t(language, "patterns.fps")),
        )
        .child(
            div()
                .id("fps-toggle")
                .w(px(88.0))
                .px_2()
                .py_1()
                .rounded_md()
                .bg(if open { rgb(0x2f6fed) } else { rgb(0x243041) })
                .opacity(if locked { 0.55 } else { 1.0 })
                .cursor_pointer()
                .text_xs()
                .child(selected.label())
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if !locked {
                        let x: f32 = event.position().x.into();
                        this.toggle_menu(MenuKind::Fps, x, cx);
                    }
                })),
        )
}

pub(crate) fn quality_control(
    cx: &mut Context<PatternsView>,
    language: Language,
    selected: Quality,
) -> impl IntoElement {
    let qualities = [Quality::Low, Quality::Medium, Quality::High];
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_shrink_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .child(t(language, "patterns.profile")),
        )
        .child(
            div()
                .flex()
                .rounded_md()
                .overflow_hidden()
                .children(qualities.into_iter().map(|quality| {
                    let active = selected == quality;
                    div()
                        .id(SharedString::from(quality_label(quality)))
                        .px_3()
                        .py_1()
                        .bg(if active { rgb(0x2f6fed) } else { rgb(0x243041) })
                        .cursor_pointer()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(quality_label(quality))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_quality(quality, cx);
                        }))
                        .into_any_element()
                })),
        )
}

pub(crate) fn output_preview(
    language: Language,
    preview: Option<Arc<RenderImage>>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap_1()
        .flex_shrink_0()
        .child(
            div()
                .text_xs()
                .opacity(0.65)
                .child(t(language, "patterns.output")),
        )
        .child(
            div()
                .w(px(128.0))
                .h(px(72.0))
                .rounded_sm()
                .border_1()
                .border_color(rgb(0x2a3340))
                .bg(rgb(0x000000))
                .overflow_hidden()
                .child(if let Some(tex) = preview {
                    img(tex)
                        .object_fit(ObjectFit::Fill)
                        .w(px(128.0))
                        .h(px(72.0))
                        .into_any_element()
                } else {
                    div().into_any_element()
                }),
        )
}

pub(crate) fn stat_row(label: &str, value: String) -> impl IntoElement {
    div()
        .flex()
        .justify_between()
        .gap_3()
        .text_xs()
        .child(div().opacity(0.65).flex_shrink_0().child(label.to_string()))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .min_w_0()
                .flex_1()
                .text_right()
                .child(value),
        )
}
