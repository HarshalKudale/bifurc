//! A dock panel that shows whichever view its screen points it at.
//!
//! A screen with tabs keeps one dock area, not one per tab: its panels ("Request", "Response") stay where the
//! user put them and follow the active tab. Each such panel is a `HostPanel`; the screen hands it the active
//! tab's view, and with no tab open it says what would appear there.

use gpui_kit::component::{
    ActiveTheme as _,
    dock::{BasePanel, DockArea, DockEvent, Panel, PanelEvent},
};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
};

use crate::{components::page, preferences::{Preferences, pane_size}};

/// Keeps the size of one pane of a two-pane dock area in the preferences, so the next run opens with it.
///
/// A dock reports every layout edit, so this reads the sizes when it is told and the preferences write only
/// when the size moved.
pub fn remember_pane_size<T: 'static>(
    dock: &Entity<DockArea>,
    id: &'static str,
    index: usize,
    cx: &mut Context<T>,
) -> Subscription {
    cx.subscribe(dock, move |_, dock, event: &DockEvent, cx| {
        if matches!(event, DockEvent::LayoutChanged) {
            if let Some(size) = pane_size(&dock.read(cx).dump(cx), 2, index) {
                Preferences::remember_dock_size(cx, id, size);
            }
        }
    })
}

/// One part of an entity's view, as a view of its own.
///
/// An editor that is one entity but fills two dock panels (a mock's rule and its response) gives each panel
/// a `PartView`: it renders by asking the owner for that part, and redraws whenever the owner does.
pub struct PartView<T: 'static> {
    owner: Entity<T>,
    render: fn(&mut T, &mut Window, &mut Context<T>) -> AnyElement,
    _subscription: Subscription,
}

impl<T: 'static> PartView<T> {
    pub fn new(
        owner: &Entity<T>,
        render: fn(&mut T, &mut Window, &mut Context<T>) -> AnyElement,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self { owner: owner.clone(), render, _subscription: cx.observe(owner, |_, _, cx| cx.notify()) })
    }
}

impl<T: 'static> Render for PartView<T> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render = self.render;
        self.owner.update(cx, |owner, cx| render(owner, window, cx))
    }
}

pub struct HostPanel {
    focus_handle: FocusHandle,
    name: &'static str,
    title: SharedString,
    empty_title: SharedString,
    empty_description: SharedString,
    view: Option<AnyView>,
}

impl HostPanel {
    /// `name` identifies the panel to the dock and must be unique within its dock area.
    pub fn new(
        name: &'static str,
        title: impl Into<SharedString>,
        empty_title: impl Into<SharedString>,
        empty_description: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            name,
            title: title.into(),
            empty_title: empty_title.into(),
            empty_description: empty_description.into(),
            view: None,
        }
    }

    pub fn set_view(&mut self, view: Option<AnyView>, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for HostPanel {}

impl Focusable for HostPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for HostPanel {
    fn panel_name(&self) -> &'static str {
        self.name
    }

    fn closable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for HostPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.title.clone())
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for HostPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.view {
            Some(view) => view.clone().into_any_element(),
            None => page::empty(self.empty_title.clone(), self.empty_description.clone()),
        };
        div().size_full().bg(cx.theme().background).child(body)
    }
}
