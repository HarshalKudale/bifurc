//! The Services screen: what is listening on this machine, and which of it has a `.localhost` name.
//!
//! The engine finds the listeners (`services.discover`); this screen lists them beside the mapping that
//! points at each port, and offers to map the ones that have none.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::AppModel,
    components::{
        dialogs::{PromptField, prompt},
        page,
    },
    engine::{call, describe, failure},
};

#[derive(Debug, Clone, PartialEq)]
struct Service {
    port: u16,
    address: String,
    pid: Option<u64>,
    process: String,
}

impl Service {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            port: u16::try_from(value.get("port")?.as_u64()?).ok()?,
            address: value.get("address").and_then(Value::as_str).unwrap_or_default().to_owned(),
            pid: value.get("pid").and_then(Value::as_u64),
            // Older engines called the process `name`.
            process: value
                .get("processName")
                .or_else(|| value.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        })
    }
}

/// The port a mapping's target points at: `localhost:3000`, `http://127.0.0.1:3000/x`, `3000`.
fn target_port(target: &str) -> Option<u16> {
    let without_scheme = target.split("://").last().unwrap_or(target);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    authority.rsplit(':').next().and_then(|port| port.trim().parse().ok())
}

enum Listing {
    Loading,
    Loaded(Vec<Service>),
    Failed(String),
}

pub struct ServicesScreen {
    model: Entity<AppModel>,
    listing: Listing,
    refreshing: bool,
    filter: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ServicesScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by process or port"));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        let mut this =
            Self { model, listing: Listing::Loading, refreshing: false, filter, _subscriptions: subscriptions };
        this.refresh(cx);
        this
    }

    pub fn status_line(&self) -> String {
        match &self.listing {
            Listing::Loaded(services) => {
                format!("{} service{}", services.len(), if services.len() == 1 { "" } else { "s" })
            }
            _ => String::new(),
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "services.discover", json!({}), |this, outcome, cx| {
            this.refreshing = false;
            this.listing = match outcome {
                Ok(result) => {
                    let mut services: Vec<Service> = result
                        .as_array()
                        .map(|list| list.iter().filter_map(Service::from_value).collect())
                        .unwrap_or_default();
                    services.sort_by_key(|service| service.port);
                    services.dedup_by_key(|service| service.port);
                    Listing::Loaded(services)
                }
                Err(error) => Listing::Failed(describe(&error)),
            };
            cx.notify();
        });
    }

    /// Asks for a name and creates a mapping to `port`.
    fn prompt_map(&mut self, port: u16, window: &mut Window, cx: &mut Context<Self>) {
        let target = format!("localhost:{port}");
        prompt(
            cx.weak_entity(),
            window,
            cx,
            format!("Map port {port}"),
            "Create",
            vec![
                PromptField::new("Subdomain", "api  (becomes api.localhost)"),
                PromptField::new("Target", "localhost:3000").initial(target),
            ],
            |this, values, _, cx| {
                let name = values[0].to_lowercase();
                let domain = if name.ends_with(".localhost") { name } else { format!("{name}.localhost") };
                let model = this.model.read(cx);
                let client = model.client().clone();
                let payload = json!({
                    "kind": "mappings",
                    "workspaceId": model.workspace_id(),
                    "entity": { "domain": domain, "target": values[1], "enabled": true },
                });
                call(&client, cx, "entity.create", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.model.update(cx, |model, cx| {
                            model.notice(format!("Couldn't create the mapping. {message}"), cx)
                        });
                    }
                });
            },
        );
    }

    fn render_rows(&self, services: &[Service], cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        // The name each mapped port answers to.
        let mapped: Vec<(u16, String)> = self
            .model
            .read(cx)
            .config()
            .mappings
            .iter()
            .filter_map(|m| Some((target_port(&m.target)?, m.domain.clone())))
            .collect();
        let rows: Vec<gpui_kit::AnyElement> = services
            .iter()
            .filter(|service| {
                needle.is_empty()
                    || service.process.to_lowercase().contains(&needle)
                    || service.port.to_string().contains(&needle)
            })
            .map(|service| {
                let port = service.port;
                let name = mapped.iter().find(|(mapped_port, _)| *mapped_port == port).map(|(_, name)| name.clone());
                h_flex()
                    .id(("service", usize::from(port)))
                    .gap_3()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().w_20().flex_none().font_family(theme.mono_font_family.clone()).child(port.to_string()))
                    .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(service.process.clone()))
                    .child(
                        div()
                            .w_24()
                            .flex_none()
                            .text_color(theme.muted_foreground)
                            .child(service.pid.map(|pid| pid.to_string()).unwrap_or_default()),
                    )
                    .child(
                        div()
                            .w_40()
                            .flex_none()
                            .text_color(theme.muted_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(service.address.clone()),
                    )
                    .child(
                        div().w_64().flex_none().child(match name {
                            Some(name) => h_flex()
                                .gap_2()
                                .child(Tag::success().small().child("Mapped"))
                                .child(
                                    div()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .font_family(theme.mono_font_family.clone())
                                        .child(name),
                                )
                                .into_any_element(),
                            None => Button::new(("map-service", usize::from(port)))
                                .small()
                                .label("Map…")
                                .tooltip("Give this port a .localhost name")
                                .on_click(cx.listener(move |this, _, window, cx| this.prompt_map(port, window, cx)))
                                .into_any_element(),
                        }),
                    )
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            return page::empty("No matches", "No service matches the filter.");
        }
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().w_20().flex_none().child("Port"))
                    .child(div().flex_1().child("Process"))
                    .child(div().w_24().flex_none().child("PID"))
                    .child(div().w_40().flex_none().child("Address"))
                    .child(div().w_64().flex_none().child("Mapping")),
            )
            .child(v_flex().id("service-rows").flex_1().min_h_0().overflow_y_scroll().children(rows))
            .into_any_element()
    }
}

impl Render for ServicesScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            "Services",
            "Programs on this computer that are listening on a port.",
            h_flex()
                .gap_2()
                .child(div().w_64().child(
                    Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ))
                .child(
                    Button::new("refresh-services")
                        .small()
                        .icon(IconName::RefreshCw)
                        .label("Refresh")
                        .loading(self.refreshing)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
            cx,
        );
        let body = match &self.listing {
            Listing::Loading => page::loading("Looking for services…"),
            Listing::Failed(message) => page::empty("Couldn't list services", message.clone()),
            Listing::Loaded(services) if services.is_empty() => {
                page::empty("No services found", "Nothing on this computer is listening on a port right now.")
            }
            Listing::Loaded(services) => {
                let services = services.clone();
                self.render_rows(&services, cx)
            }
        };
        page::screen(header, body, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_is_read_from_any_way_a_target_is_written() {
        assert_eq!(target_port("localhost:3000"), Some(3000));
        assert_eq!(target_port("http://127.0.0.1:8081/path"), Some(8081));
        assert_eq!(target_port("3000"), Some(3000));
        assert_eq!(target_port("localhost"), None);
    }
}
