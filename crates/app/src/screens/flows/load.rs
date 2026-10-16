//! A flow's load test, drawn in place of the canvas: the counts, how long an instance took, each node's
//! times, and a line for every instance, green or red by how it ended.
//!
//! Everything shown was counted by the engine (`flow.loadtest.*`); this draws what it was told.

use std::ops::Range;

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, progress::Progress, tag::Tag, v_flex,
};
use gpui_kit::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, uniform_list,
};
use serde_json::Value;

use super::FlowsScreen;
use crate::{i18n::t, model::flow::LoadView};

/// A number the engine sent, or a dash while it has none.
fn figure(from: &Value, key: &str) -> String {
    from.get(key).and_then(Value::as_u64).map_or_else(|| "\u{2013}".to_owned(), |number| number.to_string())
}

fn text<'a>(from: &'a Value, key: &str) -> &'a str {
    from.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// One figure of the load test, large.
fn stat(title: &str, value: String, color: Hsla, bar: Option<f32>, cx: &Context<FlowsScreen>) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .px_4()
        .py_3()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .child(div().text_xs().text_color(theme.muted_foreground).child(title.to_uppercase()))
        .child(div().text_2xl().font_semibold().text_color(color).child(value))
        .children(bar.map(|percent| Progress::new(title.to_owned()).value(percent).color(color)))
        .into_any_element()
}

/// How an instance ended, as a tag and the color of its line.
fn outcome(row: &Value, cx: &Context<FlowsScreen>) -> (Tag, Hsla) {
    let theme = cx.theme();
    match text(row, "status") {
        "running" | "" => (Tag::secondary().child(t("flows.load.row.running")), theme.muted_foreground),
        "succeeded" => (Tag::success().child(t("flows.load.row.passed")), theme.success),
        "cancelled" => (Tag::warning().child(t("flows.load.row.cancelled")), theme.warning),
        _ => (Tag::danger().child(t("flows.load.row.failed")), theme.danger),
    }
}

/// Each node across the instances: how many reached it, how many it failed in, and how long it took.
fn nodes(load: &LoadView, cx: &Context<FlowsScreen>) -> AnyElement {
    let theme = cx.theme().clone();
    let line = |cells: [String; 5], color: Hsla, failed: Hsla| {
        let [name, runs, failed_in, avg, max] = cells;
        let cell = |text: String, color: Hsla| div().w_20().flex_none().text_right().text_color(color).child(text);
        h_flex()
            .w_full()
            .gap_3()
            .px_4()
            .py_1()
            .text_xs()
            .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().text_color(color).child(name))
            .child(cell(runs, color))
            .child(cell(failed_in, failed))
            .child(cell(avg, color))
            .child(cell(max, color))
    };
    let heading = [
        t("flows.load.nodes.node"),
        t("flows.load.nodes.runs"),
        t("flows.load.nodes.failed"),
        t("flows.load.nodes.avg"),
        t("flows.load.nodes.max"),
    ]
    .map(str::to_owned);
    v_flex()
        .id("load-nodes")
        .flex_none()
        .max_h_40()
        .overflow_y_scroll()
        .py_1()
        .border_b_1()
        .border_color(theme.border)
        .child(line(heading, theme.muted_foreground, theme.muted_foreground))
        .children(load.nodes.iter().map(|node| {
            let failed = node.get("failed").and_then(Value::as_u64).unwrap_or(0);
            let cells = [
                text(node, "name").to_owned(),
                figure(node, "runs"),
                failed.to_string(),
                figure(node, "avgMs"),
                figure(node, "maxMs"),
            ];
            line(cells, theme.foreground, if failed > 0 { theme.danger } else { theme.muted_foreground })
        }))
        .into_any_element()
}

pub fn render(load: &LoadView, cx: &mut Context<FlowsScreen>) -> AnyElement {
    let theme = cx.theme().clone();
    let (all, running, failed) = (load.count("instances"), load.count("running"), load.count("failed"));
    let done = all - running.min(all);
    let per_second = load.summary.get("perSecond").and_then(Value::as_f64).unwrap_or(0.0);
    let times = ["min", "avg", "p50", "p95", "p99", "max"]
        .map(|name| format!("{name} {}", figure(&load.summary, &format!("{name}Ms"))))
        .join("  \u{00b7}  ");

    let rows = uniform_list(
        "load-rows",
        load.rows.len(),
        cx.processor(move |this, range: Range<usize>, _, cx| {
            let theme = cx.theme().clone();
            let Some(load) = this.load.as_ref() else { return Vec::new() };
            range
                .filter_map(|ix| Some((ix, load.rows.get(ix)?)))
                .map(|(ix, row)| {
                    let (tag, color) = outcome(row, cx);
                    let said = crate::engine::text_in(row, "error").unwrap_or_default();
                    // A row is as wide as its content unless told otherwise.
                    h_flex()
                        .w_full()
                        .gap_3()
                        .px_4()
                        .py_1p5()
                        .text_sm()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().w_1().h_4().flex_none().rounded_sm().bg(color))
                        .child(
                            div().w_12().flex_none().text_color(theme.muted_foreground).child(format!("#{}", ix + 1)),
                        )
                        .child(h_flex().w_24().flex_none().child(tag.small()))
                        .child(
                            div()
                                .w_48()
                                .flex_none()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(text(row, "at").to_owned()),
                        )
                        .child(
                            div().w_20().flex_none().text_right().text_color(theme.muted_foreground).child(
                                row.get("durationMs")
                                    .and_then(Value::as_u64)
                                    .map(|ms| format!("{ms} ms"))
                                    .unwrap_or_default(),
                            ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_color(theme.danger)
                                .child(said),
                        )
                        .into_any_element()
                })
                .collect()
        }),
    )
    .flex_1()
    .min_h_0();

    v_flex()
        .size_full()
        .child(
            h_flex()
                .flex_none()
                .items_stretch()
                .gap_3()
                .p_3()
                .child(stat(
                    t("flows.load.stat.done"),
                    format!("{done}/{all}"),
                    theme.foreground,
                    Some(if all == 0 { 0. } else { done as f32 * 100. / all as f32 }),
                    cx,
                ))
                .child(stat(t("flows.load.stat.passed"), load.count("passed").to_string(), theme.success, None, cx))
                .child(stat(
                    t("flows.load.stat.failed"),
                    failed.to_string(),
                    if failed > 0 { theme.danger } else { theme.muted_foreground },
                    None,
                    cx,
                ))
                .child(stat(t("flows.load.stat.rate"), format!("{per_second:.1}"), theme.foreground, None, cx)),
        )
        .child(
            h_flex()
                .flex_none()
                .gap_3()
                .px_4()
                .pb_2()
                .text_sm()
                .border_b_1()
                .border_color(theme.border)
                .child(div().text_color(theme.muted_foreground).child(t("flows.load.times")))
                .child(div().font_family(theme.mono_font_family.clone()).child(times)),
        )
        .children((!load.nodes.is_empty()).then(|| nodes(load, cx)))
        .child(rows)
        .into_any_element()
}
