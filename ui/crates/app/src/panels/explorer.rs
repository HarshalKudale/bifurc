//! The explorer: a tree of every saved request, and the lists of environments and mappings.
//!
//! One dockable panel whose body follows the module chosen in the rail. It owns only view state — what is
//! expanded, selected, and typed into the filter. What the tree shows comes from the model's snapshot, and
//! everything the user asks for (open, rename, delete, create) leaves as an [`ExplorerEvent`] for the shell,
//! which turns it into engine commands. After a command the engine says what changed and the tree rebuilds.

use std::collections::HashSet;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    switch::Switch,
    tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, div, rems,
};

use crate::{
    app_model::AppModel,
    model::{ExplorerNode, NodeRef, RequestKind, build_explorer},
    shell::Module,
};

/// What the user asked for in the explorer. The shell acts on it.
#[derive(Debug, Clone)]
pub enum ExplorerEvent {
    Open(NodeRef),
    NewRequest { kind: RequestKind, folder: Option<String> },
    NewFolder { kind: RequestKind, parent: Option<String> },
    Rename(NodeRef),
    Delete(NodeRef),
    NewMapping,
    DeleteMapping(String),
}

pub struct ExplorerPanel {
    focus_handle: FocusHandle,
    model: Entity<AppModel>,
    module: Module,
    filter: Entity<InputState>,
    tree: Entity<TreeState>,
    /// Folders the user has opened, by tree id, so a rebuild does not collapse them.
    expanded: HashSet<String>,
    selected: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for ExplorerPanel {}
impl EventEmitter<ExplorerEvent> for ExplorerPanel {}

impl ExplorerPanel {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter requests"));
        let tree = cx.new(|cx| TreeState::new(cx));

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&filter, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.rebuild(cx);
            }
        }));
        subscriptions.push(cx.subscribe(&tree, |this, _, event: &TreeEvent, _| match event {
            TreeEvent::Expanded(id) => {
                this.expanded.insert(id.to_string());
            }
            TreeEvent::Collapsed(id) => {
                this.expanded.remove(id.as_str());
            }
        }));
        subscriptions.push(cx.observe(&model, |this, _, cx| {
            this.rebuild(cx);
            cx.notify();
        }));

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            model,
            module: Module::Requests,
            filter,
            tree,
            expanded: HashSet::new(),
            selected: None,
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
    }

    pub fn set_module(&mut self, module: Module, cx: &mut Context<Self>) {
        if self.module != module {
            self.module = module;
            cx.notify();
        }
    }

    /// Selects a request in the tree, opening the folders above it.
    pub fn reveal(&mut self, node: &NodeRef, cx: &mut Context<Self>) {
        let id = node.encode();
        self.selected = Some(id.clone());
        let id: SharedString = id.into();
        self.tree.update(cx, |tree, cx| {
            tree.reveal_item(&id, gpui_kit::ScrollStrategy::Center, cx);
            if let Some(ix) = tree.index_of(&id) {
                tree.set_selected_index(Some(ix), cx);
            }
        });
    }

    fn to_item(&self, node: &ExplorerNode, force_open: bool) -> TreeItem {
        let id = node.node.encode();
        let open = force_open || self.expanded.contains(&id);
        TreeItem::new(id, node.label.clone())
            .expanded(open)
            .children(node.children.iter().map(|child| self.to_item(child, force_open)))
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).value().to_string();
        let nodes = build_explorer(self.model.read(cx).config(), &filter);
        let filtering = !filter.trim().is_empty();
        let items: Vec<TreeItem> = nodes.iter().map(|n| self.to_item(n, filtering)).collect();
        let selected: Option<SharedString> = self.selected.clone().map(Into::into);
        self.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            if let Some(ix) = selected.as_ref().and_then(|id| tree.index_of(id)) {
                tree.set_selected_index(Some(ix), cx);
            }
        });
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let explorer = cx.entity().downgrade();
        h_flex()
            .gap_1()
            .p_2()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ),
            )
            .child(
                Button::new("explorer-new")
                    .icon(IconName::Plus)
                    .ghost()
                    .small()
                    .tooltip("New request or folder")
                    .dropdown_menu(move |menu, window, cx| new_menu(menu, &explorer, window, cx)),
            )
    }

    fn render_requests(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let model = self.model.clone();
        let explorer = cx.entity().downgrade();
        let menu_explorer = explorer.clone();
        let has_requests =
            !self.model.read(cx).config().requests.is_empty() || !self.model.read(cx).config().folders.is_empty();
        let filtering = !self.filter.read(cx).value().trim().is_empty();

        v_flex()
            .size_full()
            .child(self.render_header(cx))
            .child(if !has_requests {
                div()
                    .flex_1()
                    .child(
                        Empty::new()
                            .header(EmptyHeader::new().title(EmptyTitle::new().child("No requests yet")).description(
                                EmptyDescription::new().child("Create one to start working in this workspace."),
                            ))
                            .content(gpui_kit::component::empty::EmptyContent::new().child(
                                Button::new("explorer-empty-new").label("New request").on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(crate::commands::NewRestRequest), cx)
                                }),
                            )),
                    )
                    .into_any_element()
            } else if filtering && self.is_tree_empty(cx) {
                div()
                    .flex_1()
                    .p_3()
                    .text_color(cx.theme().muted_foreground)
                    .child("No request matches the filter.")
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        tree(&self.tree, move |ix, entry, selected, _, cx| {
                            render_row(ix, entry, selected, &model, &explorer, cx)
                        })
                        .context_menu(move |_, entry, menu, _, _| context_menu(entry, menu, &menu_explorer)),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn is_tree_empty(&self, cx: &App) -> bool {
        self.tree.read(cx).entry(0).is_none()
    }

    fn render_environments(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let config = self.model.read(cx).config().clone();
        let model = self.model.clone();
        let active = config.active_environment_id.clone();
        let mut rows: Vec<gpui_kit::AnyElement> = Vec::new();
        rows.push(
            env_row("env-none", "No environment", active.is_none(), {
                let model = model.clone();
                move |cx: &mut App| model.update(cx, |m, cx| m.set_active_environment(None, cx))
            })
            .into_any_element(),
        );
        for env in config.selectable_environments() {
            let id = env.id.clone();
            let model = model.clone();
            rows.push(
                env_row(
                    SharedString::from(format!("env-{}", env.id)),
                    env.name.clone(),
                    active.as_deref() == Some(env.id.as_str()),
                    move |cx: &mut App| model.update(cx, |m, cx| m.set_active_environment(Some(&id), cx)),
                )
                .into_any_element(),
            );
        }
        v_flex()
            .id("environment-list")
            .size_full()
            .overflow_y_scroll()
            .p_2()
            .gap_1()
            .child(div().px_2().pb_1().text_sm().text_color(theme.muted_foreground).child("Active environment"))
            .children(rows)
            .child(div().px_2().pt_3().text_sm().text_color(theme.muted_foreground).child(
                "Variables of the active environment are listed in Details. Editing them isn't available here yet.",
            ))
            .into_any_element()
    }

    fn render_mappings(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let mappings = self.model.read(cx).config().mappings.clone();
        let explorer = cx.entity().downgrade();
        let model = self.model.clone();

        v_flex()
            .size_full()
            .child(
                h_flex().p_2().gap_1().justify_end().child(
                    Button::new("mapping-new")
                        .icon(IconName::Plus)
                        .ghost()
                        .small()
                        .tooltip("New mapping")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ExplorerEvent::NewMapping))),
                ),
            )
            .child(if mappings.is_empty() {
                div()
                    .flex_1()
                    .child(Empty::new().header(
                        EmptyHeader::new().title(EmptyTitle::new().child("No mappings")).description(
                            EmptyDescription::new().child("A mapping sends a .localhost name to a local port."),
                        ),
                    ))
                    .into_any_element()
            } else {
                v_flex()
                    .id("mapping-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .gap_1()
                    .children(mappings.into_iter().map(|mapping| {
                        let toggle_model = model.clone();
                        let toggle_id = mapping.id.clone();
                        let delete_explorer = explorer.clone();
                        let delete_id = mapping.id.clone();
                        h_flex()
                            .id(gpui_kit::ElementId::Name(format!("mapping-{}", mapping.id).into()))
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(theme.radius)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(mapping.label.clone().unwrap_or(mapping.domain.clone())),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.muted_foreground)
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(format!("{} → {}", mapping.domain, mapping.target)),
                                    ),
                            )
                            .child(
                                Switch::new(gpui_kit::ElementId::Name(format!("mapping-switch-{}", mapping.id).into()))
                                    .checked(mapping.enabled)
                                    .small()
                                    .on_change(move |checked: &bool, _, cx| {
                                        let checked = *checked;
                                        toggle_model.update(cx, |m, cx| m.set_mapping_enabled(&toggle_id, checked, cx));
                                    }),
                            )
                            .child(
                                Button::new(gpui_kit::ElementId::Name(format!("mapping-delete-{}", mapping.id).into()))
                                    .icon(IconName::Delete)
                                    .ghost()
                                    .small()
                                    .tooltip("Delete mapping")
                                    .on_click(move |_, _, cx| {
                                        delete_explorer
                                            .update(cx, |_, cx| {
                                                cx.emit(ExplorerEvent::DeleteMapping(delete_id.clone()))
                                            })
                                            .ok();
                                    }),
                            )
                    }))
                    .into_any_element()
            })
            .into_any_element()
    }
}

fn env_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    on_pick: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let id: SharedString = id.into();
    ListItem::new(gpui_kit::ElementId::Name(id))
        .selected(selected)
        .px_2()
        .py_1()
        .child(
            h_flex()
                .gap_2()
                .child(div().w_4().child(if selected {
                    Icon::new(IconName::Check).small().into_any_element()
                } else {
                    div().into_any_element()
                }))
                .child(label.into()),
        )
        .on_click(move |_, _, cx| on_pick(cx))
}

fn render_row(
    ix: usize,
    entry: &TreeEntry,
    selected: bool,
    model: &Entity<AppModel>,
    explorer: &WeakEntity<ExplorerPanel>,
    cx: &mut App,
) -> ListItem {
    let theme = cx.theme().clone();
    let node = NodeRef::parse(entry.item().id.as_str());
    let (lead, label): (gpui_kit::AnyElement, SharedString) = match &node {
        Some(NodeRef::Folder { .. }) => (
            Icon::new(if entry.is_expanded() { IconName::FolderOpen } else { IconName::Folder })
                .small()
                .text_color(theme.muted_foreground)
                .into_any_element(),
            entry.item().label.clone(),
        ),
        Some(NodeRef::Request { kind, id }) => {
            let badge = model
                .read(cx)
                .config()
                .request(*kind, id)
                .and_then(|r| r.method.clone())
                .unwrap_or_else(|| kind.badge().to_owned());
            (
                div().w_12().flex_none().text_xs().text_color(theme.muted_foreground).child(badge).into_any_element(),
                entry.item().label.clone(),
            )
        }
        None => (div().into_any_element(), entry.item().label.clone()),
    };

    let open = node.clone().filter(|n| matches!(n, NodeRef::Request { .. }));
    let explorer = explorer.clone();
    ListItem::new(ix)
        .selected(selected)
        .pl(rems(0.5 + entry.depth() as f32 * 1.0))
        .py_1()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(lead)
                .child(div().min_w_0().overflow_hidden().text_ellipsis().child(label)),
        )
        .on_click(move |_, _, cx| {
            if let Some(node) = open.clone() {
                explorer.update(cx, |_, cx| cx.emit(ExplorerEvent::Open(node))).ok();
            }
        })
}

fn context_menu(entry: &TreeEntry, menu: PopupMenu, explorer: &WeakEntity<ExplorerPanel>) -> PopupMenu {
    let Some(node) = NodeRef::parse(entry.item().id.as_str()) else { return menu };
    let emit = |event: ExplorerEvent| {
        let explorer = explorer.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            explorer.update(cx, |_, cx| cx.emit(event.clone())).ok();
        }
    };
    match node {
        NodeRef::Request { .. } => menu
            .item(PopupMenuItem::new("Open").on_click(emit(ExplorerEvent::Open(node.clone()))))
            .separator()
            .item(PopupMenuItem::new("Rename…").on_click(emit(ExplorerEvent::Rename(node.clone()))))
            .item(PopupMenuItem::new("Delete…").on_click(emit(ExplorerEvent::Delete(node.clone())))),
        NodeRef::Folder { kind, ref id } => {
            let authorable = matches!(kind, RequestKind::Rest | RequestKind::WebSocket);
            menu.when(authorable, |menu| {
                menu.item(
                    PopupMenuItem::new(if kind == RequestKind::Rest {
                        "New request here"
                    } else {
                        "New WebSocket here"
                    })
                    .on_click(emit(ExplorerEvent::NewRequest { kind, folder: Some(id.clone()) })),
                )
            })
            .item(
                PopupMenuItem::new("New folder here…")
                    .on_click(emit(ExplorerEvent::NewFolder { kind, parent: Some(id.clone()) })),
            )
            .separator()
            .item(PopupMenuItem::new("Rename…").on_click(emit(ExplorerEvent::Rename(node.clone()))))
            .item(PopupMenuItem::new("Delete…").on_click(emit(ExplorerEvent::Delete(node.clone()))))
        }
    }
}

fn new_menu(
    menu: PopupMenu,
    explorer: &WeakEntity<ExplorerPanel>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let emit = |event: ExplorerEvent| {
        let explorer = explorer.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            explorer.update(cx, |_, cx| cx.emit(event.clone())).ok();
        }
    };
    let folder_explorer = explorer.clone();
    menu.item(
        PopupMenuItem::new("New request")
            .on_click(emit(ExplorerEvent::NewRequest { kind: RequestKind::Rest, folder: None })),
    )
    .item(
        PopupMenuItem::new("New WebSocket")
            .on_click(emit(ExplorerEvent::NewRequest { kind: RequestKind::WebSocket, folder: None })),
    )
    .separator()
    .submenu("New folder", window, cx, move |submenu, _, _| {
        RequestKind::ALL.into_iter().fold(submenu, |submenu, kind| {
            let explorer = folder_explorer.clone();
            submenu.item(PopupMenuItem::new(format!("{} folder…", kind.label())).on_click(
                move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                    explorer.update(cx, |_, cx| cx.emit(ExplorerEvent::NewFolder { kind, parent: None })).ok();
                },
            ))
        })
    })
}

impl Focusable for ExplorerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ExplorerPanel {
    fn panel_name(&self) -> &'static str {
        "ExplorerPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for ExplorerPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.module.label()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ExplorerPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.module {
            Module::Requests => self.render_requests(cx),
            Module::Environments => self.render_environments(cx),
            Module::Mappings => self.render_mappings(cx),
        };
        div().size_full().bg(cx.theme().sidebar).child(body)
    }
}
