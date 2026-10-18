//! The Faults tab of Proxy rules: the engine's fault rules (an agent's or the user's) with Clear, and the
//! Add fault dialog. The list is the model's live copy (`fault.list`, `event.faults.changed`).

use gpui_kit::component::{ActiveTheme as _, Sizable as _, button::Button, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    Window, div,
};
use serde_json::{Value, json};

use super::ProxyRulesScreen;
use crate::{
    app_model::ModelEvent,
    components::{
        dialogs::{PromptField, prompt},
        page,
    },
    engine::{call, failure},
    i18n::{t, t_with},
};

/// The `fault.add` payload for what the dialog holds (target, latency ms, status, rate %, break), checked the
/// way the engine checks it; `Err` is the message to show.
fn payload_of(values: &[String]) -> Result<Value, String> {
    let number = |text: &str, key: &'static str, min: u32, max: u32| -> Result<Option<u32>, String> {
        if text.is_empty() {
            return Ok(None);
        }
        match text.parse::<u32>() {
            Ok(n) if (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(t_with("faults.error.range", &[("field", &t(key)), ("min", &min), ("max", &max)])),
        }
    };
    let target = values[0].as_str();
    let host = target.split_once("://").map_or(target, |(_, rest)| rest);
    let host = host.split(['/', '?', '#']).next().unwrap_or("");
    let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
    let name = name.strip_prefix("*.").unwrap_or(name);
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-._[]:".contains(c)) {
        return Err(t("faults.error.target").to_owned());
    }
    let latency = number(&values[1], "faults.field.latency", 1, 120_000)?;
    let status = number(&values[2], "faults.field.status", 100, 599)?;
    let rate = number(&values[3], "faults.field.rate", 1, 100)?;
    let kind = values[4].to_ascii_lowercase();
    if !matches!(kind.as_str(), "" | "drop" | "reset") {
        return Err(t("faults.error.break").to_owned());
    }
    if kind.is_empty() && latency.is_none() && status.is_none() {
        return Err(t("faults.error.nothing").to_owned());
    }
    if !kind.is_empty() && (latency.is_some() || status.is_some()) {
        return Err(t("faults.error.alone").to_owned());
    }
    let mut payload = json!({ "target": target });
    for (key, value) in [("latencyMs", latency), ("status", status), ("rate", rate)] {
        if let Some(value) = value {
            payload[key] = json!(value);
        }
    }
    if !kind.is_empty() {
        payload[kind] = json!(true);
    }
    Ok(payload)
}

impl ProxyRulesScreen {
    /// `draft` is what a refused dialog held.
    pub(super) fn prompt_fault(&mut self, draft: Option<Vec<String>>, window: &mut Window, cx: &mut Context<Self>) {
        let initial = draft.unwrap_or_else(|| vec![String::new(); 5]);
        let fields = [
            PromptField::new(t("faults.field.target"), t("faults.field.target_placeholder")),
            PromptField::new(t("faults.field.latency"), t("faults.field.latency_placeholder")).optional(),
            PromptField::new(t("faults.field.status"), t("faults.field.status_placeholder")).optional(),
            PromptField::new(t("faults.field.rate"), t("faults.field.rate_placeholder")).optional(),
            PromptField::new(t("faults.field.break"), t("faults.field.break_placeholder")).optional(),
        ];
        let fields = fields.into_iter().zip(initial).map(|(field, value)| field.initial(value)).collect();
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("faults.add.title"),
            t("faults.add.confirm"),
            fields,
            |this, values, window, cx| match payload_of(&values) {
                Ok(payload) => {
                    let client = this.model.read(cx).client().clone();
                    call(&client, cx, "fault.add", payload, |this, outcome, cx| {
                        if let Some(message) = failure(&outcome) {
                            this.model.update(cx, |_, cx| {
                                cx.emit(ModelEvent::Notice(t_with("faults.error.add", &[("message", &message)])))
                            });
                        }
                    });
                }
                // The dialog has closed; it opens again with what was typed.
                Err(message) => {
                    this.model.update(cx, |_, cx| cx.emit(ModelEvent::Notice(message)));
                    this.prompt_fault(Some(values), window, cx);
                }
            },
        );
    }

    pub(super) fn render_faults(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let faults = self.model.read(cx).faults().to_vec();
        if faults.is_empty() {
            return page::empty_with_action(
                t("faults.empty.title"),
                t("faults.empty.description"),
                Button::new("empty-add-fault")
                    .label(t("faults.add.button"))
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_fault(None, window, cx))),
            );
        }
        let rows = faults.into_iter().map(|fault| {
            let id = fault.id.clone();
            h_flex()
                .gap_3()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex_1().min_w_0().text_sm().child(fault.description))
                .child(
                    Button::new(gpui_kit::ElementId::Name(format!("fault-clear-{id}").into()))
                        .small()
                        .outline()
                        .label(t("faults.clear"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.model.update(cx, |model, cx| model.clear_faults(Some(&id), cx))
                        })),
                )
        });
        v_flex()
            .size_full()
            .child(h_flex().flex_none().justify_end().px_4().py_2().child(
                Button::new("faults-clear-all").small().outline().label(t("faults.clear_all")).on_click(
                    cx.listener(|this, _, _, cx| this.model.update(cx, |model, cx| model.clear_faults(None, cx))),
                ),
            ))
            .child(div().id("fault-rows").flex_1().min_h_0().overflow_y_scroll().children(rows))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: [&str; 5]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_payload_follows_the_engines_rules() {
        assert_eq!(
            payload_of(&v(["api.test/pay", "", "503", "30", ""])).unwrap(),
            json!({"target": "api.test/pay", "status": 503, "rate": 30})
        );
        assert_eq!(
            payload_of(&v(["a.test", "", "", "", "Reset"])).unwrap(),
            json!({"target": "a.test", "reset": true})
        );
        for bad in [
            ["", "100", "", "", ""],
            ["a.test", "", "", "", ""],
            ["a.test", "0", "", "", ""],
            ["a.test", "", "99", "", ""],
            ["a.test", "", "503", "101", ""],
            ["a.test", "5", "", "", "drop"],
            ["a.test", "", "", "", "freeze"],
        ] {
            assert!(payload_of(&v(bad)).is_err(), "{bad:?}");
        }
    }
}
