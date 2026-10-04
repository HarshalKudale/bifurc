//! What the commands' tests share: an engine that answers from a table and records what it was asked.

use std::{cell::RefCell, rc::Rc};

use serde_json::{Value, json};

use crate::{
    args::parse_args,
    commands::{Connector, Context},
    discovery::RuntimeInfo,
    error::{CliError, Result},
    session::Engine,
};

type Calls = Rc<RefCell<Vec<(String, Value)>>>;

/// One answer of the fake engine: the command it answers, or `command:kind` for the answer to that command
/// about one entity kind, and what it answers.
pub type Answer = (&'static str, Result<Value>);

struct Fake {
    info: RuntimeInfo,
    answers: Vec<Answer>,
    calls: Calls,
}

impl Engine for Fake {
    fn info(&self) -> &RuntimeInfo {
        &self.info
    }

    fn request(&mut self, command: &str, payload: Value) -> Result<Value> {
        let of_kind = format!("{command}:{}", payload.get("kind").and_then(Value::as_str).unwrap_or_default());
        self.calls.borrow_mut().push((command.to_string(), payload));
        let answer = self
            .answers
            .iter()
            .find(|(name, _)| *name == of_kind)
            .or_else(|| self.answers.iter().find(|(name, _)| *name == command));
        match answer {
            Some((_, answer)) => answer.clone(),
            None => Err(CliError::Engine { code: "UNKNOWN_COMMAND".into(), message: command.into() }),
        }
    }
}

pub fn info() -> RuntimeInfo {
    RuntimeInfo::from_json(json!({
        "pid": 5, "transport": "socket", "address": "addr", "dataDir": "/d", "engineVersion": "0.4.0"
    }))
    .unwrap()
}

pub fn config() -> Value {
    json!({
        "activeWorkspaceId": "w1",
        "activeEnvironmentId": "e2",
        "workspaces": [{"id": "w1", "name": "Default"}, {"id": "w2", "name": "Other"}],
    })
}

/// Run `f` for the arguments, with an engine that gives `answers`; returns the result and the calls made.
pub fn run(
    argv: &[&str],
    answers: Vec<Answer>,
    f: impl FnOnce(&mut Context) -> Result<String>,
) -> (Result<String>, Vec<(String, Value)>) {
    let args = parse_args(&argv.iter().map(|a| a.to_string()).collect::<Vec<_>>()).unwrap();
    let calls: Calls = Rc::default();
    let recorded = Rc::clone(&calls);
    let mut answers = Some(answers);
    let connector: Connector = Box::new(move || {
        let fake = Fake { info: info(), answers: answers.take().expect("opened once"), calls: Rc::clone(&recorded) };
        Ok(Box::new(fake) as Box<dyn Engine>)
    });
    let mut ctx = Context::new(&args, connector);
    let result = f(&mut ctx);
    let calls = calls.borrow().clone();
    (result, calls)
}

/// The verb of a command line: its second word.
pub fn sub<'a>(argv: &'a [&'a str]) -> Option<&'a str> {
    argv.get(1).copied()
}

/// The payloads of the calls of one command, in order.
pub fn payloads(calls: &[(String, Value)], command: &str) -> Vec<Value> {
    calls.iter().filter(|(name, _)| name == command).map(|(_, payload)| payload.clone()).collect()
}
