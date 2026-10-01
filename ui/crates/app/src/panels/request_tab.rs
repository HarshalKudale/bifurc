//! One open request as a dock panel: a tab in the editor area that can be dragged, split off and moved.
//!
//! The panel is a thin shell around a [`Session`]. It titles itself from the session (type or method, name,
//! a mark when there are unsaved edits), tells the shell when it becomes the active tab so the response and
//! details panels can follow it, and refuses to close while it holds unsaved edits.

use gpui_kit::component::{
    ActiveTheme as _,
    dock::{BasePanel, Panel, PanelEvent, TabGroup},
    h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, WeakEntity, Window, div,
};

use crate::{session::Session, shell::Shell};

pub struct RequestTab {
    focus_handle: FocusHandle,
    session: Session,
    shell: WeakEntity<Shell>,
    /// The tab group that draws this panel's tab. It is a separate view, so it has to be told when the title
    /// or the unsaved mark changes.
    group: Option<WeakEntity<TabGroup>>,
    _subscription: Subscription,
}

impl RequestTab {
    pub fn new(session: Session, shell: WeakEntity<Shell>, cx: &mut Context<Self>) -> Self {
        // The title and the dirty mark are drawn by the tab strip from this panel, so a change in the
        // session has to reach the panel for the strip to redraw them.
        let subscription = session.subscribe(cx, |this, _, cx| {
            cx.notify();
            if let Some(group) = &this.group {
                group.update(cx, |_, cx| cx.notify()).ok();
            }
        });
        Self { focus_handle: cx.focus_handle(), session, shell, group: None, _subscription: subscription }
    }

    /// What the tab leads with: a REST request's method is not known without reading the editor, so the
    /// type's short badge is used for every kind.
    fn badge(&self, cx: &App) -> &'static str {
        self.session.kind(cx).badge()
    }
}

impl EventEmitter<PanelEvent> for RequestTab {}

impl Focusable for RequestTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for RequestTab {
    fn panel_name(&self) -> &'static str {
        "RequestTab"
    }

    /// A tab with unsaved edits stays until they are saved or reverted: closing it would discard them, and
    /// the dock has no hook to ask first.
    fn closable(&self, cx: &App) -> bool {
        !self.session.is_dirty(cx)
    }

    fn on_added_to(&mut self, group: WeakEntity<TabGroup>, _: &mut Window, _: &mut Context<Self>) {
        self.group = Some(group);
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if !active {
            return;
        }
        let session = self.session.clone();
        self.shell.update(cx, |shell, cx| shell.activate_session(session, cx)).ok();
    }

    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.clone();
        self.shell.update(cx, |shell, cx| shell.forget_session(&session, cx)).ok();
    }
}

impl Panel for RequestTab {
    /// What the tab strip shows. The dock prefers this over `title` when there are several tabs.
    fn tab_name(&self, cx: &App) -> Option<gpui_kit::SharedString> {
        let title = self.session.title(cx);
        Some(if self.session.is_dirty(cx) { format!("{title} ●").into() } else { title })
    }

    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let badge = self.badge(cx);
        let dirty = self.session.is_dirty(cx);
        h_flex()
            .gap_1p5()
            .child(div().text_xs().text_color(theme.muted_foreground).child(badge))
            .child(self.session.title(cx))
            .when(dirty, |this| this.child(div().text_color(theme.warning).child("●")))
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for RequestTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(cx.theme().background).child(self.session.editor_view())
    }
}
