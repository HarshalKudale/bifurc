//! The details panel: facts about the active request and the environment its variables resolve against.
//!
//! Read-only on purpose. Anything editable lives in the request's own tab; this is where you look things up
//! while you work, so it can stay open beside the editor.

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    description_list::{DescriptionItem, DescriptionList},
    dock::{BasePanel, Panel, PanelEvent},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    v_flex,
};
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};

use crate::{app_model::AppModel, session::Session};

pub struct DetailsPanel {
    focus_handle: FocusHandle,
    model: Entity<AppModel>,
    active: Option<Session>,
    _model_subscription: Subscription,
    /// Follows the active request so the name and saved state shown here stay current as it loads and saves.
    _session_subscription: Option<Subscription>,
}

impl DetailsPanel {
    pub fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&model, |_, _, cx| cx.notify());
        Self {
            focus_handle: cx.focus_handle(),
            model,
            active: None,
            _model_subscription: subscription,
            _session_subscription: None,
        }
    }

    pub fn set_active(&mut self, session: Option<Session>, cx: &mut Context<Self>) {
        self._session_subscription = session.as_ref().map(|s| s.subscribe(cx, |_, _, cx| cx.notify()));
        self.active = session;
        cx.notify();
    }

    fn request_items(&self, cx: &App) -> Vec<DescriptionItem> {
        let Some(session) = &self.active else { return Vec::new() };
        let kind = session.kind(cx);
        let saved_id = session.saved_id(cx);
        let config = self.model.read(cx).config();
        let stub = saved_id.as_deref().and_then(|id| config.request(kind, id));
        let mut items = vec![
            DescriptionItem::new("Name").value(session.title(cx)),
            DescriptionItem::new("Type").value(kind.label()),
        ];
        if let Some(stub) = stub {
            if !stub.target.is_empty() {
                items.push(DescriptionItem::new("Target").value(stub.target.clone()));
            }
            if let Some(folder) = stub.folder_id.as_deref().and_then(|id| config.folder(kind, id)) {
                items.push(DescriptionItem::new("Folder").value(folder.name.clone()));
            }
        }
        items.push(DescriptionItem::new("Saved").value(match &saved_id {
            Some(id) => format!("Yes ({id})"),
            None => "Not yet".to_owned(),
        }));
        items
    }

    /// The active environment's name and the variables a request would resolve against, in order.
    fn environment(&self, cx: &App) -> (String, Vec<(String, String)>) {
        let config = self.model.read(cx).config();
        let mut variables: Vec<(String, String)> = Vec::new();
        // The global layer applies under every environment; the active one overrides it.
        let layers = config
            .environments
            .iter()
            .filter(|e| e.is_global())
            .chain(config.active_environment().filter(|e| !e.is_global()));
        for env in layers {
            for variable in &env.variables {
                variables.retain(|(key, _)| *key != variable.key);
                variables.push((variable.key.clone(), variable.value.clone()));
            }
        }
        let name = config.active_environment().map(|e| e.name.clone()).unwrap_or_else(|| "No environment".to_owned());
        (name, variables)
    }
}

impl EventEmitter<PanelEvent> for DetailsPanel {}

impl Focusable for DetailsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for DetailsPanel {
    fn panel_name(&self) -> &'static str {
        "DetailsPanel"
    }
}

impl Panel for DetailsPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Details"
    }
}

impl Render for DetailsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (env_name, variables) = self.environment(cx);
        let has_variables = !variables.is_empty();

        v_flex()
            .id("details")
            .size_full()
            .overflow_y_scroll()
            .bg(theme.background)
            .gap_4()
            .p_3()
            .child(match &self.active {
                Some(_) => v_flex()
                    .gap_2()
                    .child(div().font_semibold().child("Request"))
                    .child(DescriptionList::vertical().columns(1).children(self.request_items(cx)))
                    .into_any_element(),
                None => div()
                    .h_40()
                    .child(
                        Empty::new().header(
                            EmptyHeader::new()
                                .title(EmptyTitle::new().child("No request selected"))
                                .description(EmptyDescription::new().child("Details of the open request appear here.")),
                        ),
                    )
                    .into_any_element(),
            })
            .child(
                v_flex()
                    .gap_2()
                    .child(div().font_semibold().child("Environment"))
                    .child(div().text_color(theme.muted_foreground).child(env_name))
                    .child(if has_variables {
                        DescriptionList::vertical()
                            .columns(1)
                            .children(variables.into_iter().map(|(key, value)| DescriptionItem::new(key).value(value)))
                            .into_any_element()
                    } else {
                        div().text_color(theme.muted_foreground).child("No variables are defined.").into_any_element()
                    }),
            )
    }
}
