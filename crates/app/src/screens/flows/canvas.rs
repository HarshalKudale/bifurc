//! The canvas a flow is drawn on. Everything about a node is in the node: what it does, the inputs it takes
//! with where each gets its value, its switches, the ports it leaves by, and under it what it was handed,
//! what it produced and how its tests came out. Connections are dotted curves between ports; the dots of one
//! travel along it each time a run takes it.
//!
//! The canvas owns what is only about drawing: how far it is panned and zoomed, the gesture under way, what
//! is open, and which connections are moving. What the flow holds is the screen's: the canvas is handed the
//! flow, what the engine says of it and its run, and reports what was asked for as [`CanvasEvent`]s.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    spinner::Spinner,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, BorderStyle, Bounds, Context, DismissEvent, Div, Element, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable as _, GlobalElementId, Hsla, InspectorElementId, InteractiveElement as _, IntoElement,
    KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Point, Render,
    ScrollWheelEvent, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, anchored, canvas, deferred, div, point, px, quad, rems, size,
};
use serde_json::Value;

use super::{
    editor::{NodeEditor, rule_summary},
    json_view::{Pick, Toggle, flip, json_tree},
};
use crate::{
    commands::{DuplicateNode, NewRestRequest},
    i18n::{t, t_with},
    model::flow::{
        Described, FlowDoc, FlowEdge, FlowNode, InputInfo, NodeInfo, NodeKind, NodeRun, RunView, path_label,
    },
};

/// How wide a node is, in rems, and what a rem is before the canvas is zoomed.
const NODE_WIDTH: f32 = 19.0;
const REM: f32 = 16.0;

/// How far apart the dots of a connection are, and how fast they travel while it is being taken.
const DOT_SPACING: f32 = 9.0;
const DOT_SPEED: f32 = 70.0;

/// How far apart the dots of the background grid are before the canvas is zoomed.
const GRID_SPACING: f32 = 20.0;

/// How long a connection keeps moving after the run took it.
const PULSE: Duration = Duration::from_millis(900);

/// How near a click must be to a connection to pick it.
const PICK_DISTANCE: f32 = 7.0;

/// How near the cross of a connection the pointer stays on it: the cross is wider than the connection.
const CROSS_REACH: f32 = 14.0;

/// How near a port the end of a connection being drawn is taken by it.
const SNAP_DISTANCE: f32 = 22.0;

/// How far a node is pulled before it is being moved, and not just picked.
const DRAG_SLOP: f32 = 3.0;

/// How narrow a display node can be made, and how low what it shows.
const MIN_SIZE: (f32, f32) = (192.0, 48.0);

const ZOOM_RANGE: (f32, f32) = (0.4, 1.6);

/// One of the three views under a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Input,
    Output,
    Tests,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CanvasEvent {
    /// A node was picked, or the empty canvas was.
    Selected(Option<String>),
    /// A node was dragged to a new place.
    Moved {
        id: String,
        x: f32,
        y: f32,
    },
    /// A display node was pulled by its corner to a new size.
    Resized {
        id: String,
        width: f32,
        height: f32,
    },
    /// A port was dragged onto a node: that node runs next.
    Connect {
        from: String,
        port: String,
        to: String,
    },
    /// A port was dragged onto an input of a node: what leaves by the port is wired into it.
    Wire {
        from: String,
        port: String,
        to: String,
        input: String,
    },
    /// The part of its source's output a wire takes was chosen.
    WirePath {
        edge_id: String,
        path: Vec<Value>,
    },
    /// Remove the connection with this id.
    Disconnect(String),
    Delete(String),
    Duplicate(String),
    /// Add a node of this kind at this place. `link` is the node and the port it runs after, when it was
    /// asked for by drawing a connection to where it goes.
    Add {
        kind: NodeKind,
        x: f32,
        y: f32,
        link: Option<(String, String)>,
    },
    /// A node as a switch on it left it.
    Edited(Box<FlowNode>),
}

enum Gesture {
    None,
    Pan {
        last: Point<Pixels>,
    },
    /// `grab` is where in the node the pointer holds it, `pressed` where in the window it took hold.
    Move {
        id: String,
        grab: (f32, f32),
        pressed: Point<Pixels>,
        moved: bool,
    },
    /// `start` is the size the node had when its corner was pressed.
    Resize {
        id: String,
        pressed: Point<Pixels>,
        start: (f32, f32),
    },
    Connect {
        from: String,
        port: String,
        to: Point<Pixels>,
    },
}

pub fn kind_icon(kind: NodeKind) -> Icon {
    Icon::new(match kind {
        NodeKind::Start => IconName::Play,
        NodeKind::Request => IconName::Globe,
        NodeKind::Stream => IconName::Inbox,
        NodeKind::Mcp => IconName::Bot,
        NodeKind::Condition => IconName::Replace,
        NodeKind::If => IconName::ChevronsUpDown,
        NodeKind::Validate => IconName::CircleCheck,
        NodeKind::Evaluate => IconName::SquareTerminal,
        NodeKind::Delay => IconName::Pause,
        NodeKind::Or => IconName::Network,
        NodeKind::Repeat => IconName::RotateCw,
        NodeKind::ForEach => IconName::RefreshCw,
        NodeKind::Display => IconName::Eye,
        NodeKind::Data => IconName::Asterisk,
        NodeKind::Sample => IconName::FileText,
        NodeKind::Mail => return Icon::new(gpui_kit::assets::IconName::Mail),
    })
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

/// How far `at` is from the connection drawn from `from` to `to`.
fn distance_to(from: (f32, f32), to: (f32, f32), at: (f32, f32)) -> f32 {
    dots(from, to, 4.0, 0.0)
        .into_iter()
        .map(|(x, y)| ((x - at.0).powi(2) + (y - at.1).powi(2)).sqrt())
        .fold(f32::INFINITY, f32::min)
}

/// The zoom after one step in or out from `zoom`, kept within what stays readable.
fn zoom_step(zoom: f32, closer: bool) -> f32 {
    let next = if closer { zoom * 1.1 } else { zoom / 1.1 };
    // A step that lands next to 100% lands on it, so stepping out and back in ends where it began.
    let next = if (next - 1.0).abs() < 0.04 { 1.0 } else { next };
    next.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1)
}

fn anchor_out(node: &str, port: &str) -> String {
    format!("out|{node}|{port}")
}

/// Where a connection arrives: at the input it is wired into, or at the node it runs.
fn anchor_in(node: &str, input: &str) -> String {
    if input.is_empty() { format!("in|{node}") } else { format!("in|{node}|{input}") }
}

/// The node, and the input of it if any, that the anchor a connection arrives at is of.
fn target_of(anchor: &str) -> Option<(&str, Option<&str>)> {
    let mut parts = anchor.strip_prefix("in|")?.splitn(2, '|');
    Some((parts.next()?, parts.next()))
}

/// The kinds of node there are to add, as the items of a menu. One chosen is put at `place`, or where there
/// is room when no place is given, and runs after `link`.
fn kinds_menu(
    menu: PopupMenu,
    canvas: WeakEntity<FlowCanvas>,
    place: Option<(f32, f32)>,
    link: Option<(String, String)>,
) -> PopupMenu {
    NodeKind::ADDABLE.into_iter().fold(menu, |menu, kind| {
        let (canvas, link) = (canvas.clone(), link.clone());
        let item = PopupMenuItem::new(kind.label()).icon(kind_icon(kind)).on_click(move |_, _, cx| {
            canvas
                .update(cx, |this, cx| {
                    let (x, y) = place.unwrap_or_else(|| this.place_for_new());
                    cx.emit(CanvasEvent::Add { kind, x, y, link: link.clone() });
                })
                .ok();
        });
        // The nodes that only hold a value come after the ones that do something.
        match kind {
            NodeKind::If | NodeKind::Delay | NodeKind::Display => menu.separator().item(item),
            _ => menu.item(item),
        }
    })
}

/// A value in a line, as an input's row shows it.
fn one_line(value: &Value) -> String {
    match value {
        Value::Null => "—".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Lays out and paints what is in it with the rem `factor` times its size, which is what zooms a node: its
/// text and its spacing are both measured in rems.
struct Scaled {
    factor: f32,
    child: AnyElement,
}

impl IntoElement for Scaled {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Scaled {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let rem = px(REM * self.factor);
        (window.with_rem_size(Some(rem), |window| self.child.request_layout(window, cx)), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let rem = px(REM * self.factor);
        window.with_rem_size(Some(rem), |window| self.child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let rem = px(REM * self.factor);
        window.with_rem_size(Some(rem), |window| self.child.paint(window, cx));
    }
}

/// A connection as it is painted.
struct EdgeDraw {
    id: String,
    from: String,
    to: String,
    wire: bool,
    taken: bool,
    moving: bool,
    selected: bool,
}

/// A drawn connection: the edge id and its two end points.
type DrawnConnection = (String, (f32, f32), (f32, f32));
type Anchors = Rc<RefCell<HashMap<String, Point<Pixels>>>>;

/// The list a node to add is chosen from, open where it was asked for.
struct Picker {
    menu: Entity<PopupMenu>,
    at: Point<Pixels>,
    /// The node and the port a connection was drawn from to ask for the node.
    link: Option<(String, String)>,
    _dismissed: Subscription,
}

pub struct FlowCanvas {
    doc: Option<FlowDoc>,
    described: Described,
    run: Option<RunView>,
    /// The controls of the selected node, drawn inside it.
    editor: Entity<NodeEditor>,
    selected: Option<String>,
    selected_edge: Option<String>,
    /// The canvas takes the keyboard when it or a node's header is clicked, so Delete is the picked node's
    /// and not a field's.
    focus: FocusHandle,
    pan: (f32, f32),
    zoom: f32,
    gesture: Gesture,
    /// Where the canvas is in the window, as of the last paint: pointer events are in window coordinates.
    viewport: Rc<Cell<Bounds<Pixels>>>,
    /// Where each port is in the window, as of the last layout. Connections are painted between them.
    anchors: Anchors,
    /// The connections as they were last painted, for picking one by a click.
    drawn: Rc<RefCell<Vec<DrawnConnection>>>,
    /// How high what each display node shows is in the window, as of the last layout: what pulling its
    /// corner starts from.
    heights: Rc<RefCell<HashMap<String, f32>>>,
    /// When each connection was last taken.
    pulses: HashMap<String, Instant>,
    started: Instant,
    /// The view that is open under each node.
    details: HashMap<String, Detail>,
    /// The rows that are open in the trees of values.
    open: HashSet<String>,
    /// The wire whose part of its source's output is being chosen.
    choosing: Option<String>,
    /// The connection the pointer is on, which shows the cross that removes it.
    hover_edge: Option<String>,
    /// Where the pointer last was over the canvas: where a node asked for by the keyboard goes.
    pointer: Option<Point<Pixels>>,
    picker: Option<Picker>,
}

impl EventEmitter<CanvasEvent> for FlowCanvas {}

impl FlowCanvas {
    pub fn new(editor: Entity<NodeEditor>, cx: &mut Context<Self>) -> Self {
        Self {
            doc: None,
            described: Described::default(),
            run: None,
            editor,
            selected: None,
            selected_edge: None,
            focus: cx.focus_handle(),
            pan: (0.0, 0.0),
            zoom: 1.0,
            gesture: Gesture::None,
            viewport: Rc::new(Cell::new(Bounds::default())),
            anchors: Rc::default(),
            drawn: Rc::default(),
            heights: Rc::default(),
            pulses: HashMap::new(),
            started: Instant::now(),
            details: HashMap::new(),
            open: HashSet::new(),
            choosing: None,
            hover_edge: None,
            pointer: None,
            picker: None,
        }
    }

    /// Draw this flow, with what the engine says of it and its run. A node being dragged or resized keeps
    /// the place and the size the pointer has it at.
    pub fn set_graph(
        &mut self,
        mut doc: Option<FlowDoc>,
        described: Described,
        run: Option<RunView>,
        cx: &mut Context<Self>,
    ) {
        if let Gesture::Move { id, .. } | Gesture::Resize { id, .. } = &self.gesture
            && let Some(held) = self.doc.as_ref().and_then(|doc| doc.node(id))
            && let Some(node) = doc.as_mut().and_then(|doc| doc.node_mut(id))
        {
            (node.x, node.y, node.width, node.height) = (held.x, held.y, held.width, held.height);
        }
        if self.doc != doc || self.described != described || self.run != run {
            (self.doc, self.described, self.run) = (doc, described, run);
            cx.notify();
        }
    }

    pub fn set_selected(&mut self, selected: Option<String>, cx: &mut Context<Self>) {
        if self.selected != selected {
            self.selected = selected;
            self.choosing = None;
            cx.notify();
        }
    }

    /// Take the keyboard, as a click on the canvas does.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    /// Another flow is shown: the view starts over.
    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        (self.pan, self.zoom) = ((0.0, 0.0), 1.0);
        self.gesture = Gesture::None;
        (self.selected_edge, self.choosing, self.hover_edge, self.picker) = (None, None, None, None);
        self.pulses.clear();
        self.details.clear();
        self.open.clear();
        cx.notify();
    }

    /// The run took these connections just now.
    pub fn pulse(&mut self, edge_ids: &[String], cx: &mut Context<Self>) {
        let now = Instant::now();
        self.pulses.retain(|_, at| now.duration_since(*at) < PULSE);
        self.pulses.extend(edge_ids.iter().map(|id| (id.clone(), now)));
        cx.notify();
    }

    /// A free place for a new node: beside the selected node, or after the last one, and below whatever is
    /// already there.
    fn place_for_new(&self) -> (f32, f32) {
        let nodes = self.doc.as_ref().map(|doc| doc.nodes.as_slice()).unwrap_or(&[]);
        let width = NODE_WIDTH * REM;
        let (x, mut y) = match self.selected.as_deref().and_then(|id| nodes.iter().find(|node| node.id == id)) {
            Some(node) => (node.x + width + 72.0, node.y),
            None => {
                let right = nodes.iter().map(|node| node.x).fold(-width, f32::max);
                let top = nodes.iter().map(|node| node.y).fold(f32::INFINITY, f32::min);
                (right + width + 72.0, if top.is_finite() { top } else { 120.0 })
            }
        };
        // shortcut: a node is taken to be 240 high, which most are; measure them if new nodes land on tall ones.
        while nodes.iter().any(|node| (node.x - x).abs() < width && (node.y - y).abs() < 240.0) {
            y += 260.0;
        }
        (x, y)
    }

    /// The pointer's place on the canvas, in the flow's own coordinates.
    fn world(&self, position: Point<Pixels>) -> (f32, f32) {
        let origin = self.viewport.get().origin;
        let at = (f32::from(position.x - origin.x), f32::from(position.y - origin.y));
        ((at.0 - self.pan.0) / self.zoom, (at.1 - self.pan.1) / self.zoom)
    }

    /// Zoom to `zoom`, keeping what is at `around` (a place in the canvas) where it is.
    fn zoom_to(&mut self, zoom: f32, around: (f32, f32), cx: &mut Context<Self>) {
        let world = ((around.0 - self.pan.0) / self.zoom, (around.1 - self.pan.1) / self.zoom);
        self.zoom = zoom;
        self.pan = (around.0 - world.0 * zoom, around.1 - world.1 * zoom);
        cx.notify();
    }

    fn zoom_by_step(&mut self, closer: bool, cx: &mut Context<Self>) {
        let size = self.viewport.get().size;
        let middle = (f32::from(size.width) / 2.0, f32::from(size.height) / 2.0);
        self.zoom_to(zoom_step(self.zoom, closer), middle, cx);
    }

    fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.selected_edge = None;
        if self.selected != id {
            // A part being chosen for a wire into the node that is picked stays open: the click that opened
            // it is the click that picked the node.
            let edges = self.doc.as_ref().map(|doc| doc.edges.as_slice()).unwrap_or(&[]);
            let into_it = |wire: &String| edges.iter().any(|edge| edge.id == *wire && Some(&edge.to) == id.as_ref());
            self.choosing = self.choosing.take().filter(into_it);
            self.selected = id.clone();
            cx.emit(CanvasEvent::Selected(id));
        }
        cx.notify();
    }

    /// The connection drawn nearest to `at`, a place in the window, when one is near enough to be meant.
    fn edge_near(&self, at: (f32, f32)) -> Option<String> {
        let drawn = self.drawn.borrow();
        let near = drawn
            .iter()
            .map(|(id, from, to)| (id, distance_to(*from, *to, at)))
            .filter(|(_, distance)| *distance <= PICK_DISTANCE)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        near.map(|(id, _)| id.clone())
    }

    /// The pointer moved with nothing held: the connection it is on shows its cross.
    fn hover(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let at = (f32::from(at.x), f32::from(at.y));
        let on_cross = |id: &String| {
            self.drawn.borrow().iter().any(|(drawn, from, to)| {
                let middle = curve(*from, *to, 0.5);
                drawn == id && (middle.0 - at.0).hypot(middle.1 - at.1) <= CROSS_REACH
            })
        };
        let over = self.hover_edge.clone().filter(on_cross).or_else(|| self.edge_near(at));
        if over != self.hover_edge {
            self.hover_edge = over;
            cx.notify();
        }
    }

    /// The port a connection drawn from the node `from` is taken by while its end is at `at`: the nearest in
    /// reach, as the node and the input it is of, and where it is.
    fn snap(&self, from: &str, at: Point<Pixels>) -> Option<(String, Option<String>, Point<Pixels>)> {
        let reach = SNAP_DISTANCE * self.zoom.max(0.7);
        let far = |place: &Point<Pixels>| f32::from(place.x - at.x).hypot(f32::from(place.y - at.y));
        let anchors = self.anchors.borrow();
        let nearest = anchors
            .iter()
            .filter(|(key, place)| far(place) <= reach && target_of(key).is_some_and(|(node, _)| node != from))
            .min_by(|a, b| far(a.1).total_cmp(&far(b.1)));
        let (key, place) = nearest?;
        let (node, input) = target_of(key)?;
        Some((node.to_owned(), input.map(str::to_owned), *place))
    }

    fn pointer_moved(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.pointer = Some(event.position);
        // The button was let go outside the canvas: whatever was under way is over.
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_gesture(false, window, cx);
            return self.hover(event.position, cx);
        }
        let world = self.world(event.position);
        match &mut self.gesture {
            Gesture::None => return,
            Gesture::Pan { last } => {
                self.pan.0 += f32::from(event.position.x - last.x);
                self.pan.1 += f32::from(event.position.y - last.y);
                *last = event.position;
            }
            Gesture::Move { id, grab, pressed, moved } => {
                let pulled = f32::from(event.position.x - pressed.x).hypot(f32::from(event.position.y - pressed.y));
                *moved |= pulled > DRAG_SLOP;
                if *moved && let Some(node) = self.doc.as_mut().and_then(|doc| doc.node_mut(id)) {
                    (node.x, node.y) = ((world.0 - grab.0).round(), (world.1 - grab.1).round());
                }
            }
            Gesture::Resize { id, pressed, start } => {
                let wider = f32::from(event.position.x - pressed.x) / self.zoom;
                let higher = f32::from(event.position.y - pressed.y) / self.zoom;
                if let Some(node) = self.doc.as_mut().and_then(|doc| doc.node_mut(id)) {
                    node.width = (start.0 + wider).max(MIN_SIZE.0).round();
                    node.height = (start.1 + higher).max(MIN_SIZE.1).round();
                }
            }
            Gesture::Connect { to, .. } => *to = event.position,
        }
        cx.notify();
    }

    /// A node was pressed, anywhere that is not a control of it: it is held, to be moved or picked.
    fn hold(&mut self, id: &str, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let world = self.world(at);
        if let Some(node) = self.doc.as_ref().and_then(|doc| doc.node(id)) {
            let grab = (world.0 - node.x, world.1 - node.y);
            self.gesture = Gesture::Move { id: id.to_owned(), grab, pressed: at, moved: false };
        }
    }

    /// The button was let go. A node that was held is picked, and stays where it was moved to. A connection
    /// being drawn ends when it was let go `inside` the canvas; let go outside it, it is dropped.
    fn finish_gesture(&mut self, inside: bool, window: &mut Window, cx: &mut Context<Self>) {
        match std::mem::replace(&mut self.gesture, Gesture::None) {
            Gesture::None => return,
            Gesture::Move { id, moved, .. } => {
                if moved && let Some(node) = self.doc.as_ref().and_then(|doc| doc.node(&id)) {
                    cx.emit(CanvasEvent::Moved { id: id.clone(), x: node.x, y: node.y });
                }
                // Picked when let go and not when pressed: picking opens the node's controls, which moves
                // what is under the pointer, and a click is a press and a release on the same thing.
                self.select(Some(id), cx);
            }
            Gesture::Resize { id, .. } => {
                if let Some(node) = self.doc.as_ref().and_then(|doc| doc.node(&id)) {
                    cx.emit(CanvasEvent::Resized { id: id.clone(), width: node.width, height: node.height });
                }
            }
            Gesture::Connect { from, port, to } if inside => self.end_connection(from, port, None, to, window, cx),
            Gesture::Pan { .. } | Gesture::Connect { .. } => {}
        }
        cx.notify();
    }

    /// The pointer was let go over a node, or over one of its inputs: a connection being drawn ends there.
    fn released_on(&mut self, id: &str, input: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.gesture, Gesture::Connect { .. }) {
            return;
        }
        let Gesture::Connect { from, port, to } = std::mem::replace(&mut self.gesture, Gesture::None) else { return };
        self.end_connection(from, port, Some((id.to_owned(), input.map(str::to_owned))), to, window, cx);
    }

    /// A connection drawn from `port` of `from` was let go at `at`, `over` a node or an input of one if it
    /// was over any. It ends at the port that took it, else at what it is over; let go over nothing, it asks
    /// for the node it is to run.
    fn end_connection(
        &mut self,
        from: String,
        port: String,
        over: Option<(String, Option<String>)>,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let taken = self.snap(&from, at).map(|(node, input, _)| (node, input));
        // A node that only holds a value runs nothing after it.
        let runs = self.doc.as_ref().and_then(|doc| doc.node(&from)).is_some_and(|node| node.kind != NodeKind::Data);
        match taken.or(over) {
            Some((to, _)) if to == from => {}
            Some((to, Some(input))) => cx.emit(CanvasEvent::Wire { from, port, to, input }),
            Some((to, None)) => cx.emit(CanvasEvent::Connect { from, port, to }),
            None if runs => self.open_picker(at, Some((from, port)), window, cx),
            None => {}
        }
        cx.notify();
    }

    /// Open the list a node to add is chosen from, at `at` in the window. The node chosen is put there, and
    /// runs after `link` when it was asked for by a connection.
    fn open_picker(
        &mut self,
        at: Point<Pixels>,
        link: Option<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.doc.is_none() || self.run.as_ref().is_some_and(RunView::is_running) {
            return;
        }
        // The port a node is run by is in its header, a little under its top.
        let world = self.world(at);
        let place = (world.0.round(), (world.1 - 18.0).round());
        let canvas = cx.weak_entity();
        let menu = PopupMenu::build(window, cx, |menu, _, _| kinds_menu(menu, canvas, Some(place), link.clone()));
        let dismissed = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.picker = None;
            this.focus.focus(window, cx);
            cx.notify();
        });
        menu.focus_handle(cx).focus(window, cx);
        self.picker = Some(Picker { menu, at, link, _dismissed: dismissed });
        cx.notify();
    }

    /// The empty canvas was pressed: a connection under the pointer is picked, else the canvas is held to
    /// be moved.
    fn pressed(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let near = self.edge_near((f32::from(event.position.x), f32::from(event.position.y)));
        self.select(None, cx);
        match near {
            Some(id) => self.selected_edge = Some(id),
            None => self.gesture = Gesture::Pan { last: event.position },
        }
    }

    fn key_pressed(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A key typed into a field of a node is that field's.
        if !self.focus.is_focused(window) {
            return;
        }
        match (event.keystroke.key.as_str(), event.keystroke.modifiers.secondary()) {
            ("delete" | "backspace", false) => {
                if let Some(id) = self.selected_edge.take() {
                    cx.emit(CanvasEvent::Disconnect(id));
                } else if let Some(id) = self.selected.clone() {
                    cx.emit(CanvasEvent::Delete(id));
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// What `node` handed on by `port`, as far as is known without running it: what its last run did, or
    /// what a node that holds a value holds.
    fn sample_of(&self, node_id: &str, port: &str) -> Option<Value> {
        let doc = self.doc.as_ref()?;
        let node = doc.node(node_id)?;
        let ran = self.run.as_ref().and_then(|run| run.nodes.get(node_id)).and_then(|run| run.output(port));
        match (ran, node.kind) {
            (Some(output), _) => Some(output.clone()),
            (None, NodeKind::Data | NodeKind::Sample) => Some(node.value.clone()),
            (None, NodeKind::Start) => Some(doc.default_inputs()),
            _ => None,
        }
    }

    fn toggle(&self, cx: &mut Context<Self>) -> Toggle {
        let this = cx.weak_entity();
        Rc::new(move |key, _, cx| {
            this.update(cx, |this, cx| {
                flip(&mut this.open, key);
                cx.notify();
            })
            .ok();
        })
    }

    fn tree(&self, scope: String, value: &Value, pick: Option<Pick>, cx: &mut Context<Self>) -> AnyElement {
        json_tree(&scope, value, &self.open, self.toggle(cx), pick, cx).into_any_element()
    }

    /// The dot of a port. It notes where it is each time it is laid out, which is where its connections end.
    fn port(&self, key: String, top: f32, right: bool, color: Hsla, cx: &Context<Self>) -> gpui_kit::Div {
        let anchors = self.anchors.clone();
        let note = canvas(
            move |bounds: Bounds<Pixels>, _, _| {
                anchors.borrow_mut().insert(key, bounds.center());
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .absolute()
            .top(rems(top))
            .when(right, |this| this.right(rems(-0.375)))
            .when(!right, |this| this.left(rems(-0.375)))
            .size(rems(0.75))
            .rounded_full()
            .border_1()
            .border_color(cx.theme().muted_foreground)
            .bg(color)
            .child(note)
    }

    fn render_status(run: Option<&NodeRun>, problem: bool, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let label = |text: &'static str, color: Hsla, icon: AnyElement| {
            h_flex().flex_none().gap_1().text_color(color).child(icon).child(text).into_any_element()
        };
        let icon = |name: IconName, color: Hsla| Icon::new(name).xsmall().text_color(color).into_any_element();
        match run.map_or("idle", |run| run.status.as_str()) {
            "running" => label(t("flows.status.running"), theme.primary, Spinner::new().xsmall().into_any_element()),
            "listening" => label(t("flows.status.connected"), theme.info, Spinner::new().xsmall().into_any_element()),
            "succeeded" => label(t("flows.status.done"), theme.success, icon(IconName::CircleCheck, theme.success)),
            "failed" => label(t("flows.status.failed"), theme.danger, icon(IconName::CircleX, theme.danger)),
            "stopped" => {
                label(t("flows.status.stopped"), theme.muted_foreground, icon(IconName::Minus, theme.muted_foreground))
            }
            _ if problem => label(t("flows.status.fix"), theme.warning, icon(IconName::TriangleAlert, theme.warning)),
            _ => div().into_any_element(),
        }
    }

    /// What a node does, in a line or two under its name.
    fn summary(&self, node: &FlowNode, info: Option<&NodeInfo>, run: Option<&NodeRun>) -> Vec<String> {
        let request = || {
            let Some(info) = info.filter(|info| info.request_name.is_some()) else {
                return t("flows.summary.no_request").to_owned();
            };
            let address = info.address.clone().or_else(|| info.request_name.clone()).unwrap_or_default();
            match &info.method {
                Some(method) => format!("{method} {address}"),
                None => address,
            }
        };
        // What a loop node has done so far, once it has run.
        let done = || {
            run.filter(|run| run.status != "idle")
                .map(|run| t_with("flows.loop.done", &[("count", &run.handed_on)]))
                .unwrap_or_default()
        };
        match node.kind {
            NodeKind::Start => vec![match self.doc.as_ref().map_or(0, |doc| doc.inputs.len()) {
                0 => t("flows.summary.start_none").to_owned(),
                1 => t_with("flows.summary.start_one", &[("count", &1)]),
                count => t_with("flows.summary.start_other", &[("count", &count)]),
            }],
            NodeKind::Request => vec![request()],
            NodeKind::Stream => {
                let mut lines = vec![request()];
                if let Some(run) = run.filter(|run| run.status != "idle") {
                    let state = if run.status == "listening" {
                        t("flows.status.connected")
                    } else {
                        t("flows.status.disconnected")
                    };
                    lines.push(t_with("flows.stream.messages", &[("state", &state), ("count", &run.handed_on)]));
                }
                lines
            }
            NodeKind::Mcp => {
                let what =
                    if node.target.is_empty() { node.call.clone() } else { format!("{} {}", node.call, node.target) };
                vec![request(), what]
            }
            NodeKind::Condition => node
                .branches
                .iter()
                .map(|branch| {
                    let rules: Vec<String> = branch.rules.iter().map(rule_summary).collect();
                    let joined = rules.join(if branch.matching == "any" {
                        t("flows.condition.join_any")
                    } else {
                        t("flows.condition.join_all")
                    });
                    let name = if branch.name.is_empty() { &branch.id } else { &branch.name };
                    t_with("flows.condition.branch", &[("name", name), ("rules", &joined)])
                })
                .collect(),
            NodeKind::Validate => node.rules.iter().map(rule_summary).collect(),
            NodeKind::Evaluate | NodeKind::If => vec![node.expression.clone()],
            NodeKind::Delay => vec![t_with("flows.delay.waits", &[("ms", &node.delay_ms)])],
            NodeKind::Mail => {
                let ms = if node.timeout_ms == 0 { 30_000 } else { node.timeout_ms };
                vec![t_with("flows.mail.waits", &[("ms", &ms)])]
            }
            NodeKind::Or => vec![t("flows.or.summary").to_owned()],
            NodeKind::Repeat => vec![t_with("flows.repeat.times", &[("count", &node.count), ("done", &done())])],
            NodeKind::ForEach => {
                let list = if node.path.is_empty() { "input" } else { node.path.as_str() };
                vec![t_with("flows.for_each.summary", &[("list", &list), ("done", &done())])]
            }
            NodeKind::Data => vec![one_line(&node.value)],
            NodeKind::Display | NodeKind::Sample => Vec::new(),
        }
    }

    /// What a display node shows of what it was handed. More than fits is scrolled, in the height the node
    /// was given by its corner or the one it has by itself.
    fn render_shown(&self, node: &FlowNode, run: Option<&NodeRun>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let shown_id = ElementId::Name(format!("flow-shown-{}", node.id).into());
        let muted = |text: &'static str| div().px_3().pb_2().text_color(theme.muted_foreground).child(text);
        let shown = match run.and_then(|run| run.output("out")) {
            Some(value) => self.render_value(node, value, shown_id, cx),
            None => muted(t("flows.display.empty")).id(shown_id),
        };
        let (heights, id) = (self.heights.clone(), node.id.clone());
        let note = canvas(
            move |bounds: Bounds<Pixels>, _, _| {
                heights.borrow_mut().insert(id, f32::from(bounds.size.height));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let height = rems(node.height / REM);
        shown.when(node.height > 0.0, |this| this.h(height).max_h(height)).child(note).into_any_element()
    }

    fn render_value(
        &self,
        node: &FlowNode,
        value: &Value,
        shown_id: ElementId,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let muted = |text: String| div().px_3().pb_2().text_color(theme.muted_foreground).child(text);
        let rows = value.as_array().filter(|rows| node.format == "table" && !rows.is_empty());
        match (node.format.as_str(), rows) {
            ("text", _) => {
                div().id(shown_id).mx_3().mb_2().max_h(rems(10.0)).overflow_y_scroll().child(one_line(value))
            }
            ("table", Some(rows)) => {
                // The columns are the first row's keys; a row that is not an object is one cell.
                let columns: Vec<String> =
                    rows[0].as_object().map(|first| first.keys().take(4).cloned().collect()).unwrap_or_default();
                let cell = |text: String| div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(text);
                let line =
                    |cells: Vec<String>| h_flex().gap_2().px_3().h(rems(1.375)).children(cells.into_iter().map(cell));
                let body = rows.iter().take(20).map(|row| match columns.is_empty() {
                    true => line(vec![one_line(row)]),
                    false => line(columns.iter().map(|column| one_line(&row[column.as_str()])).collect()),
                });
                v_flex()
                    .id(shown_id)
                    .pb_2()
                    .max_h(rems(14.0))
                    .overflow_y_scroll()
                    .when(!columns.is_empty(), |this| {
                        this.child(line(columns.clone()).text_color(theme.muted_foreground))
                    })
                    .children(body)
                    .when(rows.len() > 20, |this| this.child(muted(t_with("flows.json.more_rows", &[("count", &20)]))))
            }
            _ => json_tree(&format!("{}|shown", node.id), value, &self.open, self.toggle(cx), None, cx),
        }
    }

    /// One input of a node: its name, where its value comes from, and the dot a value is wired into.
    fn render_input(
        &self,
        node: &FlowNode,
        input: &InputInfo,
        run: Option<&NodeRun>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let wire = input.from.as_ref();
        // What it resolved to in the run shown, else what it resolves to as the flow stands.
        let resolved = run.and_then(|run| run.inputs.get(&input.id)).filter(|_| wire.is_none());
        let value = match wire {
            Some(from) if from.path.is_empty() => t_with("flows.input.wired_all", &[("node", &from.node_name)]),
            Some(from) => {
                t_with("flows.input.wired_path", &[("node", &from.node_name), ("path", &path_label(&from.path))])
            }
            None => one_line(resolved.unwrap_or(&input.value)),
        };
        let badge = match input.source.as_str() {
            "wired" => Tag::info().small().child(t("flows.badge.wired")),
            "env" => Tag::secondary().small().child(t("flows.badge.env")),
            "static" => Tag::secondary().small().child(t("flows.badge.typed")),
            "input" => Tag::secondary().small().child(t("flows.badge.run_input")),
            _ => Tag::warning().small().child(t("flows.badge.unset")),
        };
        let (drop_node, drop_input) = (node.id.clone(), input.id.clone());
        let edge_id = wire.map(|from| from.edge_id.clone());
        let choosing = edge_id.is_some() && self.choosing == edge_id;
        let row_id = ElementId::Name(format!("flow-input-{}-{}", node.id, input.id).into());
        // An input that is typed in is typed in its own row, while its node is picked and nothing is wired
        // into it.
        let picked = self.selected.as_deref() == Some(node.id.as_str());
        let field = (picked && wire.is_none()).then(|| self.editor.read(cx).input_control(&input.id)).flatten();
        let typed_here = field.is_some();
        let shown = match field {
            Some(field) => div().flex_1().min_w_0().child(field),
            None => div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_color(theme.muted_foreground)
                .child(value),
        };
        let row = h_flex()
            .id(row_id)
            .relative()
            .h(rems(1.5))
            .px_3()
            .gap_2()
            .child(self.port(anchor_in(&node.id, &input.id), 0.375, false, theme.background, cx))
            .child(div().flex_none().font_family(theme.mono_font_family.clone()).child(input.name.clone()))
            .child(shown)
            // A field says for itself that the value is typed in.
            .when(!typed_here, |this| this.child(badge))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if matches!(this.gesture, Gesture::Connect { .. }) {
                        this.released_on(&drop_node, Some(&drop_input), window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .when_some(edge_id.clone(), |this, edge_id| {
                let (open_id, remove_id) = (edge_id.clone(), edge_id);
                // GPUI's own tooltip, not the button's: the kit's still shows if the button is gone before its
                // delay is up, and stays.
                this.child(
                    div()
                        .id(ElementId::Name(format!("flow-unwire-tip-{remove_id}").into()))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(t("flows.input.remove_wire")).build(window, cx)
                        })
                        .child(
                            Button::new(ElementId::Name(format!("flow-unwire-{remove_id}").into()))
                                .xsmall()
                                .ghost()
                                .icon(IconName::Close)
                                .on_click(
                                    cx.listener(move |_, _, _, cx| cx.emit(CanvasEvent::Disconnect(remove_id.clone()))),
                                ),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.choosing = (this.choosing.as_deref() != Some(open_id.as_str())).then(|| open_id.clone());
                    cx.notify();
                }))
            });
        if !choosing {
            return row.into_any_element();
        }
        // The part of the source's output that is taken is chosen by clicking it.
        let from = wire.expect("a wire is being chosen for");
        let port = self.doc.as_ref().and_then(|doc| doc.edges.iter().find(|edge| edge.id == from.edge_id));
        let sample = port.and_then(|edge| self.sample_of(&from.node_id, &edge.port));
        let this = cx.weak_entity();
        let picked_edge = from.edge_id.clone();
        let whole_edge = from.edge_id.clone();
        let pick: Pick = Rc::new(move |path, _, cx| {
            this.update(cx, |this, cx| {
                this.choosing = None;
                cx.emit(CanvasEvent::WirePath { edge_id: picked_edge.clone(), path });
            })
            .ok();
        });
        let chooser = match sample {
            Some(sample) => self.tree(format!("{}|wire|{}", node.id, input.id), &sample, Some(pick), cx),
            None => div()
                .px_2()
                .py_1()
                .text_color(theme.muted_foreground)
                .child(t_with("flows.input.run_once", &[("node", &from.node_name)]))
                .into_any_element(),
        };
        v_flex()
            .child(row)
            .child(
                v_flex()
                    .mx_3()
                    .mb_1()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .px_2()
                            .pt_1()
                            .justify_between()
                            .child(div().text_color(theme.muted_foreground).child(t("flows.input.click_part")))
                            .child(
                                Button::new(ElementId::Name(format!("flow-wire-all-{whole_edge}").into()))
                                    .xsmall()
                                    .ghost()
                                    .label(t("flows.common.all_of_it"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.choosing = None;
                                        cx.emit(CanvasEvent::WirePath {
                                            edge_id: whole_edge.clone(),
                                            path: Vec::new(),
                                        });
                                    })),
                            ),
                    )
                    .child(chooser),
            )
            .into_any_element()
    }

    /// The switches a request node carries on itself. Only a REST request has scripts and tests.
    fn render_switches(&self, node: &FlowNode, cx: &mut Context<Self>) -> Option<AnyElement> {
        if node.kind != NodeKind::Request || node.request_kind != "requests" {
            return None;
        }
        let switch = |name: &'static str,
                      label: &'static str,
                      on: bool,
                      set: fn(&mut FlowNode, bool),
                      cx: &mut Context<Self>| {
            let node = node.clone();
            Checkbox::new(ElementId::Name(format!("flow-{name}-{}", node.id).into()))
                .small()
                .checked(on)
                .label(label)
                .on_click(cx.listener(move |_, checked: &bool, _, cx| {
                    let mut node = node.clone();
                    set(&mut node, *checked);
                    cx.emit(CanvasEvent::Edited(Box::new(node)));
                }))
        };
        Some(
            h_flex()
                .flex_wrap()
                .px_3()
                .py_1()
                .gap_x_3()
                .gap_y_1()
                .child(switch(
                    "scripts",
                    t("flows.request.execute_scripts"),
                    node.execute_scripts,
                    |node, on| node.execute_scripts = on,
                    cx,
                ))
                .child(switch(
                    "tests",
                    t("flows.request.execute_tests"),
                    node.execute_tests,
                    |node, on| node.execute_tests = on,
                    cx,
                ))
                .child(switch(
                    "ignore-tests",
                    t("flows.request.ignore_test_errors"),
                    node.ignore_test_errors,
                    |node, on| node.ignore_test_errors = on,
                    cx,
                ))
                .into_any_element(),
        )
    }

    /// The node under a node: what it was handed, what it produced, and its tests, each closed until asked for.
    fn render_details(
        &self,
        node: &FlowNode,
        info: Option<&NodeInfo>,
        run: Option<&NodeRun>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let open = self.details.get(&node.id).copied();
        let tests = run.map(|run| run.tests.as_slice()).unwrap_or(&[]);
        let passed = tests.iter().filter(|test| test.passed).count();
        let tab = |detail: Detail, label: String, cx: &mut Context<Self>| {
            let id = node.id.clone();
            Button::new(ElementId::Name(format!("flow-detail-{label}-{}", node.id).into()))
                .xsmall()
                .ghost()
                .selected(open == Some(detail))
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.details.get(&id) == Some(&detail) {
                        this.details.remove(&id);
                    } else {
                        this.details.insert(id.clone(), detail);
                    }
                    cx.notify();
                }))
        };
        let tests_label = if tests.is_empty() {
            t("flows.details.tests").to_owned()
        } else {
            t_with("flows.details.tests_count", &[("passed", &passed), ("total", &tests.len())])
        };
        let muted =
            |text: String| div().px_2().py_1().text_color(theme.muted_foreground).child(text).into_any_element();
        let heading = |text: String, note: String| {
            h_flex()
                .px_2()
                .pt_1()
                .gap_2()
                .child(div().flex_none().font_medium().child(text))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(theme.muted_foreground)
                        .child(note),
                )
                .into_any_element()
        };
        let mut content: Vec<AnyElement> = Vec::new();
        match open {
            None => {}
            Some(Detail::Input) => {
                if node.kind.is_run() {
                    content
                        .push(heading(t("flows.details.handed").to_owned(), t("flows.details.handed_note").to_owned()));
                    content.push(match run.filter(|run| run.runs > 0) {
                        Some(run) => self.tree(format!("{}|handed", node.id), &run.input, None, cx),
                        None => muted(t("flows.details.not_run").to_owned()),
                    });
                }
                for input in info.map(|info| info.inputs.as_slice()).unwrap_or(&[]) {
                    let source = match (&input.from, input.source.as_str()) {
                        (Some(from), _) if from.path.is_empty() => {
                            t_with("flows.details.wired_from", &[("node", &from.node_name)])
                        }
                        (Some(from), _) => t_with(
                            "flows.details.wired_from_path",
                            &[("node", &from.node_name), ("path", &path_label(&from.path))],
                        ),
                        (None, "env") => t("flows.details.from_env").to_owned(),
                        (None, "static") => t("flows.details.typed_in").to_owned(),
                        (None, "input") => t("flows.details.run_input").to_owned(),
                        _ => t("flows.details.no_value").to_owned(),
                    };
                    content.push(heading(input.name.clone(), source));
                    let resolved = run.and_then(|run| run.inputs.get(&input.id)).unwrap_or(&input.value);
                    content.push(match (resolved.is_null(), input.from.is_some()) {
                        (true, true) => muted(t("flows.details.value_later").to_owned()),
                        _ => self.tree(format!("{}|input|{}", node.id, input.id), resolved, None, cx),
                    });
                }
                if content.is_empty() {
                    content.push(muted(t("flows.details.takes_nothing").to_owned()));
                }
            }
            Some(Detail::Output) => {
                for (port, label) in info.map(|info| info.ports.as_slice()).unwrap_or(&[]) {
                    let left = run.is_some_and(|run| run.port.as_deref() == Some(port.as_str()));
                    content.push(heading(
                        label.clone(),
                        if left { t("flows.details.left_last").to_owned() } else { String::new() },
                    ));
                    content.push(match self.sample_of(&node.id, port) {
                        Some(value) => self.tree(format!("{}|output|{port}", node.id), &value, None, cx),
                        None => muted(t("flows.details.nothing_left").to_owned()),
                    });
                }
            }
            Some(Detail::Tests) => {
                for test in tests {
                    let (icon, color) = match test.passed {
                        true => (IconName::CircleCheck, theme.success),
                        false => (IconName::CircleX, theme.danger),
                    };
                    content.push(
                        h_flex()
                            .px_2()
                            .py_0p5()
                            .gap_2()
                            .items_start()
                            .child(Icon::new(icon).xsmall().text_color(color).flex_none())
                            .child(v_flex().min_w_0().child(test.name.clone()).children(
                                test.error.clone().map(|error| div().text_color(theme.muted_foreground).child(error)),
                            ))
                            .into_any_element(),
                    );
                }
                if tests.is_empty() {
                    content.push(muted(match node.kind {
                        NodeKind::Validate => t("flows.details.checks_later").to_owned(),
                        NodeKind::Request => t("flows.details.tests_later").to_owned(),
                        _ => t("flows.details.no_tests").to_owned(),
                    }));
                }
            }
        }
        v_flex()
            .mx_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .child(
                h_flex()
                    .px_1()
                    .py_0p5()
                    .gap_1()
                    .child(tab(Detail::Input, t("flows.common.input").to_owned(), cx))
                    .child(tab(Detail::Output, t("flows.details.output").to_owned(), cx))
                    .child(tab(Detail::Tests, tests_label, cx)),
            )
            .when(!content.is_empty(), |this| {
                this.child(v_flex().pb_1().border_t_1().border_color(theme.border).children(content))
            })
            .into_any_element()
    }

    fn render_node(&self, node: &FlowNode, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let info = self.described.nodes.get(&node.id);
        let run = self.run.as_ref().and_then(|run| run.nodes.get(&node.id));
        let selected = self.selected.as_deref() == Some(node.id.as_str());
        let problem = self.described.has_error(&node.id);
        let status = run.map_or("idle", |run| run.status.as_str());
        // The node that is running pulses; one that is done is tinted by how it ended.
        let beat = 0.55 + 0.45 * (self.started.elapsed().as_secs_f32() * 4.0).sin().abs();
        let (border, tint) = match status {
            "running" => (theme.primary.opacity(beat), Some(theme.primary.opacity(0.10))),
            "listening" => (theme.info.opacity(beat), Some(theme.info.opacity(0.10))),
            "succeeded" => (theme.success, Some(theme.success.opacity(0.10))),
            "failed" => (theme.danger, Some(theme.danger.opacity(0.10))),
            _ if selected => (theme.ring, None),
            _ => (theme.border, None),
        };
        let id = node.id.clone();
        let named = |part: &str| ElementId::Name(format!("flow-node-{part}-{id}").into());
        let (hold_id, drop_id) = (id.clone(), id.clone());

        let title: AnyElement = match selected {
            true => div()
                .flex_1()
                .min_w_0()
                .child(Input::new(self.editor.read(cx).name_field()).xsmall())
                .into_any_element(),
            false => div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_medium()
                .overflow_hidden()
                .text_ellipsis()
                .child(SharedString::from(node.title().to_owned()))
                .into_any_element(),
        };
        let problems = self.described.problems_of(&node.id);
        let header = h_flex()
            .id(named("header"))
            .relative()
            .h(rems(2.25))
            .px_3()
            .gap_2()
            .rounded_t(theme.radius)
            .when_some(tint, |this, tint| this.bg(tint))
            .when(!problems.is_empty(), |this| {
                this.tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(problems.clone()).build(window, cx)
                })
            })
            .when(node.kind.is_run(), |this| {
                this.child(self.port(anchor_in(&node.id, ""), 0.75, false, theme.background, cx))
            })
            .child(kind_icon(node.kind).small().text_color(theme.muted_foreground).flex_none())
            .child(title)
            .when(run.is_some_and(|run| run.runs > 1), |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(format!("×{}", run.map_or(0, |run| run.runs))),
                )
            })
            .child(Self::render_status(run, problem, cx));

        let lines =
            self.summary(node, info, run).into_iter().filter(|line| !line.is_empty()).map(|line| {
                div().px_3().overflow_hidden().text_ellipsis().text_color(theme.muted_foreground).child(line)
            });
        let summary = v_flex().gap_1().children(lines);
        let body: Option<AnyElement> = match node.kind {
            NodeKind::Display => Some(self.render_shown(node, run, cx)),
            NodeKind::Sample => Some(self.tree(format!("{}|sample", node.id), &node.value, None, cx)),
            _ => None,
        };
        let failure =
            run.and_then(|run| run.error.clone()).map(|error| div().px_3().text_color(theme.danger).child(error));
        let inputs: Vec<AnyElement> = info
            .map(|info| info.inputs.iter().map(|input| self.render_input(node, input, run, cx)).collect())
            .unwrap_or_default();
        let ports = info.map(|info| info.ports.as_slice()).unwrap_or(&[]).iter().map(|(port, label)| {
            let (from, port_id) = (id.clone(), port.clone());
            let left = run.is_some_and(|run| run.port.as_deref() == Some(port.as_str()));
            let handle = self
                .port(anchor_out(&node.id, port), 0.375, true, theme.primary, cx)
                .id(ElementId::Name(format!("flow-port-{id}-{port}").into()))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(t("flows.port.tooltip")).build(window, cx)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.gesture =
                            Gesture::Connect { from: from.clone(), port: port_id.clone(), to: event.position };
                        cx.stop_propagation();
                        cx.notify();
                    }),
                );
            h_flex()
                .relative()
                .h(rems(1.5))
                .justify_end()
                .pr_3()
                .when(!left, |this| this.text_color(theme.muted_foreground))
                .child(label.clone())
                .child(handle)
        });
        let ports: Vec<_> = ports.collect();

        // The corner a display node is pulled by to make room for what it shows.
        let grip = (node.kind == NodeKind::Display).then(|| {
            let grip_id = id.clone();
            let width = if node.width > 0.0 { node.width } else { NODE_WIDTH * REM };
            div()
                .absolute()
                .bottom_0()
                .right_0()
                .cursor_nwse_resize()
                .child(Icon::new(IconName::ResizeCorner).xsmall().text_color(theme.muted_foreground))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.focus.focus(window, cx);
                        let shown = this.heights.borrow().get(&grip_id).copied().unwrap_or_default();
                        let start = (width, shown / this.zoom);
                        this.gesture = Gesture::Resize { id: grip_id.clone(), pressed: event.position, start };
                    }),
                )
        });

        let card = v_flex()
            .id(named("card"))
            .relative()
            .pb_1()
            .gap_1()
            .rounded(theme.radius)
            .border_1()
            .when(selected || matches!(status, "running" | "listening"), |this| this.border_2())
            .border_color(border)
            .bg(theme.popover)
            .shadow_sm()
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| this.released_on(&drop_id, None, window, cx)),
            )
            .child(header)
            .child(summary)
            .children(body)
            .children(failure)
            .when(selected, |this| this.child(self.editor.clone()))
            .children(inputs)
            .children(self.render_switches(node, cx))
            .children(grip)
            .children(ports);

        // What can be done with the node that is picked, above it.
        let actions = (selected && node.kind != NodeKind::Start).then(|| {
            let (copy_id, delete_id) = (id.clone(), id.clone());
            h_flex()
                .absolute()
                .top(rems(-2.0))
                .left_0()
                .gap_1()
                .p_0p5()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_sm()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new(named("duplicate"))
                        .xsmall()
                        .ghost()
                        .icon(IconName::Copy)
                        .label(t("flows.common.duplicate"))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(CanvasEvent::Duplicate(copy_id.clone())))),
                )
                .child(
                    Button::new(named("delete"))
                        .xsmall()
                        .ghost()
                        .icon(IconName::Delete)
                        .label(t("flows.common.delete"))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(CanvasEvent::Delete(delete_id.clone())))),
                )
        });

        let whole = v_flex()
            .relative()
            .w(rems(if node.width > 0.0 { node.width / REM } else { NODE_WIDTH }))
            .gap_1()
            .text_xs()
            // The wheel over a node scrolls what is in the node, and never moves the canvas under it. With
            // Ctrl it still zooms.
            .on_scroll_wheel(|event, _, cx| {
                if !event.modifiers.secondary() {
                    cx.stop_propagation();
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    // A press on a node is not a press on the canvas under it.
                    cx.stop_propagation();
                    // A button, a switch or a field of the node that was pressed has said the press is its
                    // own. Anywhere else the node is held.
                    if !window.default_prevented() {
                        this.hold(&hold_id, event.position, window, cx);
                    }
                }),
            )
            .children(actions)
            .child(card)
            .child(self.render_details(node, info, run, cx));
        div()
            .absolute()
            .left(px(node.x * self.zoom + self.pan.0))
            .top(px(node.y * self.zoom + self.pan.1))
            .child(Scaled { factor: self.zoom, child: whole.into_any_element() })
            .into_any_element()
    }

    /// The buttons that float over the canvas: adding a node, and the zoom.
    fn render_controls(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let this = cx.weak_entity();
        let running = self.run.as_ref().is_some_and(RunView::is_running);
        let add = Button::new("flow-add-node")
            .small()
            .icon(IconName::Plus)
            .label(t("flows.controls.add_node"))
            .disabled(running || self.doc.is_none())
            .tooltip(t("flows.controls.add_tooltip"))
            .dropdown_menu(move |menu, _, _| kinds_menu(menu, this.clone(), None, None));
        let zoom = h_flex()
            .gap_0p5()
            .p_0p5()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .child(
                Button::new("flow-zoom-out")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Minus)
                    .tooltip(t("flows.controls.zoom_out"))
                    .disabled(self.zoom <= ZOOM_RANGE.0)
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_by_step(false, cx))),
            )
            .child(
                Button::new("flow-zoom-reset")
                    .xsmall()
                    .ghost()
                    .label(format!("{}%", (self.zoom * 100.0).round()))
                    .tooltip(t("flows.controls.zoom_reset"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let size = this.viewport.get().size;
                        this.zoom_to(1.0, (f32::from(size.width) / 2.0, f32::from(size.height) / 2.0), cx);
                    })),
            )
            .child(
                Button::new("flow-zoom-in")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Plus)
                    .tooltip(t("flows.controls.zoom_in"))
                    .disabled(self.zoom >= ZOOM_RANGE.1)
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_by_step(true, cx))),
            );
        let stop = |cx: &mut App| cx.stop_propagation();
        vec![
            div()
                .absolute()
                .top_3()
                .left_3()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| stop(cx))
                .child(add)
                .into_any_element(),
            div()
                .absolute()
                .bottom_3()
                .right_3()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| stop(cx))
                .child(zoom)
                .into_any_element(),
        ]
    }
}

impl Render for FlowCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let edges: &[FlowEdge] = self.doc.as_ref().map(|doc| doc.edges.as_slice()).unwrap_or(&[]);
        let taken = |id: &str| self.run.as_ref().and_then(|run| run.edges.get(id)).copied().unwrap_or(0) > 0;
        let draws: Vec<EdgeDraw> = edges
            .iter()
            .map(|edge| EdgeDraw {
                id: edge.id.clone(),
                from: anchor_out(&edge.from, &edge.port),
                to: anchor_in(&edge.to, &edge.input),
                wire: edge.is_wire(),
                taken: taken(&edge.id),
                moving: self.pulses.get(&edge.id).is_some_and(|at| at.elapsed() < PULSE),
                selected: self.selected_edge.as_deref() == Some(edge.id.as_str()),
            })
            .collect();
        let busy =
            |run: &RunView| run.nodes.values().any(|node| matches!(node.status.as_str(), "running" | "listening"));
        // A connection that is being taken, and a node that is running, are drawn again every frame.
        if draws.iter().any(|draw| draw.moving) || self.run.as_ref().is_some_and(busy) {
            window.request_animation_frame();
        }
        // A connection being drawn ends at the pointer, or at the port that is near enough to take it. One
        // that asked for a node stays drawn to where the node will go until the node is chosen.
        let connecting = match (&self.gesture, &self.picker) {
            (Gesture::Connect { from, port, to }, _) => {
                Some((anchor_out(from, port), self.snap(from, *to).map_or(*to, |(_, _, port)| port)))
            }
            (_, Some(Picker { link: Some((from, port)), at, .. })) => Some((anchor_out(from, port), *at)),
            _ => None,
        };
        let origin = self.viewport.get().origin;
        // The cross that removes the connection the pointer is on, or the one that is picked, at its middle.
        let crossed = self.hover_edge.clone().or_else(|| self.selected_edge.clone()).and_then(|id| {
            let middle =
                self.drawn.borrow().iter().find(|line| line.0 == id).map(|(_, from, to)| curve(*from, *to, 0.5))?;
            Some(
                div()
                    .id("flow-edge-remove-tip")
                    .absolute()
                    .left(px(middle.0 - f32::from(origin.x) - 10.0))
                    .top(px(middle.1 - f32::from(origin.y) - 10.0))
                    .rounded_full()
                    .bg(theme.popover)
                    // GPUI's own tooltip, not the button's: the kit's still shows if the cross is gone before its
                    // delay is up, and stays.
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(t("flows.edge.remove")).build(window, cx)
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(Button::new("flow-edge-remove").xsmall().outline().icon(IconName::Close).on_click(
                        cx.listener(move |this, _, _, cx| {
                            (this.hover_edge, this.selected_edge) = (None, None);
                            cx.emit(CanvasEvent::Disconnect(id.clone()));
                            cx.notify();
                        }),
                    )),
            )
        });
        let picker = self.picker.as_ref().map(|picker| {
            deferred(anchored().position(picker.at).snap_to_window_with_margin(px(8.0)).child(picker.menu.clone()))
                .with_priority(1)
        });
        let clock = self.started.elapsed().as_secs_f32() * DOT_SPEED;
        let (zoom, viewport, anchors, drawn) =
            (self.zoom, self.viewport.clone(), self.anchors.clone(), self.drawn.clone());
        let (ring, primary, quiet) = (theme.ring, theme.primary, theme.muted_foreground.opacity(0.7));
        let (pan, grid) = (self.pan, theme.muted_foreground.opacity(0.25));
        // The ports note where they are while the nodes are laid out, before anything is painted.
        self.anchors.borrow_mut().clear();
        let connections = canvas(
            move |bounds: Bounds<Pixels>, _, _| viewport.set(bounds),
            move |bounds, _, window, _| {
                // The dotted background moves and scales with the nodes.
                // Zoomed far out, every other dot, so the grid neither turns into a haze nor costs a quad per 8px.
                let step = GRID_SPACING * zoom * if zoom < 0.6 { 2.0 } else { 1.0 };
                let dot = 1.5;
                let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let mut y = top + pan.1.rem_euclid(step);
                while y < top + height {
                    let mut x = left + pan.0.rem_euclid(step);
                    while x < left + width {
                        let corner = point(px(x - dot / 2.0), px(y - dot / 2.0));
                        let bounds = Bounds::new(corner, size(px(dot), px(dot)));
                        window.paint_quad(quad(bounds, px(dot / 2.0), grid, px(0.0), grid, BorderStyle::default()));
                        x += step;
                    }
                    y += step;
                }
                let anchors = anchors.borrow();
                let place = |key: &str| anchors.get(key).map(|at| (f32::from(at.x), f32::from(at.y)));
                let mut paint = |from: (f32, f32), to: (f32, f32), color: Hsla, dot: f32, phase: f32| {
                    let dot = dot * zoom.max(0.7);
                    for (x, y) in dots(from, to, DOT_SPACING, phase) {
                        let corner = point(px(x - dot / 2.0), px(y - dot / 2.0));
                        let bounds = Bounds::new(corner, size(px(dot), px(dot)));
                        window.paint_quad(quad(bounds, px(dot / 2.0), color, px(0.0), color, BorderStyle::default()));
                    }
                };
                let mut lines = Vec::new();
                for draw in &draws {
                    let (Some(from), Some(to)) = (place(&draw.from), place(&draw.to)) else { continue };
                    // A wire carries a value and is drawn lighter than a connection that runs a node.
                    let size = if draw.wire { 2.0 } else { 3.0 };
                    let (color, dot, phase) = match (draw.selected, draw.moving, draw.taken) {
                        (true, ..) => (ring, size + 1.5, 0.0),
                        (_, true, _) => (primary, size + 1.0, clock),
                        (_, _, true) => (primary.opacity(0.6), size, 0.0),
                        _ => (quiet, size - 0.5, 0.0),
                    };
                    paint(from, to, color, dot, phase);
                    lines.push((draw.id.clone(), from, to));
                }
                if let Some((from, to)) = &connecting
                    && let Some(from) = place(from)
                {
                    paint(from, (f32::from(to.x), f32::from(to.y)), primary, 3.0, 0.0);
                }
                *drawn.borrow_mut() = lines;
            },
        )
        .absolute()
        .size_full();
        let mut nodes: Vec<FlowNode> = self.doc.as_ref().map(|doc| doc.nodes.clone()).unwrap_or_default();
        // The node that is picked is drawn last, over a node it has grown onto, and one with a view open
        // under it over the ones without.
        nodes.sort_by_key(|node| {
            (self.selected.as_deref() == Some(node.id.as_str()), self.details.contains_key(&node.id))
        });
        let nodes: Vec<AnyElement> = nodes.iter().map(|node| self.render_node(node, cx)).collect();

        div()
            .id("flow-canvas")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.muted.opacity(0.35))
            .key_context("FlowCanvas")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.key_pressed(event, window, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| this.pressed(event, window, cx)),
            )
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, window, cx| this.pointer_moved(event, window, cx)),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, window, cx| this.finish_gesture(true, window, cx)))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.finish_gesture(false, window, cx)),
            )
            // Ctrl+D, which elsewhere copies the open item, copies the picked node here. A key typed into a field of
            // a node is that field's, so nothing is copied then.
            .on_action(cx.listener(|this, _: &DuplicateNode, window, cx| {
                if this.focus.is_focused(window)
                    && let Some(id) = this.selected.clone()
                {
                    cx.emit(CanvasEvent::Duplicate(id));
                }
            }))
            // Ctrl+N, which elsewhere starts a request, asks here for a node, where the pointer is.
            .on_action(cx.listener(|this, _: &NewRestRequest, window, cx| {
                let viewport = this.viewport.get();
                let at = this.pointer.filter(|at| viewport.contains(at)).unwrap_or_else(|| viewport.center());
                this.open_picker(at, None, window, cx);
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height());
                // With Ctrl (Command on a Mac) the wheel zooms about the pointer; without, it moves the canvas.
                if event.modifiers.secondary() {
                    let origin = this.viewport.get().origin;
                    let around = (f32::from(event.position.x - origin.x), f32::from(event.position.y - origin.y));
                    if delta.y != px(0.0) {
                        this.zoom_to(zoom_step(this.zoom, delta.y > px(0.0)), around, cx);
                    }
                    return;
                }
                this.pan.0 += f32::from(delta.x);
                this.pan.1 += f32::from(delta.y);
                cx.notify();
            }))
            .child(connections)
            .children(nodes)
            .children(crossed)
            .children(self.render_controls(cx))
            .children(picker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_bends_between_its_ends_and_is_picked_by_a_click_near_it() {
        assert_eq!(curve((0.0, 0.0), (100.0, 50.0), 0.0), (0.0, 0.0));
        assert_eq!(curve((0.0, 0.0), (100.0, 50.0), 1.0), (100.0, 50.0));
        assert!(distance_to((0.0, 0.0), (200.0, 0.0), (100.0, 3.0)) <= PICK_DISTANCE);
        assert!(distance_to((0.0, 0.0), (200.0, 0.0), (100.0, 40.0)) > PICK_DISTANCE);
        assert_eq!((anchor_in("a", ""), anchor_in("a", "var:id")), ("in|a".to_owned(), "in|a|var:id".to_owned()));
        // A connection that a port takes ends at what the port is of; where one leaves takes none.
        assert_eq!(target_of(&anchor_in("a", "")), Some(("a", None)));
        assert_eq!(target_of(&anchor_in("a", "var:id")), Some(("a", Some("var:id"))));
        assert_eq!(target_of(&anchor_out("a", "success")), None);
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

    #[test]
    fn the_zoom_steps_within_its_range_and_comes_back_to_100_percent() {
        let out = zoom_step(1.0, false);
        assert!(out < 1.0);
        assert_eq!(zoom_step(out, true), 1.0);
        assert_eq!(zoom_step(ZOOM_RANGE.1, true), ZOOM_RANGE.1);
        assert_eq!(zoom_step(ZOOM_RANGE.0, false), ZOOM_RANGE.0);
    }
}
