//! The strip of open tabs above a screen's editors.
//!
//! Only the tabs in view are built: the window is redrawn whole on every frame it is moved or resized, and
//! a strip that built every open tab made that cost grow with how many were open. Tabs therefore share one
//! width, which is what lets the strip know which ones are in view without building the others.
//!
//! The strip owns nothing. The screen owns its tabs and says what each one shows ([`StripTab`]); the strip
//! reports what the user asked for ([`TabAction`]) and the screen does it.

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    h_flex, h_virtual_list,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Rems, Render, SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    WeakEntity, Window, div, px, rems, size,
};

use crate::i18n::t;

/// The width every tab takes. A longer title is cut with an ellipsis.
const TAB_WIDTH: Rems = rems(11.);

/// What one tab shows.
#[derive(Default)]
pub struct StripTab {
    /// Identifies the tab for as long as it is open, whatever its position.
    pub id: SharedString,
    /// A few characters before the title: a method, a type.
    pub badge: SharedString,
    /// A coloured dot before the title, for a tab that shows how something is doing.
    pub dot: Option<Hsla>,
    pub title: SharedString,
    /// It holds changes that are not saved.
    pub dirty: bool,
    /// What it shows can be copied into a new entity.
    pub can_duplicate: bool,
    /// The user cannot close it from the strip: no close button, no menu.
    pub fixed: bool,
}

/// What the user asked for. Positions are the ones the strip was drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAction {
    Select(usize),
    Close(usize),
    CloseOthers(usize),
    CloseToRight(usize),
    CloseAll,
    Duplicate(usize),
}

type Describe<V> = fn(&V, usize, &App) -> Option<StripTab>;
type Handler<V> = fn(&mut V, TabAction, &mut Window, &mut Context<V>);

pub struct TabStrip<V> {
    pub id: &'static str,
    pub count: usize,
    pub active: Option<usize>,
    /// Kept by the screen, so it can bring a tab into view when it becomes the active one.
    pub scroll: VirtualListScrollHandle,
    pub describe: Describe<V>,
    pub on_action: Handler<V>,
}

fn emit<V: 'static>(
    view: &WeakEntity<V>,
    on_action: Handler<V>,
    action: TabAction,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let view = view.clone();
    move |_, window, cx| {
        view.update(cx, |this, cx| on_action(this, action, window, cx)).ok();
    }
}

fn render_tab<V: 'static>(
    tab: StripTab,
    ix: usize,
    active: bool,
    count: usize,
    on_action: Handler<V>,
    window: &Window,
    cx: &mut Context<V>,
) -> AnyElement {
    let theme = cx.theme();
    let view = cx.weak_entity();
    let (menu_view, middle_view) = (view.clone(), view.clone());
    let close = emit(&view, on_action, TabAction::Close(ix));
    let (can_duplicate, fixed) = (tab.can_duplicate, tab.fixed);
    let tab_element = h_flex()
        .id(ElementId::Name(format!("tab-{}", tab.id).into()))
        .test_support()
        .flex_none()
        .w(TAB_WIDTH.to_pixels(window.rem_size()))
        .h_7()
        .gap_1p5()
        .pl_2()
        .pr_1()
        .when(fixed, |this| this.pr_2())
        .text_sm()
        .border_r_1()
        .border_color(theme.border)
        .map(|this| {
            if active {
                this.bg(theme.tab_active).text_color(theme.tab_active_foreground)
            } else {
                this.text_color(theme.tab_foreground).hover(|this| this.text_color(theme.tab_active_foreground))
            }
        })
        .child(div().flex_none().text_xs().text_color(theme.muted_foreground).child(tab.badge))
        .children(tab.dot.map(|color| div().flex_none().size_2().rounded_full().bg(color)))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(tab.title))
        .children(tab.dirty.then(|| div().flex_none().text_xs().child("●")))
        .children((!fixed).then(|| {
            Button::new("close-tab")
                .icon(IconName::Close)
                .ghost()
                .xsmall()
                .tooltip(t("components.entity_bar.close"))
                .on_click(move |event, window, cx| {
                    // The tab under the button would otherwise take the click as a selection.
                    cx.stop_propagation();
                    close(event, window, cx);
                })
        }))
        .on_click(emit(&view, on_action, TabAction::Select(ix)));
    if fixed {
        return tab_element.into_any_element();
    }
    tab_element
        .on_mouse_down(MouseButton::Middle, move |_, window, cx| {
            middle_view.update(cx, |this, cx| on_action(this, TabAction::Close(ix), window, cx)).ok();
        })
        .context_menu(move |menu, _, _| {
            let item = |label: &'static str, action: TabAction| {
                PopupMenuItem::new(label).on_click(emit(&menu_view, on_action, action))
            };
            menu.item(item(t("components.entity_bar.close"), TabAction::Close(ix)))
                .item(item(t("components.tab_strip.close_others"), TabAction::CloseOthers(ix)).disabled(count < 2))
                .item(
                    item(t("components.tab_strip.close_to_right"), TabAction::CloseToRight(ix))
                        .disabled(ix + 1 >= count),
                )
                .item(item(t("components.tab_strip.close_all"), TabAction::CloseAll))
                .separator()
                .item(item(t("components.tab_strip.duplicate"), TabAction::Duplicate(ix)).disabled(!can_duplicate))
        })
        .into_any_element()
}

/// The strip: `prefix`, the tabs in view, a menu of every open tab, `suffix`.
pub fn tab_strip<V: Render>(
    this: &V,
    strip: TabStrip<V>,
    prefix: impl IntoElement,
    suffix: impl IntoElement,
    window: &Window,
    cx: &mut Context<V>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let TabStrip { id, count, active, scroll, describe, on_action } = strip;
    // Only the width is read; the strip's height is the tabs'.
    let sizes = Rc::new(vec![size(TAB_WIDTH.to_pixels(window.rem_size()), px(0.)); count]);
    let view = cx.weak_entity();
    // The menu lists every tab, in view or not, so one that has scrolled away can still be reached.
    let titles: Vec<(usize, SharedString)> =
        (0..count).filter_map(|ix| describe(this, ix, cx).map(|tab| (ix, tab.title))).collect();

    let tabs = h_virtual_list(cx.entity(), id, sizes, move |this, range, window, cx| {
        range
            .filter_map(|ix| {
                let tab = describe(this, ix, cx)?;
                Some(render_tab(tab, ix, active == Some(ix), count, on_action, window, cx))
            })
            .collect::<Vec<_>>()
    })
    .track_scroll(&scroll);

    h_flex()
        .flex_none()
        .w_full()
        .bg(theme.tab_bar)
        .border_b_1()
        .border_color(theme.border)
        .child(prefix.into_any_element())
        .child(
            div()
                .id("tabs-in-view")
                .flex_1()
                .min_w_0()
                .h_7()
                .overflow_hidden()
                .child(tabs.size_full())
                // A mouse wheel turns one way; over the strip that way is along it.
                .on_scroll_wheel(move |event, window, cx| {
                    let delta = event.delta.pixel_delta(window.line_height() * 3.);
                    let along = if delta.x == px(0.) { delta.y } else { delta.x };
                    let handle = scroll.base_handle();
                    let mut offset = handle.offset();
                    offset.x = (offset.x + along).clamp(-handle.max_offset().x, px(0.));
                    handle.set_offset(offset);
                    cx.stop_propagation();
                    window.refresh();
                }),
        )
        .when(count > 0, |this| {
            this.child(
                h_flex().flex_none().pl_1().child(
                    Button::new("all-tabs")
                        .icon(IconName::ChevronDown)
                        .ghost()
                        .small()
                        .tooltip(t("components.tab_strip.open_tabs"))
                        .dropdown_menu(move |menu, _, _| {
                            titles.iter().fold(menu.scrollable(true), |menu, (ix, title)| {
                                menu.item(
                                    PopupMenuItem::new(title.clone()).checked(active == Some(*ix)).on_click(emit(
                                        &view,
                                        on_action,
                                        TabAction::Select(*ix),
                                    )),
                                )
                            })
                        }),
                ),
            )
        })
        .child(suffix.into_any_element())
        .into_any_element()
}

/// The tabs a screen's user closed last, newest last, for Ctrl+Shift+T.
pub struct ClosedTabs<K>(Vec<K>);

impl<K: PartialEq> ClosedTabs<K> {
    const CAP: usize = 10;

    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, key: K) {
        self.0.retain(|kept| *kept != key);
        self.0.push(key);
        if self.0.len() > Self::CAP {
            self.0.remove(0);
        }
    }

    /// The newest closed tab that `can_open`. Entries that cannot be opened any more are dropped on the way.
    pub fn pop_openable(&mut self, can_open: impl Fn(&K) -> bool) -> Option<K> {
        while let Some(key) = self.0.pop() {
            if can_open(&key) {
                return Some(key);
            }
        }
        None
    }
}

#[cfg(test)]
mod closed_tabs_tests {
    use super::ClosedTabs;

    #[test]
    fn keeps_the_newest_ten_without_repeats() {
        let mut stack = ClosedTabs::new();
        for n in 0..12 {
            stack.push(n);
        }
        stack.push(5);
        assert_eq!(stack.0.len(), 10);
        assert_eq!(stack.pop_openable(|_| true), Some(5));
        assert_eq!(stack.pop_openable(|_| true), Some(11));
        assert_eq!(stack.0[0], 2);
    }

    #[test]
    fn skips_and_drops_entries_that_are_gone() {
        let mut stack = ClosedTabs::new();
        for n in [1, 2, 3] {
            stack.push(n);
        }
        assert_eq!(stack.pop_openable(|n| *n == 1), Some(1));
        assert_eq!(stack.pop_openable(|_| true), None);
    }
}
