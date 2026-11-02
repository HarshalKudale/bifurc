//! What the engine knows about newer versions (`update.status`, `event.update.changed`), and the words for it.

use serde_json::Value;

use crate::i18n::t;

/// What a person does to get an update, which depends on how this copy was installed.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateAction {
    /// A package manager installs it, with this command.
    Command(String),
    /// A file to download and run.
    Download(String),
    /// Bifurc installs it itself (with winget, or the install script), then opens again.
    Install,
}

/// How far the reinstall is (`event.update.progress`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateProgress {
    /// `starting`, `downloading`, `installing`, `done` or `failed`.
    pub state: String,
    pub received: u64,
    pub total: u64,
    pub error: Option<String>,
}

impl UpdateProgress {
    pub fn from_value(value: &Value) -> Self {
        let number = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
        Self {
            state: value.get("state").and_then(Value::as_str).unwrap_or_default().to_owned(),
            received: number("received"),
            total: number("total"),
            error: value.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_owned),
        }
    }

    /// The reinstall is working, so another run must not start.
    pub fn busy(&self) -> bool {
        matches!(self.state.as_str(), "starting" | "downloading" | "installing")
    }

    /// The part of the download that is in, when its size is known.
    pub fn percent(&self) -> Option<u64> {
        (self.state == "downloading" && self.total > 0).then(|| (self.received * 100 / self.total).min(100))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AvailableUpdate {
    /// The version the install ends up on.
    pub version: String,
    pub summary: Option<String>,
    pub notes_url: Option<String>,
    /// It fixes a security problem.
    pub critical: bool,
    /// The user asked not to be told about this version.
    pub skipped: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateStatus {
    /// How this copy was installed: `store`, `installer`, `winget`, `portable`, `appimage`, `deb`,
    /// `aur`, `tarball`, or `unknown` for a build.
    pub channel: String,
    pub checking: bool,
    /// The engine has read the list of releases at least once.
    pub checked: bool,
    /// Why the last check failed.
    pub error: Option<String>,
    pub available: Option<AvailableUpdate>,
    pub action: Option<UpdateAction>,
}

impl UpdateStatus {
    pub fn from_value(value: &Value) -> Self {
        let text = |value: &Value, key: &str| {
            value.get(key).and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_owned)
        };
        let flag = |value: &Value, key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
        let available = value.get("available").filter(|available| available.is_object()).and_then(|available| {
            Some(AvailableUpdate {
                version: text(available, "version")?,
                summary: text(available, "summary"),
                notes_url: text(available, "notesUrl"),
                critical: flag(available, "critical"),
                skipped: flag(available, "skipped"),
            })
        });
        let action = value.get("action").and_then(|action| match action.get("kind").and_then(Value::as_str)? {
            "command" => Some(UpdateAction::Command(text(action, "command")?)),
            "download" => Some(UpdateAction::Download(text(action, "url")?)),
            "install" => Some(UpdateAction::Install),
            _ => None,
        });
        Self {
            channel: text(value, "channel").unwrap_or_default(),
            checking: flag(value, "checking"),
            checked: value.get("checkedAt").is_some_and(|at| !at.is_null()),
            error: crate::engine::text_in(value, "error"),
            available,
            action,
        }
    }

    /// The update the window announces on its own: one there is, that the user did not skip.
    pub fn announced(&self) -> Option<&AvailableUpdate> {
        self.available.as_ref().filter(|update| !update.skipped)
    }

    /// Who updates this copy, in a sentence.
    pub fn channel_note(&self) -> &'static str {
        t(match self.channel.as_str() {
            "store" => "model.update.channel.store",
            "winget" => "model.update.channel.winget",
            "aur" => "model.update.channel.aur",
            "deb" => "model.update.channel.deb",
            "appimage" => "model.update.channel.appimage",
            "portable" | "tarball" => "model.update.channel.portable",
            "dmg" => "model.update.channel.installer",
            _ => "model.update.channel.build",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_status_is_read_from_what_the_engine_reports() {
        let status = UpdateStatus::from_value(&json!({
            "currentVersion": "0.4.0", "channel": "installer", "enabled": true, "checking": false, "checkedAt": 5,
            "error": null,
            "action": { "kind": "download", "url": "https://bifurc.app/download" },
            "available": { "version": "0.6.0", "summary": "Faster.", "notesUrl": null, "critical": true,
                           "skipped": false },
        }));
        assert!(status.checked && !status.checking);
        assert_eq!(status.action, Some(UpdateAction::Download("https://bifurc.app/download".to_owned())));
        let update = status.announced().expect("an update");
        assert_eq!((update.version.as_str(), update.critical), ("0.6.0", true));
    }

    #[test]
    fn reinstalling_is_an_action_and_reports_its_progress() {
        let status = UpdateStatus::from_value(&json!({
            "available": { "version": "0.6.0" }, "action": { "kind": "install" },
        }));
        assert_eq!(status.action, Some(UpdateAction::Install));
        let progress = UpdateProgress::from_value(&json!({ "state": "downloading", "received": 25, "total": 100 }));
        assert!(progress.busy());
        assert_eq!(progress.percent(), Some(25));
        let failed = UpdateProgress::from_value(&json!({ "state": "failed", "error": "no checksum" }));
        assert!(!failed.busy() && failed.percent().is_none());
        assert_eq!(failed.error.as_deref(), Some("no checksum"));
    }

    #[test]
    fn an_engine_that_has_not_looked_reports_nothing() {
        let status = UpdateStatus::from_value(
            &json!({ "channel": "store", "checkedAt": null, "available": null, "action": null }),
        );
        assert!(!status.checked);
        assert_eq!((status.available.clone(), status.action.clone()), (None, None));
        assert!(status.channel_note().contains("Microsoft Store"));
    }

    #[test]
    fn a_skipped_update_is_known_but_not_announced() {
        let status = UpdateStatus::from_value(&json!({
            "available": { "version": "0.6.0", "skipped": true },
            "action": { "kind": "command", "command": "yay -Syu bifurc-bin" },
        }));
        assert!(status.available.is_some() && status.announced().is_none());
        assert_eq!(status.action, Some(UpdateAction::Command("yay -Syu bifurc-bin".to_owned())));
    }
}
