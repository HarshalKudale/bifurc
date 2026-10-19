//! The engine's event log: it numbers every wire event, keeps a bounded recent history so a reconnecting
//! client can be caught up, and fans events out to the sessions subscribed to them.
//!
//! An event name is retained (numbered and buffered) from the first time any session subscribes to it.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::Value;

use crate::{
    kernel::{EngineEventBus, ListenerId},
    protocol,
};

pub const DEFAULT_MAX_EVENTS: usize = 1000;
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(5 * 60);

/// One event as a session receives it.
#[derive(Debug, Clone)]
pub struct EngineEvent {
    /// The wire name (`event.log.entry`).
    pub event: String,
    pub seq: u64,
    pub payload: Value,
}

pub enum ReplayOutcome {
    Replay(Vec<EngineEvent>),
    Resync(String),
}

type Subscriber = Arc<dyn Fn(&EngineEvent) + Send + Sync>;

#[derive(Default)]
struct State {
    buffer: VecDeque<(EngineEvent, Instant)>,
    seq: u64,
    closed: bool,
    retained: HashSet<String>,
    subscribers: HashMap<String, Vec<(u64, Subscriber)>>,
}

pub struct EventLog {
    state: Arc<Mutex<State>>,
    bus: &'static EngineEventBus,
    listener: ListenerId,
    next_subscriber: AtomicU64,
    max_events: usize,
    max_age: Duration,
}

/// Returned by [`EventLog::subscribe`]; pass to [`EventLog::unsubscribe`].
#[derive(Debug, Clone)]
pub struct SubscriptionId {
    id: u64,
    events: Vec<String>,
}

impl EventLog {
    pub fn new(bus: &'static EngineEventBus) -> Arc<Self> {
        let state = Arc::new(Mutex::new(State::default()));
        let sink = state.clone();
        let (max_events, max_age) = (DEFAULT_MAX_EVENTS, DEFAULT_MAX_AGE);
        let listener = bus.on_any(move |bus_name, payload| {
            let Some(wire) = protocol::wire_name_for(bus_name) else { return };
            let (event, targets) = {
                let mut state = sink.lock();
                if state.closed || !state.retained.contains(wire) {
                    return;
                }
                state.seq += 1;
                let event = EngineEvent { event: wire.to_string(), seq: state.seq, payload: payload.clone() };
                state.buffer.push_back((event.clone(), Instant::now()));
                prune(&mut state, max_events, max_age);
                let targets: Vec<Subscriber> = state
                    .subscribers
                    .get(wire)
                    .map(|subs| subs.iter().map(|(_, cb)| cb.clone()).collect())
                    .unwrap_or_default();
                (event, targets)
            };
            for target in targets {
                target(&event);
            }
        });
        Arc::new(Self { state, bus, listener, next_subscriber: AtomicU64::new(1), max_events, max_age })
    }

    pub fn newest_seq(&self) -> u64 {
        self.state.lock().seq
    }

    /// Deliver every future `events` to `cb`. The names must be wire events (`protocol::is_wire_event`).
    pub fn subscribe(&self, events: &[String], cb: impl Fn(&EngineEvent) + Send + Sync + 'static) -> SubscriptionId {
        let id = self.next_subscriber.fetch_add(1, Ordering::Relaxed);
        let cb: Subscriber = Arc::new(cb);
        let mut state = self.state.lock();
        for wire in events {
            state.retained.insert(wire.clone());
            state.subscribers.entry(wire.clone()).or_default().push((id, cb.clone()));
        }
        SubscriptionId { id, events: events.to_vec() }
    }

    pub fn unsubscribe(&self, subscription: &SubscriptionId) {
        let mut state = self.state.lock();
        for wire in &subscription.events {
            if let Some(subs) = state.subscribers.get_mut(wire) {
                subs.retain(|(id, _)| *id != subscription.id);
            }
        }
    }

    /// The events after `last_seq`, optionally only those named in `names`, or the reason they cannot be
    /// replayed.
    pub fn replay_from(&self, last_seq: u64, names: Option<&[String]>) -> ReplayOutcome {
        let mut state = self.state.lock();
        prune(&mut state, self.max_events, self.max_age);
        let Some(oldest) = state.buffer.front().map(|(event, _)| event.seq) else {
            return if last_seq == state.seq {
                ReplayOutcome::Replay(Vec::new())
            } else {
                ReplayOutcome::Resync(format!(
                    "no events are retained, so a client at seq {last_seq} cannot be caught up"
                ))
            };
        };
        if last_seq + 1 < oldest {
            return ReplayOutcome::Resync(format!(
                "seq {last_seq} is older than the oldest retained event ({oldest}); the events in between have \
                 been dropped and cannot be replayed"
            ));
        }
        if last_seq > state.seq {
            return ReplayOutcome::Resync(format!("seq {last_seq} is ahead of this engine's newest ({})", state.seq));
        }
        ReplayOutcome::Replay(
            state
                .buffer
                .iter()
                .map(|(event, _)| event)
                .filter(|event| event.seq > last_seq && names.is_none_or(|names| names.contains(&event.event)))
                .cloned()
                .collect(),
        )
    }

    pub fn close(&self) {
        let mut state = self.state.lock();
        if state.closed {
            return;
        }
        state.closed = true;
        state.retained.clear();
        state.subscribers.clear();
        drop(state);
        self.bus.off(self.listener);
    }
}

fn prune(state: &mut State, max_events: usize, max_age: Duration) {
    let now = Instant::now();
    while state.buffer.front().is_some_and(|(_, at)| now.duration_since(*at) >= max_age) {
        state.buffer.pop_front();
    }
    while state.buffer.len() > max_events {
        state.buffer.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn leaked_bus() -> &'static EngineEventBus {
        Box::leak(Box::new(EngineEventBus::new()))
    }

    #[test]
    fn numbers_only_subscribed_events_and_replays_them() {
        let bus = leaked_bus();
        let log = EventLog::new(bus);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let sub = log.subscribe(&["event.ws.status".to_string()], move |e| sink.lock().push(e.seq));

        bus.emit("log.entry", json!({}));
        bus.emit("ws.status", json!({ "n": 1 }));
        bus.emit("ws.status", json!({ "n": 2 }));
        assert_eq!(*seen.lock(), vec![1, 2]);

        match log.replay_from(1, None) {
            ReplayOutcome::Replay(events) => assert_eq!(events.len(), 1),
            ReplayOutcome::Resync(reason) => panic!("{reason}"),
        }
        assert!(matches!(log.replay_from(9, None), ReplayOutcome::Resync(_)));

        log.unsubscribe(&sub);
        bus.emit("ws.status", json!({ "n": 3 }));
        assert_eq!(*seen.lock(), vec![1, 2]);
        assert_eq!(log.newest_seq(), 3);
    }
}
