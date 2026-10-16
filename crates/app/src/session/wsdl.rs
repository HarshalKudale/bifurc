//! What a SOAP request knows about its service's WSDL: the tab that lists the operations.
//!
//! The WSDL is fetched with `soap.fetchWsdl` and lives only in this session's memory. The panel owns the
//! operations and their view; the session (`protocol`) owns the fetch and the choice, because it is the one
//! that can read and fill the editor.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};

use super::protocol::ProtocolSession;
use crate::{
    components::page,
    i18n::{t, t_with},
    model::wsdl::Operation,
};

pub struct WsdlPanel {
    pub fetching: bool,
    /// Where the WSDL is read from; empty means the endpoint's address plus `?wsdl`.
    pub url: Entity<InputState>,
    operations: Option<Vec<Operation>>,
    selected: Option<usize>,
}

impl WsdlPanel {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder(t("session.wsdl.url_placeholder")));
        Self { fetching: false, url, operations: None, selected: None }
    }

    pub fn set_operations(&mut self, operations: Vec<Operation>) {
        self.operations = Some(operations);
        self.selected = None;
    }

    pub fn operations(&self) -> &[Operation] {
        self.operations.as_deref().unwrap_or_default()
    }

    pub fn select(&mut self, ix: usize) {
        self.selected = Some(ix);
    }

    pub fn view(&self, cx: &mut Context<ProtocolSession>) -> AnyElement {
        let theme = cx.theme().clone();
        let fetch = Button::new("fetch-wsdl")
            .small()
            .when(self.operations.is_none(), |this| this.primary())
            .when(self.operations.is_some(), |this| this.outline())
            .label(t(if self.operations.is_some() { "session.wsdl.refresh" } else { "session.wsdl.fetch" }))
            .loading(self.fetching)
            .on_click(cx.listener(|this, _, window, cx| this.fetch_wsdl(window, cx)));
        let body = match &self.operations {
            None => page::empty(t("session.wsdl.empty_title"), t("session.wsdl.empty_description")),
            Some(operations) if operations.is_empty() => {
                page::empty(t("session.wsdl.none_title"), t("session.wsdl.none_description"))
            }
            Some(operations) => v_flex()
                .px_3()
                .pb_3()
                .gap_0p5()
                .text_color(theme.foreground)
                .children(operations.iter().enumerate().map(|(ix, operation)| self.row(ix, operation, cx)))
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.url).small().cleanable(true)))
                    .child(fetch),
            )
            .child(div().id("wsdl-body").flex_1().min_h_0().overflow_y_scroll().child(body))
            .into_any_element()
    }

    /// `Name` with its SOAP version and SOAPAction beside it, and the endpoint under it.
    fn row(&self, ix: usize, operation: &Operation, cx: &mut Context<ProtocolSession>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .id(SharedString::from(format!("wsdl-operation-{ix}-{}", operation.name)))
            .px_1()
            .py_0p5()
            .cursor_pointer()
            .hover(|style| style.bg(theme.muted))
            .when(self.selected == Some(ix), |this| this.bg(theme.muted))
            .on_click(cx.listener(move |this, _, window, cx| this.choose_operation(ix, window, cx)))
            .child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(operation.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t_with("session.wsdl.soap_version", &[("version", &operation.version())])),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(format!("{}  {}", operation.action, operation.endpoint).trim().to_owned()),
            )
            .into_any_element()
    }
}
