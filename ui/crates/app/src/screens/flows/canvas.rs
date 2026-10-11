//! The canvas a flow is drawn on: its nodes as cards, the connections between their ports as dotted curves,
//! and a run shown on both. A node that is running says so, and the dots of a connection travel along it
//! each time the run takes it.
//!
//! The canvas owns what is only about drawing: how far it is panned, the gesture under way and which
//! connections are moving. What the flow holds is the screen's; the canvas is handed nodes and edges to
//! draw and reports what the pointer asked for as [`CanvasEvent`]s.

use std::{
    cell::Cell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex, spinner::Spinner, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, BorderStyle, Bounds, Context, ElementId, EventEmitter, FocusHandle, Hsla, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Point, Render,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px, quad,
    size,
};

use crate::model::flow::NodeKind;

const NODE_WIDTH: f32 = 208.0;
const HEAD_HEIGHT: f32 = 46.0;
const PORT_HEIGHT: f32 = 24.0;
const HANDLE: f32 = 12.0;

/// How far apart the dots of a connection are, and how fast they travel while it is being taken.
const DOT_SPACING: f32 = 9.0;
const DOT_SPEED: f32 = 70.0;

/// How long a connection keeps moving after the run took it.
const PULSE: Duration = Duration::from_millis(900);

/// A node as the canvas draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasNode {
    pub id: String,
    pub kind: NodeKind,
    pub title: SharedString,
    /// The line under the title: the request the node runs, or what it does.
    pub detail: SharedString,
    pub x: f32,
    pub y: f32,
    /// The ports the node leaves by: id and label.
    pub ports: Vec<(String, SharedString)>,
    /// The flow cannot run as this node stands.
    pub problem: bool,
    /// The node's state in the run shown: `idle`, `running`, `listening`, `succeeded`, `failed`, `stopped`.
    pub status: SharedString,
    pub runs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasEdge {
    pub id: String,
    pub from: String,
    pub port: String,
    pub to: String,
    /// How often the run shown took it.
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CanvasEvent {
    /// A node was picked, or the empty canvas was.
    Selected(Option<String>),
    /// A node was dragged to a new place.
    Moved { id: String, x: f32, y: f32 },
    /// A port was dragged onto a node.
    Connected { from: String, port: String, to: String },
    /// Delete was pressed with this node picked.
    Delete(String),
}

enum Gesture {
    None,
    Pan {
        last: Point<Pixels>,
    },
    /// `grab` is where in the node the pointer holds it.
    Move {
        id: String,
        grab: (f32, f32),
        moved: bool,
    },
    Connect {
        from: String,
        port: String,
        to: Point<Pixels>,
    },
}

pub struct FlowCanvas {
    nodes: Vec<CanvasNode>,
    edges: Vec<CanvasEdge>,
    selected: Option<String>,
    /// The canvas takes the keyboard when it is clicked, so Delete is the picked node's and not a field's.
    focus: FocusHandle,
    pan: (f32, f32),
    gesture: Gesture,
    /// Where the canvas is in the window, as of the last paint: pointer events are in window coordinates.
    origin: Rc<Cell<Point<Pixels>>>,
    /// When each connection was last taken.
    pulses: HashMap<String, Instant>,
    started: Instant,
}

impl EventEmitter<CanvasEvent> for FlowCanvas {}

fn node_height(node: &CanvasNode) -> f32 {
    HEAD_HEIGHT + PORT_HEIGHT * node.ports.len() as f32 + 4.0
}

/// Where a connection enters a node, and where each of its ports leaves it.
fn input_anchor(node: &CanvasNode) -> (f32, f32) {
    (node.x, node.y + HEAD_HEIGHT / 2.0)
}

fn port_anchor(node: &CanvasNode, port: &str) -> Option<(f32, f32)> {
    let index = node.ports.iter().position(|(id, _)| id == port)?;
    Some((node.x + NODE_WIDTH, node.y + HEAD_HEIGHT + PORT_HEIGHT * (index as f32 + 0.5)))
}

/// A point of the curve a connection follows: it leaves and arrives level, bending in between.
fn curve(from: (f32, f32), to: (f32, f32), t: f32) -> (f32, f32) {
    let bend = ((to.0 - from.0).abs() / 2.0).max(40.0);
    let (c1, c2) = ((from.0 + bend, from.1), (to.0 - bend, to.1));
    let u = 1.0 - t;
    let mix =
        |a: f32, b: f32, c: f32, d: f32| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
    (mix(from.0, c1.0, c2.0, to.0), mix(from.1, c1.1, c2.1, to.1))
}

/// The dots of a connection: points `spacing` apart along its curve, the first one `phase` in. Moving the
/// phase moves the dots.
pub fn dots(from: (f32, f32), to: (f32, f32), spacing: f32, phase: f32) -> Vec<(f32, f32)> {
    const STEPS: usize = 48;
    let mut out = Vec::new();
    let mut next = phase.rem_euclid(spacing);
    let (mut walked, mut last) = (0.0, from);
    for step in 1..=STEPS {
        let here = curve(from, to, step as f32 / STEPS as f32);
        let length = ((here.0 - last.0).powi(2) + (here.1 - last.1).powi(2)).sqrt();
        while length > 0.0 && next <= walked + length {
            let along = (next - walked) / length;
            out.push((last.0 + (here.0 - last.0) * along, last.1 + (here.1 - last.1) * along));
            next += spacing;
        }
        walked += length;
        last = here;
    }
    out
}

/// A connection ready to paint, in canvas coordinates.
struct Line {
    from: (f32, f32),
    to: (f32, f32),
    color: Hsla,
    dot: f32,
    phase: f32,
}

impl FlowCanvas {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            selected: None,
            focus: cx.focus_handle(),
            pan: (0.0, 0.0),
            gesture: Gesture::None,
            origin: Rc::new(Cell::new(Point::default())),
            pulses: HashMap::new(),
            started: Instant::now(),
        }
    }

    /// Draw this flow. A node being dragged keeps the place the pointer has it at.
    pub fn set_graph(&mut self, mut nodes: Vec<CanvasNode>, edges: Vec<CanvasEdge>, cx: &mut Context<Self>) {
        if let Gesture::Move { id, .. } = &self.gesture
            && let Some(held) = self.nodes.iter().find(|node| node.id == *id)
            && let Some(node) = nodes.iter_mut().find(|node| node.id == *id)
        {
            (node.x, node.y) = (held.x, held.y);
        }
        if self.nodes != nodes || self.edges != edges {
            self.nodes = nodes;
            self.edges = edges;
            cx.notify();
        }
    }

    pub fn set_selected(&mut self, selected: Option<String>, cx: &mut Context<Self>) {
        if self.selected != selected {
            self.selected = selected;
            cx.notify();
        }
    }

    /// Take the keyboard, as a click on the canvas does.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    /// Another flow is shown: the view starts over.
    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.pan = (0.0, 0.0);
        self.gesture = Gesture::None;
        self.pulses.clear();
        cx.notify();
    }

    /// The run took these connections just now.
    pub fn pulse(&mut self, edge_ids: &[String], cx: &mut Context<Self>) {
        let now = Instant::now();
        self.pulses.retain(|_, at| now.duration_since(*at) < PULSE);
        self.pulses.extend(edge_ids.iter().map(|id| (id.clone(), now)));
        cx.notify();
    }

    /// A free place for a new node: beside the selected node, or after the last one.
    pub fn place_for_new(&self) -> (f32, f32) {
        let beside = self.selected.as_deref().and_then(|id| self.nodes.iter().find(|node| node.id == id));
        match beside {
            Some(node) => (node.x + NODE_WIDTH + 72.0, node.y + 24.0),
            None => {
                let right = self.nodes.iter().map(|node| node.x).fold(-NODE_WIDTH, f32::max);
                let top = self.nodes.iter().map(|node| node.y).fold(f32::INFINITY, f32::min);
                (right + NODE_WIDTH + 72.0, if top.is_finite() { top } else { 120.0 })
            }
        }
    }

    /// The pointer's place on the canvas, in the flow's own coordinates.
    fn world(&self, position: Point<Pixels>) -> (f32, f32) {
        let origin = self.origin.get();
        (f32::from(position.x - origin.x) - self.pan.0, f32::from(position.y - origin.y) - self.pan.1)
    }

    fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id.clone();
            cx.emit(CanvasEvent::Selected(id));
        }
        cx.notify();
    }

    fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        // The button was let go outside the canvas: whatever was under way is over.
        if event.pressed_button != Some(MouseButton::Left) {
            return self.finish_gesture(cx);
        }
        let world = self.world(event.position);
        match &mut self.gesture {
            Gesture::None => return,
            Gesture::Pan { last } => {
                self.pan.0 += f32::from(event.position.x - last.x);
                self.pan.1 += f32::from(event.position.y - last.y);
                *last = event.position;
            }
            Gesture::Move { id, grab, moved } => {
                if let Some(node) = self.nodes.iter_mut().find(|node| node.id == *id) {
                    (node.x, node.y) = ((world.0 - grab.0).round(), (world.1 - grab.1).round());
                    *moved = true;
                }
            }
            Gesture::Connect { to, .. } => *to = event.position,
        }
        cx.notify();
    }

    fn finish_gesture(&mut self, cx: &mut Context<Self>) {
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        match gesture {
            Gesture::None => return,
            Gesture::Move { id, moved: true, .. } => {
                if let Some(node) = self.nodes.iter().find(|node| node.id == id) {
                    cx.emit(CanvasEvent::Moved { id, x: node.x, y: node.y });
                }
            }
            Gesture::Move { .. } | Gesture::Pan { .. } | Gesture::Connect { .. } => {}
        }
        cx.notify();
    }

    /// The pointer was let go over a node: a connection being drawn ends there.
    fn released_on(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Gesture::Connect { from, port, .. } = &self.gesture
            && from != id
        {
            cx.emit(CanvasEvent::Connected { from: from.clone(), port: port.clone(), to: id.to_owned() });
            self.gesture = Gesture::None;
            cx.notify();
        }
    }

    /// Every connection as it is painted now, and whether any of them is moving.
    fn lines(&self, cx: &Context<Self>) -> (Vec<Line>, bool) {
        let theme = cx.theme();
        let clock = self.started.elapsed().as_secs_f32() * DOT_SPEED;
        let place = |anchor: (f32, f32)| (anchor.0 + self.pan.0, anchor.1 + self.pan.1);
        let find = |id: &str| self.nodes.iter().find(|node| node.id == id);
        let mut moving = false;
        let mut lines: Vec<Line> = self
            .edges
            .iter()
            .filter_map(|edge| {
                let from = place(port_anchor(find(&edge.from)?, &edge.port)?);
                let to = place(input_anchor(find(&edge.to)?));
                let live = self.pulses.get(&edge.id).is_some_and(|at| at.elapsed() < PULSE);
                moving |= live;
                Some(match (live, edge.count > 0) {
                    (true, _) => Line { from, to, color: theme.primary, dot: 4.0, phase: clock },
                    (false, true) => Line { from, to, color: theme.primary.opacity(0.6), dot: 3.0, phase: 0.0 },
                    (false, false) => {
                        Line { from, to, color: theme.muted_foreground.opacity(0.7), dot: 2.5, phase: 0.0 }
                    }
                })
            })
            .collect();
        if let Gesture::Connect { from, port, to } = &self.gesture
            && let Some(anchor) = find(from).and_then(|node| port_anchor(node, port))
        {
            let origin = self.origin.get();
            let to = (f32::from(to.x - origin.x), f32::from(to.y - origin.y));
            lines.push(Line { from: place(anchor), to, color: theme.primary, dot: 3.0, phase: 0.0 });
        }
        (lines, moving)
    }

    fn render_status(node: &CanvasNode, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let label = |text: &'static str, color: Hsla, icon: AnyElement| {
            h_flex().flex_none().gap_1().text_xs().text_color(color).child(icon).child(text).into_any_element()
        };
        let icon = |name: IconName, color: Hsla| Icon::new(name).small().text_color(color).into_any_element();
        match node.status.as_ref() {
            "running" => label("Running", theme.primary, Spinner::new().small().into_any_element()),
            "listening" => label("Listening", theme.info, Spinner::new().small().into_any_element()),
            "succeeded" => label("Done", theme.success, icon(IconName::CircleCheck, theme.success)),
            "failed" => label("Failed", theme.danger, icon(IconName::CircleX, theme.danger)),
            "stopped" => label("Stopped", theme.muted_foreground, icon(IconName::Minus, theme.muted_foreground)),
            _ if node.problem => label("Fix", theme.warning, icon(IconName::TriangleAlert, theme.warning)),
            _ => div().into_any_element(),
        }
    }

    fn render_node(&self, node: &CanvasNode, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let selected = self.selected.as_deref() == Some(node.id.as_str());
        let border = match node.status.as_ref() {
            "running" => theme.primary,
            "listening" => theme.info,
            "succeeded" => theme.success,
            "failed" => theme.danger,
            _ if selected => theme.ring,
            _ => theme.border,
        };
        let icon = match node.kind {
            NodeKind::Start => IconName::Play,
            NodeKind::Request => IconName::Globe,
            NodeKind::Stream => IconName::Inbox,
            NodeKind::Mcp => IconName::Bot,
            NodeKind::Condition => IconName::Replace,
            NodeKind::Delay => IconName::Pause,
        };
        let id = node.id.clone();
        let named = |part: &str| ElementId::Name(format!("flow-node-{part}-{id}").into());
        let (down_id, up_id) = (id.clone(), id.clone());

        let ports = node.ports.iter().enumerate().map(|(index, (port, label))| {
            let (from, port) = (id.clone(), port.clone());
            let handle = div()
                .id(ElementId::Name(format!("flow-port-{id}-{port}").into()))
                .absolute()
                .right(px(-HANDLE / 2.0))
                .top(px(HEAD_HEIGHT + PORT_HEIGHT * (index as f32 + 0.5) - HANDLE / 2.0))
                .size(px(HANDLE))
                .rounded_full()
                .border_1()
                .border_color(theme.background)
                .bg(theme.primary)
                .cursor_pointer()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("Drag to the node that runs next").build(window, cx)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.gesture = Gesture::Connect { from: from.clone(), port: port.clone(), to: event.position };
                        cx.stop_propagation();
                        cx.notify();
                    }),
                );
            let row = h_flex()
                .h(px(PORT_HEIGHT))
                .justify_end()
                .pr_3()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label.clone());
            (row, handle)
        });
        let (rows, handles): (Vec<_>, Vec<_>) = ports.unzip();

        div()
            .id(named("card"))
            .absolute()
            .left(px(node.x + self.pan.0))
            .top(px(node.y + self.pan.1))
            .w(px(NODE_WIDTH))
            .h(px(node_height(node)))
            .rounded(theme.radius)
            .border_1()
            .when(selected, |this| this.border_2())
            .border_color(border)
            .bg(theme.popover)
            .shadow_sm()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    // Stopping the event below also keeps the canvas from taking the keyboard by itself.
                    this.focus.focus(window, cx);
                    let world = this.world(event.position);
                    if let Some(node) = this.nodes.iter().find(|node| node.id == down_id) {
                        let grab = (world.0 - node.x, world.1 - node.y);
                        this.gesture = Gesture::Move { id: down_id.clone(), grab, moved: false };
                    }
                    this.select(Some(down_id.clone()), cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(move |this, _, _, cx| this.released_on(&up_id, cx)))
            .child(
                h_flex()
                    .h(px(HEAD_HEIGHT))
                    .px_3()
                    .gap_2()
                    .child(Icon::new(icon).small().text_color(theme.muted_foreground))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(node.title.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(node.detail.clone()),
                            ),
                    )
                    .when(node.runs > 1, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!("×{}", node.runs)),
                        )
                    })
                    .child(Self::render_status(node, cx)),
            )
            .children(rows)
            .when(node.kind != NodeKind::Start, |this| {
                // Where connections arrive.
                this.child(
                    div()
                        .absolute()
                        .left(px(-HANDLE / 2.0))
                        .top(px(HEAD_HEIGHT / 2.0 - HANDLE / 2.0))
                        .size(px(HANDLE))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.background),
                )
            })
            .children(handles)
            .into_any_element()
    }
}

impl Render for FlowCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (lines, moving) = self.lines(cx);
        // A connection that is being taken is drawn again every frame, for as long as it moves.
        if moving {
            window.request_animation_frame();
        }
        let origin = self.origin.clone();
        let connections = canvas(
            move |bounds: Bounds<Pixels>, _, _| origin.set(bounds.origin),
            move |bounds: Bounds<Pixels>, _, window, _| {
                for line in &lines {
                    for (x, y) in dots(line.from, line.to, DOT_SPACING, line.phase) {
                        let corner = bounds.origin + point(px(x - line.dot / 2.0), px(y - line.dot / 2.0));
                        let dot = Bounds::new(corner, size(px(line.dot), px(line.dot)));
                        window.paint_quad(quad(
                            dot,
                            px(line.dot / 2.0),
                            line.color,
                            px(0.0),
                            line.color,
                            BorderStyle::default(),
                        ));
                    }
                }
            },
        )
        .absolute()
        .size_full();
        let nodes: Vec<AnyElement> = self.nodes.iter().map(|node| self.render_node(node, cx)).collect();

        div()
            .id("flow-canvas")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.muted.opacity(0.35))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "delete" | "backspace")
                    && let Some(id) = this.selected.clone()
                {
                    cx.emit(CanvasEvent::Delete(id));
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.gesture = Gesture::Pan { last: event.position };
                    this.select(None, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| this.pointer_moved(event, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.finish_gesture(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.finish_gesture(cx)))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height());
                this.pan.0 += f32::from(delta.x);
                this.pan.1 += f32::from(delta.y);
                cx.notify();
            }))
            .child(connections)
            .children(nodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, x: f32, ports: &[&str]) -> CanvasNode {
        CanvasNode {
            id: id.to_owned(),
            kind: NodeKind::Request,
            title: id.to_owned().into(),
            detail: SharedString::default(),
            x,
            y: 100.0,
            ports: ports.iter().map(|port| (port.to_string(), SharedString::from(port.to_string()))).collect(),
            problem: false,
            status: "idle".into(),
            runs: 0,
        }
    }

    #[test]
    fn a_connection_leaves_its_port_and_arrives_at_the_side_of_the_next_node() {
        let from = node("a", 0.0, &["success", "error"]);
        assert_eq!(port_anchor(&from, "success"), Some((NODE_WIDTH, 100.0 + HEAD_HEIGHT + PORT_HEIGHT * 0.5)));
        assert_eq!(port_anchor(&from, "error"), Some((NODE_WIDTH, 100.0 + HEAD_HEIGHT + PORT_HEIGHT * 1.5)));
        assert_eq!(port_anchor(&from, "nope"), None);
        assert_eq!(input_anchor(&node("b", 300.0, &[])), (300.0, 100.0 + HEAD_HEIGHT / 2.0));
        assert_eq!(curve((0.0, 0.0), (100.0, 50.0), 0.0), (0.0, 0.0));
        assert_eq!(curve((0.0, 0.0), (100.0, 50.0), 1.0), (100.0, 50.0));
    }

    #[test]
    fn dots_are_evenly_spaced_and_move_with_the_phase() {
        let (from, to) = ((0.0, 0.0), (200.0, 0.0));
        let still = dots(from, to, 10.0, 0.0);
        assert_eq!(still.len(), 21, "a dot every 10 along a straight 200");
        assert!(still.iter().all(|(_, y)| y.abs() < 0.001));
        assert!(still.windows(2).all(|pair| (pair[1].0 - pair[0].0 - 10.0).abs() < 0.5), "{still:?}");

        let moved = dots(from, to, 10.0, 4.0);
        assert!((moved[0].0 - 4.0).abs() < 0.5, "the first dot is the phase in: {:?}", moved[0]);
        assert_eq!(dots(from, to, 10.0, 14.0)[0], moved[0], "a whole spacing later the dots are where they were");
    }
}
