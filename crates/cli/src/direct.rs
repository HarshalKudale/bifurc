//! The verbs that are one engine command. A row of [`verbs`] says which command a verb runs and where each of
//! its arguments goes in that command's payload; [`run`] does the rest. The same row is the verb's entry in
//! `bifurc <group> --help` and its tool in the MCP server, so the three cannot disagree.

use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    commands::{Context, account_id, find_one, list_entities, refused, with_field, workspace_id, workspace_rows},
    error::{CliError, Result},
    format::{js_template, json},
    vault::find_vault,
};

/// What running a verb does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Reads,
    Adds,
    Changes,
    /// Reaches outside the engine: a remote, a server, a file.
    Runs,
    Deletes,
}

/// What an argument is, and so how it becomes a value of the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    Text,
    Int,
    /// `true` or `false`.
    Bool,
    Choice(&'static [&'static str]),
    /// JSON, or `@<file>` that holds it.
    Json,
    /// This operand and every one after it, as a list of text.
    List,
    /// The id or name of a saved item of one of these kinds: its id, and its kind as the payload's `kind`.
    Item(&'static [&'static str]),
    /// [`Take::Item`] for a command that knows the kind: the id alone.
    Named(&'static [&'static str]),
    /// A workspace's id or name: its id.
    Workspace,
    /// A signed-in account's login: its id.
    Account,
    /// A vault's name: its id.
    Vault,
    /// A file on this machine: uploaded to the engine, and the id it was given.
    File,
}

/// One argument of a verb.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// The option's name (no dashes), or what an operand is called in the help. One lowercase word.
    pub name: &'static str,
    /// Where it goes in the payload; `a/b` is the field `b` of the object `a`.
    pub key: &'static str,
    pub take: Take,
    pub operand: bool,
    pub required: bool,
    pub about: &'static str,
}

impl Arg {
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// How the argument is written in a usage line.
    fn signature(&self) -> String {
        let written = match (self.operand, self.take) {
            (true, Take::List) => format!("<{}>...", self.name),
            (true, _) => format!("<{}>", self.name),
            (false, take) => {
                let value = match take {
                    Take::Int => "<n>".to_string(),
                    Take::Bool => "true|false".to_string(),
                    Take::Choice(words) => words.join("|"),
                    Take::Json => "<json|@file>".to_string(),
                    Take::Item(_) | Take::Named(_) | Take::Workspace => "<id|name>".to_string(),
                    Take::Account => "<login>".to_string(),
                    Take::File => "<file>".to_string(),
                    Take::Text | Take::List | Take::Vault => "<text>".to_string(),
                };
                format!("--{} {value}", self.name)
            }
        };
        if self.required { written } else { format!("[{written}]") }
    }
}

/// An operand; required unless made [`Arg::optional`].
pub fn operand(name: &'static str, key: &'static str, take: Take, about: &'static str) -> Arg {
    Arg { name, key, take, operand: true, required: true, about }
}

/// An option; optional unless made [`Arg::required`].
pub fn option(name: &'static str, key: &'static str, take: Take, about: &'static str) -> Arg {
    Arg { name, key, take, operand: false, required: false, about }
}

/// Which workspace a verb acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// None: the engine as a whole.
    Engine,
    /// `--workspace`, else the active one.
    Workspace,
    /// The active one only: the engine command has no way to name another.
    Active,
}

/// One verb.
#[derive(Debug, Clone, PartialEq)]
pub struct Verb {
    /// The group and the verb: `["capture", "list"]`.
    pub words: &'static [&'static str],
    pub command: &'static str,
    /// One sentence: what it does.
    pub about: &'static str,
    /// What else there is to know; may be empty.
    pub details: &'static str,
    pub effect: Effect,
    pub scope: Scope,
    /// Fields every call sends, as the text of a JSON object; empty for none.
    pub fixed: &'static str,
    pub args: Vec<Arg>,
    /// The verb changes a saved item: `data` is laid over the item as it is saved and the whole is written back.
    pub patch: bool,
    /// The command answers with a file: it is written to `--out`, or printed.
    pub out: bool,
    /// What it does cannot be undone: it wants `--yes`.
    pub confirm: bool,
}

impl Verb {
    fn details(mut self, details: &'static str) -> Self {
        self.details = details;
        self
    }

    fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    fn fixed(mut self, fixed: &'static str) -> Self {
        self.fixed = fixed;
        self
    }

    fn args(mut self, args: Vec<Arg>) -> Self {
        self.args = args;
        self
    }

    fn patch(mut self) -> Self {
        self.patch = true;
        self
    }

    fn out(mut self) -> Self {
        self.out = true;
        self
    }

    fn confirm(mut self) -> Self {
        self.confirm = true;
        self
    }

    /// The details, with what the row itself says about the verb.
    pub fn notes(&self) -> String {
        let mut notes = vec![self.details];
        if self.scope == Scope::Active {
            notes.push("Acts on the active workspace only.");
        }
        if self.confirm {
            notes.push("Cannot be undone: it runs only with --yes.");
        }
        if self.out {
            notes.push("The file is written to --out; without it the content is printed.");
        }
        notes.retain(|note| !note.is_empty());
        notes.join(" ")
    }
}

fn verb(words: &'static [&'static str], command: &'static str, effect: Effect, about: &'static str) -> Verb {
    Verb {
        words,
        command,
        about,
        details: "",
        effect,
        scope: Scope::Workspace,
        fixed: "",
        args: vec![],
        patch: false,
        out: false,
        confirm: false,
    }
}

/// The request kinds, as the engine names them.
pub const REQUEST_KINDS: &[&str] = &[
    "requests",
    "graphqlRequests",
    "grpcRequests",
    "soapRequests",
    "wsConnections",
    "sseRequests",
    "socketioRequests",
    "mcpRequests",
];

/// Every kind of item that is deleted with `delete` and so may wait to be confirmed.
const DELETED_KINDS: &[&str] = &[
    "mappings",
    "proxyRules",
    "mocks",
    "flows",
    "environments",
    "requests",
    "graphqlRequests",
    "grpcRequests",
    "soapRequests",
    "wsConnections",
    "sseRequests",
    "socketioRequests",
    "mcpRequests",
];

const TREES: &[&str] = &["request", "mock", "flow", "rule"];

const DATA: &str =
    "A JSON object of the fields to set, or @<file> that holds one. The fields are the ones `show --json` prints.";
const TREE: &str = "Which tree the folder is in.";
const FOLDER: &str = "The folder's id (see: folders list).";
const PATH: &str = "The item's file in the workspace, as `sync changes` lists it, e.g. mocks/Default/<id>.json.";
const PENDING: &str = "One that was never synced is gone at once. One that was synced waits as a pending deletion, \
    which `sync confirm` commits and `sync restore` undoes.";

/// `delete`, `duplicate` and the verb that sets fields from JSON, for one group of saved items.
macro_rules! items {
    ($group:literal, $one:literal, $a:literal, $kinds:expr, $patch:literal, $fields:literal) => {
        [
            verb(&[$group, "delete"], "entity.delete", Effect::Deletes, concat!("Delete ", $a, "."))
                .details(PENDING)
                .scope(Scope::Active)
                .args(vec![operand($one, "id", Take::Item($kinds), "Its id or name.")]),
            verb(&[$group, "duplicate"], "entity.duplicate", Effect::Adds, concat!("Save a copy of ", $a, "."))
                .details("The copy is named \"<name> copy\"; a copy of something that can be enabled starts disabled.")
                .args(vec![operand($one, "id", Take::Item($kinds), "Its id or name.")]),
            verb(
                &[$group, $patch],
                "entity.update",
                Effect::Changes,
                concat!("Set any fields of ", $a, " from JSON; the fields not given stay as they are."),
            )
            .details($fields)
            .patch()
            .args(vec![
                operand($one, "id", Take::Item($kinds), "Its id or name."),
                option("data", "data", Take::Json, DATA).required(),
            ]),
        ]
    };
}

/// Every verb that is one engine command, in the order the help lists them.
pub fn verbs() -> Vec<Verb> {
    let folder = || operand("folder", "id", Take::Text, FOLDER);
    let tree = || option("tree", "kind", Take::Choice(TREES), TREE).required();
    let path = || operand("path", "relPath", Take::Text, PATH);
    let limit = || option("limit", "limit", Take::Int, "The most entries to return.");
    let offset = || option("offset", "offset", Take::Int, "How many entries to skip.");
    let target = || {
        option("target", "target", Take::Choice(&["forward", "reverse"]), "Which proxy. Default: the forward proxy.")
    };
    let message = || option("message", "message", Take::Text, "The commit message. Default: one the engine writes.");

    let mut rows: Vec<Verb> = Vec::new();
    rows.extend(items!("mappings", "mapping", "a mapping", &["mappings"], "patch", ""));
    rows.extend(items!(
        "rules",
        "rule",
        "a forward-proxy rule",
        &["proxyRules"],
        "patch",
        "requestScript and responseScript are the rule's scripts (JavaScript). folderId moves it to another folder."
    ));
    rows.extend(items!(
        "mocks",
        "mock",
        "a mock",
        &["mocks"],
        "patch",
        "folderId moves it to another folder; streamingMode is none, chunked or sse."
    ));
    rows.extend(items!(
        "requests",
        "request",
        "a saved request",
        REQUEST_KINDS,
        "patch",
        "preScript, postScript and testScript are its scripts (JavaScript, the bc.* API, shaped like Postman's \
         pm.*). folderId moves it to another folder."
    ));
    rows.extend(items!(
        "env",
        "environment",
        "an environment",
        &["environments"],
        "patch",
        "variables is the whole list, [{\"key\": \"HOST\", \"value\": \"…\"}]; `env set` changes one variable."
    ));
    rows.extend(items!(
        "flows",
        "flow",
        "a flow",
        &["flows"],
        "update",
        "nodes and edges are each the whole list. Check the result with `flows validate`."
    ));
    rows.extend([
        // -- Flows --------------------------------------------------------------------------------------------
        verb(&["flows", "create"], "entity.create", Effect::Adds, "Save a new flow from its JSON.")
            .details("See `bifurc flows --help` for the JSON of a flow. Check the result with `flows validate`.")
            .fixed(r#"{"kind": "flows"}"#)
            .args(vec![option("data", "entity", Take::Json, "The flow, or @<file> that holds it.").required()]),
        verb(
            &["flows", "validate"],
            "flow.validate",
            Effect::Reads,
            "Say what is wrong with a flow, and the ports of each node.",
        )
        .args(vec![operand("flow", "id", Take::Named(&["flows"]), "Its id or name.")]),
        verb(
            &["flows", "status"],
            "flow.status",
            Effect::Reads,
            "Show the flow's run: the one going on, or the last report.",
        )
        .args(vec![operand("flow", "id", Take::Named(&["flows"]), "Its id or name.")]),
        verb(&["flows", "cancel"], "flow.cancel", Effect::Changes, "Stop the run of a flow.").args(vec![operand(
            "flow",
            "id",
            Take::Named(&["flows"]),
            "Its id or name.",
        )]),
        verb(
            &["flows", "loadstatus"],
            "flow.loadtest.status",
            Effect::Reads,
            "Show the flow's load test: the one going on, or the last report.",
        )
        .args(vec![operand("flow", "id", Take::Named(&["flows"]), "Its id or name.")]),
        verb(&["flows", "loadcancel"], "flow.loadtest.cancel", Effect::Changes, "Stop the load test of a flow.")
            .args(vec![operand("flow", "id", Take::Named(&["flows"]), "Its id or name.")]),
        // -- Environments -------------------------------------------------------------------------------------
        verb(&["env", "create"], "entity.create", Effect::Adds, "Create an environment with no variables.")
            .details("Fill it with `env set`.")
            .fixed(r#"{"kind": "environments", "entity": {"variables": []}}"#)
            .args(vec![operand("name", "entity/name", Take::Text, "The new environment's name.")]),
        // -- Folders ------------------------------------------------------------------------------------------
        verb(&["folders", "create"], "folder.add", Effect::Adds, "Create a folder in a tree.")
            .fixed(r#"{"parentId": null}"#)
            .args(vec![
                operand("name", "name", Take::Text, "The folder's name."),
                tree(),
                option("parent", "parentId", Take::Text, "The id of the folder it goes in. Default: the top level."),
            ]),
        verb(&["folders", "rename"], "folder.rename", Effect::Changes, "Rename a folder.").args(vec![
            folder(),
            operand("name", "name", Take::Text, "The new name."),
            tree(),
        ]),
        verb(
            &["folders", "move"],
            "folder.move",
            Effect::Changes,
            "Move a folder into another folder, or to the top level.",
        )
        .fixed(r#"{"parentId": null}"#)
        .args(vec![
            folder(),
            tree(),
            option("parent", "parentId", Take::Text, "The id of the folder it goes in. Default: the top level."),
        ]),
        verb(
            &["folders", "delete"],
            "folder.delete",
            Effect::Deletes,
            "Delete a folder, the folders in it and every item in them.",
        )
        .details(PENDING)
        .args(vec![folder(), tree()]),
        verb(
            &["folders", "confirm"],
            "folder.confirmDelete",
            Effect::Deletes,
            "Commit the pending deletion of a folder.",
        )
        .args(vec![folder(), tree()]),
        verb(
            &["folders", "restore"],
            "folder.revertDelete",
            Effect::Changes,
            "Bring back a folder whose deletion is pending.",
        )
        .args(vec![folder(), tree()]),
        // -- Collection runner --------------------------------------------------------------------------------
        verb(
            &["runner", "status"],
            "runner.status",
            Effect::Reads,
            "Show the run of a folder: the one going on, or the last.",
        )
        .args(vec![operand("folder", "folderId", Take::Text, FOLDER)]),
        verb(&["runner", "cancel"], "runner.cancel", Effect::Changes, "Stop the run of a folder.").args(vec![operand(
            "folder",
            "folderId",
            Take::Text,
            FOLDER,
        )]),
        verb(
            &["runner", "config"],
            "runner.loadConfig",
            Effect::Reads,
            "Show how a folder is run: the order of its requests and the delay.",
        )
        .args(vec![operand("folder", "folderId", Take::Text, FOLDER)]),
        verb(&["runner", "configure"], "runner.saveConfig", Effect::Changes, "Set how a folder is run.")
            .details("The JSON is what `runner config` prints, e.g. {\"delayMs\": 200}.")
            .args(vec![
                operand("folder", "folderId", Take::Text, FOLDER),
                option("data", "config", Take::Json, "The whole configuration, or @<file> that holds it.").required(),
            ]),
        // -- Captured traffic ---------------------------------------------------------------------------------
        // `capture list` and `capture show` are code (capture.rs): filters, columns and the response output.
        verb(&["capture", "remove"], "capture.remove", Effect::Deletes, "Remove captured entries.")
            .args(vec![operand("entry", "ids", Take::List, "The ids of the entries.")]),
        verb(
            &["capture", "secrets"],
            "capture.inspectImport",
            Effect::Reads,
            "List the credentials and cookies that importing these entries would take out of the requests.",
        )
        .args(vec![operand("entry", "ids", Take::List, "The ids of the entries.")]),
        verb(
            &["capture", "import"],
            "capture.import",
            Effect::Adds,
            "Save entries as requests, in a new folder when there are several, optionally chained into a flow.",
        )
        .details("Credentials go to the selected environment or an unsealed vault, cookies to the workspace's jar, when asked.")
        .args(vec![
            operand("entry", "ids", Take::List, "The ids of the entries."),
            option("secrets", "secrets", Take::Choice(&["keep", "environment", "vault"]), "Where credentials go. Default: keep."),
            option("vault", "vaultId", Take::Vault, "The unsealed vault, with --secrets vault."),
            option("jar", "cookiesToJar", Take::Bool, "true: move cookies into the cookie jar."),
            option("flow", "flow", Take::Bool, "true: also build a flow that sends them in the captured order."),
            option("folder", "folderName", Take::Text, "The name of the new folder (and flow). Default: Captured."),
            option(
                "keep-all",
                "keepAll",
                Take::Bool,
                "true: also save preflights, static assets and analytics calls, which are left out by default.",
            ),
        ]),
        verb(&["capture", "clear"], "capture.clear", Effect::Deletes, "Remove every captured entry of the workspace."),
        verb(&["capture", "pause"], "capture.setPaused", Effect::Changes, "Stop capturing traffic.")
            .fixed(r#"{"paused": true}"#),
        verb(&["capture", "resume"], "capture.setPaused", Effect::Changes, "Capture traffic again.")
            .fixed(r#"{"paused": false}"#),
        verb(
            &["capture", "cache"],
            "capture.setDisableCache",
            Effect::Changes,
            "Say whether the proxies let clients cache responses.",
        )
        .scope(Scope::Engine)
        .args(vec![
            option("off", "disableCache", Take::Bool, "true: responses are never cached; false: they may be.")
                .required(),
        ]),
        // -- Cookies ------------------------------------------------------------------------------------------
        verb(&["cookies", "list"], "cookies.list", Effect::Reads, "List the cookies the workspace's requests hold."),
        verb(&["cookies", "clear"], "cookies.clear", Effect::Deletes, "Forget the cookies of the workspace.")
            .args(vec![option("domain", "domain", Take::Text, "Forget only this domain's cookies.")]),
        verb(
            &["cookies", "configure"],
            "cookies.configure",
            Effect::Changes,
            "Say whether the cookies are forgotten when the engine restarts.",
        )
        .args(vec![
            option("clear", "clearOnRestart", Take::Bool, "true: forgotten on a restart; false: kept.").required(),
        ]),
        // -- Git: sync, history -------------------------------------------------------------------------------
        verb(
            &["sync", "status"],
            "sync.getState",
            Effect::Reads,
            "Show the workspace's remote, account, owner and what waits to be pushed or pulled.",
        ),
        verb(
            &["sync", "changes"],
            "sync.getEntityStatus",
            Effect::Reads,
            "List the files that changed since the last sync, and the pending deletions.",
        ),
        verb(&["sync", "pull"], "sync.pull", Effect::Runs, "Pull the remote's changes into the workspace."),
        verb(&["sync", "push"], "sync.push", Effect::Runs, "Push the workspace's commits to its remote."),
        verb(&["sync", "commit"], "entity.publish", Effect::Runs, "Commit the changes of some files, and push.")
            .args(vec![operand("path", "paths", Take::List, PATH), message()]),
        verb(&["sync", "diff"], "git.diff", Effect::Reads, "Show what changed in a file since the last sync.")
            .args(vec![path()]),
        verb(&["sync", "discard"], "git.discard", Effect::Deletes, "Throw away a file's changes since the last sync.")
            .confirm()
            .args(vec![path()]),
        verb(&["sync", "confirm"], "entity.confirmDelete", Effect::Deletes, "Commit the pending deletion of an item.")
            .args(vec![
                operand("kind", "kind", Take::Choice(DELETED_KINDS), "The item's kind: mappings, proxyRules, mocks, flows, environments, or the kind of a request (requests, graphqlRequests, grpcRequests, soapRequests, wsConnections, sseRequests, socketioRequests, mcpRequests)."),
                operand("id", "id", Take::Text, "The item's id."),
            ]),
        verb(
            &["sync", "restore"],
            "entity.revertDelete",
            Effect::Changes,
            "Bring back an item whose deletion is pending.",
        )
        .args(vec![
            operand("kind", "kind", Take::Choice(DELETED_KINDS), "The item's kind: mappings, proxyRules, mocks, flows, environments, or the kind of a request (requests, graphqlRequests, grpcRequests, soapRequests, wsConnections, sseRequests, socketioRequests, mcpRequests)."),
            operand("id", "id", Take::Text, "The item's id."),
        ]),
        verb(
            &["history", "log"],
            "audit.list",
            Effect::Reads,
            "List the workspace's audit log: who changed what, newest first.",
        )
        .args(vec![
            option("search", "search", Take::Text, "Only entries that mention this text."),
            option(
                "entity",
                "entity",
                Take::Text,
                "Only entries about this kind of item, e.g. mock, request, mapping.",
            ),
            option("action", "action", Take::Text, "Only entries of this action: create, update or delete."),
            option("item", "entityId", Take::Text, "Only entries about the item with this id."),
            limit(),
            offset(),
        ]),
        verb(&["history", "file"], "history.list", Effect::Reads, "List the commits that changed one file.")
            .args(vec![operand("path", "filePath", Take::Text, PATH), limit(), offset()]),
        verb(&["history", "show"], "history.patch", Effect::Reads, "Show what one commit changed, as a patch.").args(
            vec![
                operand("commit", "commitHash", Take::Text, "The commit's hash (see: history log)."),
                option("path", "filePath", Take::Text, "Only the changes to this file."),
            ],
        ),
        verb(&["history", "export"], "audit.export", Effect::Reads, "Write the whole audit log as CSV or JSON.")
            .out()
            .args(vec![option("format", "format", Take::Choice(&["csv", "json"]), "The format.").required()]),
        // -- Workspaces and accounts --------------------------------------------------------------------------
        verb(
            &["workspaces", "publish"],
            "sync.publish",
            Effect::Runs,
            "Create a GitHub repository for a workspace and push it there.",
        )
        .args(vec![
            option("account", "accountId", Take::Account, "The signed-in account that will own the repository.")
                .required(),
            option("repo", "repoName", Take::Text, "The new repository's name.").required(),
            option("private", "private", Take::Bool, "Whether the repository is private. Default: true."),
            option("all", "syncAll", Take::Bool, "true: also commit and push everything that was never synced."),
        ]),
        verb(
            &["workspaces", "import"],
            "workspace.import",
            Effect::Runs,
            "Clone a Bifurc workspace from a GitHub repository.",
        )
        .scope(Scope::Engine)
        .args(vec![
            option("account", "accountId", Take::Account, "The signed-in account that can read the repository.")
                .required(),
            option("repo", "repo", Take::Text, "The repository, as owner/name (see: account repos).").required(),
        ]),
        verb(&["workspaces", "delete"], "workspace.delete", Effect::Deletes, "Remove a workspace from this device.")
            .details("What was never pushed is lost.")
            .scope(Scope::Engine)
            .confirm()
            .args(vec![operand("workspace", "id", Take::Workspace, "The workspace's id or name.")]),
        verb(
            &["account", "logout"],
            "account.logout",
            Effect::Deletes,
            "Sign a GitHub account out and remove its workspaces from this device.",
        )
        .details("What those workspaces never pushed is lost.")
        .scope(Scope::Engine)
        .confirm()
        .args(vec![operand("account", "accountId", Take::Account, "The account's login.")]),
        // -- Import and export --------------------------------------------------------------------------------
        verb(
            &["export", "formats"],
            "export.formats",
            Effect::Reads,
            "List what can be exported and imported, and the format names.",
        )
        .scope(Scope::Engine),
        verb(
            &["export", "create"],
            "export.create",
            Effect::Reads,
            "Export a workspace's items of one kind as a file.",
        )
        .out()
        .args(vec![
            option("kind", "kind", Take::Text, "What to export (see: export formats).").required(),
            option("format", "format", Take::Text, "The format's id (see: export formats).").required(),
        ]),
        verb(
            &["import", "check"],
            "import.preflight",
            Effect::Reads,
            "Say what importing a file would add, without adding it.",
        )
        .args(vec![
            operand("file", "blobId", Take::File, "The file to import."),
            option("kind", "kind", Take::Text, "What the file holds (see: export formats).").required(),
            option("format", "format", Take::Text, "The format's id (see: export formats).").required(),
        ]),
        verb(
            &["import", "run"],
            "import.commit",
            Effect::Adds,
            "Import a file of another tool, or a Bifurc export, into the workspace.",
        )
        .args(vec![
            operand("file", "blobId", Take::File, "The file to import."),
            option("kind", "kind", Take::Text, "What the file holds (see: export formats).").required(),
            option("format", "format", Take::Text, "The format's id (see: export formats).").required(),
            option(
                "collision",
                "collisionStrategy",
                Take::Choice(&["keep", "override", "new"]),
                "An item that is already there: keep it (default), replace it, or add the imported one beside it.",
            ),
        ]),
        // -- Proxies, settings, HTTPS -------------------------------------------------------------------------
        verb(&["proxy", "status"], "server.status", Effect::Reads, "Show whether a proxy is running, and its port.")
            .scope(Scope::Engine)
            .args(vec![target()]),
        verb(&["proxy", "start"], "server.start", Effect::Changes, "Start a proxy.")
            .scope(Scope::Engine)
            .args(vec![target()]),
        verb(&["proxy", "stop"], "server.stop", Effect::Changes, "Stop a proxy.")
            .scope(Scope::Engine)
            .args(vec![target()]),
        verb(&["proxy", "restart"], "server.restart", Effect::Changes, "Stop a proxy and start it again.")
            .scope(Scope::Engine)
            .args(vec![target()]),
        verb(
            &["settings", "update"],
            "settings.update",
            Effect::Changes,
            "Change the engine's settings; a proxy whose port changed is restarted.",
        )
        .scope(Scope::Engine)
        .args(vec![
            option("port", "port", Take::Int, "The reverse proxy's port, where <sub>.localhost is served."),
            option("forwardport", "forwardProxyPort", Take::Int, "The forward proxy's port."),
            option("companionport", "companionPort", Take::Int, "The port the browser extension connects to."),
            option("capturelimit", "captureLimit", Take::Int, "How many unpinned captured requests are kept (10-10000)."),
            option("approvaltimeout", "approvalTimeoutSeconds", Take::Int, "Seconds a call held by a require_human policy waits for a person (default 300, 10-3600)."),
            option("mailport", "mailPort", Take::Int, "The port of the local mail sink (default 1025); mail list shows what it caught."),
            option("traceheader", "traceHeader", Take::Text, "Header that carries the capture id upstream (default x-bifurc-trace; empty = off)."),
            option(
                "https",
                "tlsEnabled",
                Take::Bool,
                "Whether HTTPS is served and intercepted (needs a root certificate: tls generate).",
            ),
            option("telemetry", "telemetryEnabled", Take::Bool, "Whether anonymous usage analytics are sent."),
            option(
                "updatecheck",
                "updateCheckEnabled",
                Take::Bool,
                "Whether a newer version is looked for once a day.",
            ),
        ]),
        verb(
            &["tls", "status"],
            "tls.certStatus",
            Effect::Reads,
            "Show whether there is a root certificate, and its fingerprint.",
        )
        .scope(Scope::Engine),
        verb(
            &["tls", "generate"],
            "tls.generate",
            Effect::Changes,
            "Generate a new root certificate, replacing the one there is.",
        )
        .details("It has to be trusted by the system or the browser before HTTPS interception works.")
        .scope(Scope::Engine),
        verb(
            &["tls", "export"],
            "tls.exportCert",
            Effect::Reads,
            "Write the root certificate (PEM), to install it as trusted.",
        )
        .scope(Scope::Engine)
        .out(),
        verb(&["tls", "importcert"], "tls.importCert", Effect::Changes, "Use a root certificate of your own.")
            .scope(Scope::Engine)
            .args(vec![operand("file", "blobId", Take::File, "The certificate, a PEM file.")]),
        verb(
            &["tls", "importkey"],
            "tls.importKey",
            Effect::Changes,
            "Use the private key of a root certificate of your own.",
        )
        .scope(Scope::Engine)
        .args(vec![operand("file", "blobId", Take::File, "The key, a PEM file.")]),
        verb(&["tls", "remove"], "tls.removeCert", Effect::Deletes, "Remove the root certificate and its key.")
            .scope(Scope::Engine),
        // -- Vaults, updates ----------------------------------------------------------------------------------
        verb(
            &["vault", "rename"],
            "vault.rename",
            Effect::Changes,
            "Rename a vault; {{vault.SECRET}} references follow the new name.",
        )
        .args(vec![
            operand("vault", "id", Take::Vault, "The vault's name."),
            operand("name", "name", Take::Text, "The new name."),
        ]),
        verb(&["vault", "unset"], "vault.deleteSecret", Effect::Deletes, "Remove one secret from a local vault.")
            .details("A linked vault's secrets are removed in its provider; the next pull leaves them out.")
            .args(vec![
                operand("vault", "id", Take::Vault, "The vault's name."),
                operand("secret", "name", Take::Text, "The secret's name."),
            ]),
        verb(&["vault", "delete"], "vault.delete", Effect::Deletes, "Delete a vault and every secret in it.")
            .confirm()
            .args(vec![operand("vault", "id", Take::Vault, "The vault's name.")]),
        verb(&["vault", "providers"], "vault.providers", Effect::Reads, "The secret managers a vault can be linked to.")
            .scope(Scope::Engine),
        verb(&["vault", "test"], "vault.test", Effect::Runs, "Pull a linked vault's source once and name what it holds.")
            .details("Prints the names of the secrets it would pull, never a value. Needs a person at a terminal.")
            .args(vec![operand("vault", "id", Take::Vault, "The vault's name.")]),
        verb(
            &["vault", "approve"],
            "vault.configure",
            Effect::Changes,
            "Accept on this device where a linked vault pulls its secrets from.",
        )
        .details("Needed once on each device, and again when a sync changed its provider settings.")
        .args(vec![operand("vault", "id", Take::Vault, "The vault's name.")]),
        verb(&["update", "skip"], "update.skip", Effect::Changes, "Stop being told about one version of Bifurc.")
            .scope(Scope::Engine)
            .args(vec![operand("version", "version", Take::Text, "The version to skip, e.g. 0.7.0.")]),
    ]);
    rows
}

/// The verb `group verb` names, when it is one of the table's.
pub fn find(group: &str, verb: &str) -> Option<Verb> {
    verbs().into_iter().find(|row| row.words == [group, verb])
}

/// The table's verbs of `group` as `--help` shows them; empty when it has none.
pub fn usage(group: &str) -> String {
    let mut lines = Vec::new();
    for verb in verbs().iter().filter(|verb| verb.words[0] == group) {
        let mut line = format!("  {}", verb.words[1]);
        for arg in &verb.args {
            line.push(' ');
            line.push_str(&arg.signature());
        }
        if verb.out {
            line.push_str(" [--out <file>]");
        }
        if verb.confirm {
            line.push_str(" --yes");
        }
        lines.push(line);
        lines.push(format!("{:6}{} {}", "", verb.about, verb.notes()).trim_end().to_string());
        for arg in &verb.args {
            let label = if arg.operand { format!("<{}>", arg.name) } else { format!("--{}", arg.name) };
            lines.push(format!("{:8}{label:<16} {}", "", arg.about));
        }
    }
    lines.join("\n")
}

/// Set the field `key` names; `a/b` is the field `b` of the object `a`.
fn put(payload: &mut Value, key: &str, value: Value) {
    let mut at = payload;
    for part in key.split('/') {
        at = &mut at[part];
    }
    *at = value;
}

/// An argument that needs nothing of the engine, as the value the payload takes. `None` for one that does.
fn simple(arg: &Arg, given: &[String]) -> Result<Option<Value>> {
    let raw = given[0].as_str();
    let wrong = |expected: &str| CliError::usage(format!("{} must be {expected}, not \"{raw}\"", arg.name));
    Ok(Some(match arg.take {
        Take::Text => json!(raw),
        Take::Int => json!(raw.parse::<i64>().map_err(|_| wrong("a whole number"))?),
        Take::Bool => json!(raw.parse::<bool>().map_err(|_| wrong("true or false"))?),
        Take::Choice(words) if words.contains(&raw) => json!(raw),
        Take::Choice(words) => return Err(wrong(&format!("one of {}", words.join(", ")))),
        Take::Json => {
            let text = match raw.strip_prefix('@') {
                Some(file) => {
                    std::fs::read_to_string(file).map_err(|e| CliError::msg(format!("cannot read {file}: {e}")))?
                }
                None => raw.to_string(),
            };
            serde_json::from_str(&text)
                .map_err(|e| CliError::usage(format!("--{} is not valid JSON: {e}", arg.name)))?
        }
        Take::List => json!(given),
        Take::Item(_) | Take::Named(_) | Take::Workspace | Take::Account | Take::Vault | Take::File => return Ok(None),
    }))
}

/// Give the engine the file at `path` and return the id it is kept under.
fn upload(ctx: &mut Context, path: &str, workspace: Option<&str>) -> Result<String> {
    // The engine first: without one there is nowhere to put the file, and that is the error to give.
    ctx.engine()?;
    let bytes = std::fs::read(path).map_err(|e| CliError::msg(format!("cannot read {path}: {e}")))?;
    let name = Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = ctx.request(
        "blob.put",
        json!({
            "filename": name, "mimeType": "application/octet-stream", "size": bytes.len(),
            "data": STANDARD.encode(&bytes), "workspaceId": workspace,
        }),
    )?;
    Ok(js_template(staged.get("blobId")))
}

/// The file a command answered with: written to `--out`, or returned as text.
fn save(ctx: &mut Context, artifact: &Value, workspace: Option<&str>) -> Result<String> {
    let decode = |data: Option<&Value>| {
        STANDARD
            .decode(data.and_then(Value::as_str).unwrap_or_default())
            .map_err(|e| CliError::msg(format!("the engine sent a file that cannot be read: {e}")))
    };
    let bytes = match artifact.get("inline") {
        Some(inline) => decode(Some(inline))?,
        // Too large to come with the answer: the engine keeps it, and it is read a piece at a time.
        None => {
            let mut bytes = Vec::new();
            loop {
                let request = json!({ "blobId": artifact["blobId"], "offset": bytes.len(), "workspaceId": workspace });
                let piece = ctx.request("blob.read", request)?;
                let data = decode(piece.get("data"))?;
                let done = data.is_empty() || piece.get("eof") != Some(&json!(false));
                bytes.extend(data);
                if done {
                    break bytes;
                }
            }
        }
    };
    match ctx.args.flag("out") {
        Some(file) => {
            std::fs::write(file, &bytes).map_err(|e| CliError::msg(format!("cannot write {file}: {e}")))?;
            let size = bytes.len();
            Ok(ctx.print(&json!({ "ok": true, "file": file, "size": size }), || format!("Wrote {file} ({size} bytes)")))
        }
        None => Ok(String::from_utf8_lossy(&bytes).into_owned()),
    }
}

/// Lay the payload's `data` over the item it names and write the item back. Returns the item as it is now.
fn patch(ctx: &mut Context, payload: &Value) -> Result<Value> {
    let Some(data) = payload.get("data").and_then(Value::as_object) else {
        return Err(CliError::usage("--data must be a JSON object of the fields to set"));
    };
    if data.contains_key("id") {
        return Err(CliError::usage("an item's id cannot be changed"));
    }
    let (workspace, kind, id) = (&payload["workspaceId"], &payload["kind"], &payload["id"]);
    let loaded = ctx.request("entity.load", json!({ "workspaceId": workspace, "kind": kind, "id": id }))?;
    let Some(mut entity) = loaded.get("entity").filter(|entity| entity.is_object()).cloned() else {
        return Err(CliError::msg(format!("{} cannot be read", js_template(Some(id)))));
    };
    for (field, value) in data {
        entity[field] = value.clone();
    }
    if entity.get("workspaceId").is_none_or(Value::is_null) {
        entity["workspaceId"] = workspace.clone();
    }
    refused(ctx.request("entity.update", json!({ "kind": kind, "entity": entity }))?)?;
    Ok(entity)
}

/// Run `verb` with the arguments of the command line.
pub fn run(ctx: &mut Context, verb: &Verb) -> Result<String> {
    let args = ctx.args;
    let (group, command) = (verb.words[0], verb.words.join(" "));
    let mut flags: Vec<&str> = verb.args.iter().filter(|arg| !arg.operand).map(|arg| arg.name).collect();
    if verb.out {
        flags.push("out");
    }
    if verb.confirm {
        flags.push("yes");
    }
    // A list takes every operand that is left.
    let listed = verb.args.iter().any(|arg| arg.take == Take::List);
    let operands = if listed { 1000 } else { verb.args.iter().filter(|arg| arg.operand).count() };
    ctx.accept(&flags, operands)?;
    if verb.scope == Scope::Active && args.flag("workspace").is_some() {
        return Err(CliError::msg(format!(
            "\"{command}\" acts on the active workspace: run bifurc workspaces use <workspace>, and leave out --workspace"
        )));
    }

    // What needs nothing of the engine is read first, so a bad argument changes nothing.
    let mut payload: Value = if verb.fixed.is_empty() {
        json!({})
    } else {
        serde_json::from_str(verb.fixed).expect("a row's fixed fields are JSON")
    };
    let mut later: Vec<(&Arg, String)> = Vec::new();
    let mut next = 2;
    for arg in &verb.args {
        let given: Vec<String> = match (arg.operand, arg.take) {
            (true, Take::List) => args.positionals.iter().skip(next).cloned().collect(),
            (true, _) => {
                next += 1;
                args.positional(next - 1).map(str::to_string).into_iter().collect()
            }
            (false, _) => args.flag(arg.name).map(str::to_string).into_iter().collect(),
        };
        if given.is_empty() {
            if arg.required {
                let label = if arg.operand { format!("<{}>", arg.name) } else { format!("--{}", arg.name) };
                return Err(CliError::usage(format!("missing {label}: {} (see: bifurc {group} --help)", arg.about)));
            }
            continue;
        }
        match simple(arg, &given)? {
            Some(value) => put(&mut payload, arg.key, value),
            None => later.push((arg, given[0].clone())),
        }
    }
    if verb.confirm && !args.has("yes") {
        return Err(CliError::usage(format!("\"{command}\" cannot be undone: add --yes to do it")));
    }

    let workspace = match verb.scope {
        Scope::Engine => None,
        Scope::Workspace | Scope::Active => Some(workspace_id(ctx)?),
    };
    if let Some(workspace) = &workspace {
        payload["workspaceId"] = json!(workspace);
    }
    for (arg, wanted) in later {
        let value = match arg.take {
            Take::Item(kinds) | Take::Named(kinds) => {
                let workspace = workspace.as_deref().expect("a verb that names an item acts on a workspace");
                let mut rows = Vec::new();
                for kind in kinds {
                    for row in list_entities(ctx, workspace, kind)? {
                        rows.push(with_field(&row, "__kind", json!(kind)));
                    }
                }
                let hit = find_one(&rows, &wanted, &["name", "domain", "label"], arg.name, group)?;
                if matches!(arg.take, Take::Item(_)) {
                    payload["kind"] = hit["__kind"].clone();
                }
                hit["id"].clone()
            }
            Take::Workspace => {
                let (rows, _) = workspace_rows(ctx)?;
                find_one(&rows, &wanted, &["name"], "workspace", "workspaces")?["id"].clone()
            }
            Take::Account => json!(account_id(ctx, &wanted)?),
            Take::Vault => {
                let listed = ctx.request("vault.list", json!({ "workspaceId": workspace }))?;
                find_vault(&listed, &wanted)?["id"].clone()
            }
            Take::File => json!(upload(ctx, &wanted, workspace.as_deref())?),
            _ => unreachable!("simple() converted it"),
        };
        put(&mut payload, arg.key, value);
    }

    // A captured entry is named by a full id, `#tail` or `tail`.
    if group == "capture"
        && let Some(wanted) = payload.get("ids").and_then(Value::as_array)
    {
        let wanted: Vec<String> = wanted.iter().map(|id| js_template(Some(id))).collect();
        payload["ids"] = json!(crate::capture::entry_ids(ctx, workspace.as_deref().unwrap_or_default(), &wanted)?);
    }

    let response = if verb.patch {
        patch(ctx, &payload)?
    } else {
        // `{ok: false, error}` is a refusal. `{ok: false}` with no error is an answer: a flow that has problems.
        let response = ctx.request(verb.command, payload)?;
        if response.get("error").is_some_and(Value::is_string) { refused(response)? } else { response }
    };
    if verb.out {
        return save(ctx, &response, workspace.as_deref());
    }
    if command == "capture import" && !ctx.json {
        return Ok(crate::capture::imported(ctx, workspace.as_deref().unwrap_or_default(), &response));
    }
    // A bare `{ok: true}` (plus `pending` for a deletion) is one line, not the raw result.
    let bare = response
        .as_object()
        .is_some_and(|o| o.get("ok") == Some(&json!(true)) && o.keys().all(|k| k == "ok" || k == "pending"));
    if bare && !ctx.json {
        let pending = if response["pending"] == json!(true) {
            " (pending: sync confirm commits, sync restore undoes)"
        } else {
            ""
        };
        return Ok(match (verb.words[1], args.positional(2)) {
            ("delete", Some(item)) => format!("Deleted {item}{pending}"),
            _ => format!("ok{pending}"),
        });
    }
    Ok(ctx.print(&response, || json(&response)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{config, payloads, run as with_engine};

    fn go(argv: &[&str], answers: Vec<crate::testing::Answer>) -> (Result<String>, Vec<(String, Value)>) {
        let verb = find(argv[0], argv[1]).unwrap_or_else(|| panic!("no verb {argv:?}"));
        with_engine(argv, answers, |ctx| run(ctx, &verb))
    }

    #[test]
    fn every_row_is_well_formed() {
        let rows = verbs();
        for (i, row) in rows.iter().enumerate() {
            let name = row.words.join(" ");
            assert_eq!(row.words.len(), 2, "{name}");
            assert!(rows[..i].iter().all(|other| other.words != row.words), "{name} is there twice");
            assert!(row.about.ends_with('.') && row.about.len() <= 130, "{name}: {}", row.about.len());
            assert!(
                row.fixed.is_empty() || serde_json::from_str::<Value>(row.fixed).is_ok_and(|v| v.is_object()),
                "{name}"
            );
            let word = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_lowercase());
            assert!(row.words.iter().all(|w| word(w)), "{name}");
            for arg in &row.args {
                assert!(arg.name.split('-').all(word) && !arg.about.is_empty(), "{name} {}", arg.name);
                // An item is looked up in a workspace; a list has to be the last operand.
                let looked_up = matches!(arg.take, Take::Item(_) | Take::Named(_) | Take::Vault);
                assert!(!looked_up || row.scope != Scope::Engine, "{name}");
                assert!(arg.take != Take::List || row.args.iter().rfind(|a| a.operand) == Some(arg), "{name}");
            }
            assert!(!row.patch || row.args.iter().any(|arg| arg.key == "data"), "{name}");
        }
    }

    /// The engine checks every payload against the protocol's schema of the command, so a row whose payload
    /// the schema refuses is a verb that can never work: with every argument, and with only the required ones.
    #[test]
    fn every_row_sends_a_payload_the_protocol_takes() {
        let mut refused = Vec::new();
        for row in verbs().iter().filter(|row| !row.patch) {
            for all in [true, false] {
                let mut payload: Value =
                    if row.fixed.is_empty() { json!({}) } else { serde_json::from_str(row.fixed).unwrap() };
                if row.scope != Scope::Engine {
                    payload["workspaceId"] = json!("w1");
                }
                for arg in row.args.iter().filter(|arg| all || arg.required) {
                    let value = match arg.take {
                        Take::Int => json!(10),
                        Take::Bool => json!(true),
                        Take::Choice(words) => json!(words[0]),
                        Take::Json => json!({"name": "x"}),
                        Take::List => json!(["x"]),
                        Take::Item(kinds) => {
                            payload["kind"] = json!(kinds[0]);
                            json!("x")
                        }
                        _ => json!("x"),
                    };
                    put(&mut payload, arg.key, value);
                }
                if let Err(reason) = bifurc_protocol::commands::validate(row.command, &payload) {
                    refused.push(format!("{} sends {payload} to {}: {reason}", row.words.join(" "), row.command));
                }
            }
        }
        assert!(
            refused.is_empty(),
            "{}",
            refused.join(
                "
"
            )
        );
    }

    #[test]
    fn arguments_go_where_the_row_says() {
        let argv = ["folders", "create", "Auth", "--tree", "request", "--parent", "f1"];
        let (out, calls) = go(&argv, vec![("config.get", Ok(config())), ("folder.add", Ok(json!({"id": "f2"})))]);
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap(), json!({"id": "f2"}));
        assert_eq!(
            payloads(&calls, "folder.add"),
            [json!({"parentId": "f1", "name": "Auth", "kind": "request", "workspaceId": "w1"})]
        );

        let argv = ["env", "create", "Staging", "--workspace", "w2"];
        let (_, calls) = go(&argv, vec![("entity.create", Ok(json!({"id": "e9"})))]);
        assert_eq!(
            calls,
            [(
                "entity.create".to_string(),
                json!({"kind": "environments", "entity": {"variables": [], "name": "Staging"}, "workspaceId": "w2"})
            )]
        );

        let argv = ["capture", "remove", "#a1", "bbbb2", "--workspace", "w1"];
        let entries = json!({"entries": [{"id": "xxxxa1"}, {"id": "bbbb2"}]});
        let (_, calls) = go(&argv, vec![("capture.list", Ok(entries)), ("capture.remove", Ok(json!({"ok": true})))]);
        assert_eq!(calls[1].1, json!({"ids": ["xxxxa1", "bbbb2"], "workspaceId": "w1"}));

        let argv = ["settings", "update", "--port", "8081", "--https", "true"];
        let (_, calls) = go(&argv, vec![("settings.update", Ok(json!({"ok": true})))]);
        assert_eq!(calls, [("settings.update".to_string(), json!({"port": 8081, "tlsEnabled": true}))]);
    }

    #[test]
    fn a_bad_or_missing_argument_is_refused_before_the_engine_is_asked() {
        for (argv, expected) in [
            (
                vec!["folders", "create", "Auth"],
                "missing --tree: Which tree the folder is in. (see: bifurc folders --help)",
            ),
            (
                vec!["folders", "create", "Auth", "--tree", "shrub"],
                "tree must be one of request, mock, flow, rule, not \"shrub\"",
            ),
            (vec!["settings", "update", "--port", "eighty"], "port must be a whole number, not \"eighty\""),
            (vec!["cookies", "configure", "--clear", "yes"], "clear must be true or false, not \"yes\""),
            (vec!["capture", "remove"], "missing <entry>: The ids of the entries. (see: bifurc capture --help)"),
            (vec!["sync", "discard", "mocks/a.json"], "\"sync discard\" cannot be undone: add --yes to do it"),
            (
                vec!["mocks", "delete", "m1", "--workspace", "w2"],
                "\"mocks delete\" acts on the active workspace: run bifurc workspaces use <workspace>, and leave out --workspace",
            ),
        ] {
            let (out, calls) = go(&argv, vec![]);
            assert_eq!(out.unwrap_err().line(), format!("bifurc: {expected}"), "{argv:?}");
            assert!(calls.is_empty(), "{argv:?}");
        }
        let (out, _) = go(&["flows", "create", "--data", "{oops"], vec![]);
        assert!(out.unwrap_err().line().starts_with("bifurc: --data is not valid JSON"));
    }

    #[test]
    fn an_item_is_named_by_id_or_name_and_its_kind_goes_with_it() {
        let answers = || {
            vec![
                ("config.get", Ok(config())),
                ("entity.list:requests", Ok(json!({"entities": [{"id": "r1", "name": "Ping"}]}))),
                ("entity.list:wsConnections", Ok(json!({"entities": [{"id": "s1", "name": "Feed"}]}))),
                ("entity.list", Ok(json!({"entities": []}))),
                ("entity.delete", Ok(json!({"ok": true, "pending": true}))),
            ]
        };
        let (out, calls) = go(&["requests", "delete", "feed"], answers());
        assert_eq!(out.unwrap(), "Deleted feed (pending: sync confirm commits, sync restore undoes)");
        assert_eq!(
            payloads(&calls, "entity.delete"),
            [json!({"workspaceId": "w1", "kind": "wsConnections", "id": "s1"})]
        );
        let (out, _) = go(&["requests", "delete", "nope"], answers());
        assert_eq!(out.unwrap_err().line(), "bifurc: no request \"nope\" (see: bifurc requests list)");
    }

    #[test]
    fn a_patch_lays_the_given_fields_over_the_saved_item() {
        let saved = json!({"id": "r1", "name": "Ping", "url": "http://a", "workspaceId": "w1"});
        let answers = || {
            vec![
                ("config.get", Ok(config())),
                ("entity.list:requests", Ok(json!({"entities": [{"id": "r1", "name": "Ping"}]}))),
                ("entity.list", Ok(json!({"entities": []}))),
                ("entity.load", Ok(json!({"ok": true, "entity": saved.clone()}))),
                ("entity.update", Ok(json!({"ok": true}))),
            ]
        };
        let argv =
            ["requests", "patch", "Ping", "--data", r#"{"testScript": "bc.test('ok', () => {})", "folderId": "f1"}"#];
        let (out, calls) = go(&argv, answers());
        out.unwrap();
        assert_eq!(
            payloads(&calls, "entity.update"),
            [json!({"kind": "requests", "entity": {
                "id": "r1", "name": "Ping", "url": "http://a", "workspaceId": "w1",
                "testScript": "bc.test('ok', () => {})", "folderId": "f1",
            }})]
        );
        for (data, expected) in
            [("[1]", "--data must be a JSON object"), (r#"{"id": "r2"}"#, "an item's id cannot be changed")]
        {
            let (out, calls) = go(&["requests", "patch", "Ping", "--data", data], answers());
            assert!(out.unwrap_err().line().contains(expected), "{data}");
            assert!(payloads(&calls, "entity.update").is_empty());
        }
    }

    #[test]
    fn a_file_is_uploaded_and_a_file_answer_is_written_or_printed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("collection.json");
        std::fs::write(&source, "{}").unwrap();
        let argv = [
            "import",
            "run",
            source.to_str().unwrap(),
            "--kind",
            "requests",
            "--format",
            "postman",
            "--workspace",
            "w1",
        ];
        let answers = vec![("blob.put", Ok(json!({"blobId": "b1"}))), ("import.commit", Ok(json!({"ok": true})))];
        let (_, calls) = go(&argv, answers);
        assert_eq!(calls[0].1["data"], "e30=");
        assert_eq!(calls[0].1["filename"], "collection.json");
        assert_eq!(calls[1].1, json!({"kind": "requests", "format": "postman", "workspaceId": "w1", "blobId": "b1"}));

        let artifact = || vec![("tls.exportCert", Ok(json!({"ok": true, "inline": "aGk="})))];
        assert_eq!(go(&["tls", "export"], artifact()).0.unwrap(), "hi");
        let target = dir.path().join("ca.pem");
        let (out, _) = go(&["tls", "export", "--out", target.to_str().unwrap()], artifact());
        assert_eq!(out.unwrap(), format!("Wrote {} (2 bytes)", target.display()));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hi");

        // One too large to come inline is read from the engine until its end.
        let large = vec![
            ("tls.exportCert", Ok(json!({"ok": true, "blobId": "b2"}))),
            ("blob.read", Ok(json!({"data": "aGk=", "eof": true}))),
        ];
        assert_eq!(go(&["tls", "export"], large).0.unwrap(), "hi");
    }

    #[test]
    fn what_the_engine_refuses_is_an_error_and_a_list_is_a_table() {
        let (out, _) =
            go(&["tls", "generate"], vec![("tls.generate", Ok(json!({"ok": false, "error": "no openssl"})))]);
        assert_eq!(out.unwrap_err().line(), "bifurc: no openssl");
        let problems = json!({"ok": false, "problems": [{"message": "no start node"}]});
        let answers = vec![
            ("config.get", Ok(config())),
            ("entity.list", Ok(json!({"entities": [{"id": "f1", "name": "F"}]}))),
            ("flow.validate", Ok(problems.clone())),
        ];
        let (out, calls) = go(&["flows", "validate", "F"], answers);
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap(), problems);
        assert_eq!(payloads(&calls, "flow.validate"), [json!({"workspaceId": "w1", "id": "f1"})]);
    }

    #[test]
    fn cookies_clear_takes_a_domain() {
        let (out, calls) = go(
            &["cookies", "clear", "--domain", "a.com"],
            vec![("config.get", Ok(config())), ("cookies.clear", Ok(json!({"ok": true})))],
        );
        out.unwrap();
        assert_eq!(payloads(&calls, "cookies.clear"), [json!({"workspaceId": "w1", "domain": "a.com"})]);
    }

    #[test]
    fn the_help_of_a_group_is_written_from_its_rows() {
        let help = usage("cookies");
        assert!(
            help.starts_with(
                "  list\n      List the cookies the workspace's requests hold.\n  clear [--domain <text>]\n"
            ),
            "{help}"
        );
        assert!(help.contains("  configure --clear true|false\n"), "{help}");
        assert!(usage("sync").contains("  discard <path> --yes\n      Throw away a file's changes since the last sync. Cannot be undone: it runs only with --yes."));
        assert!(usage("tls").contains("  export [--out <file>]"));
        assert_eq!(usage("nonsense"), "");
    }
}
