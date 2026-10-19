//! What the engine knows about newer versions (`update.status`, `event.update.changed`), and the words for it.

use serde_json::Value;

use crate::i18n::{t, t_with};

/// What a person does to get an update, which depends on how this copy was installed.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateAction {
    /// The Microsoft Store installs it; the address opens the Store's own updates page.
    Store(String),
    /// A package manager installs it, with this command.
    Command(String),
    /// A file to download and run.
    Download(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AvailableUpdate {
    /// The version the install ends up on.
    pub version: String,
    pub summary: Option<String>,
    pub notes_url: Option<String>,
    /// It fixes a security problem.
    pub critical: bool,
    /// The versions to install, in order. More than one when this version cannot go to the newest in one step.
    pub path: Vec<String>,
    /// The user asked not to be told about this version.
    pub skipped: bool,
}

impl AvailableUpdate {
    /// The step to take now.
    pub fn next(&self) -> &str {
        self.path.first().map_or(self.version.as_str(), String::as_str)
    }

    /// The order to install in, when there is more than one step.
    pub fn steps(&self) -> Option<String> {
        let (first, rest) = self.path.split_first().filter(|(_, rest)| !rest.is_empty())?;
        Some(t_with(
            "model.update.steps",
            &[("version", &self.version), ("first", first), ("rest", &rest.join(t("model.update.step_separator")))],
        ))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateStatus {
    /// How this copy was installed: `store`, `installer`, `winget`, `scoop`, `portable`, `appimage`, `deb`,
    /// `aur`, `tarball`, or `unknown` for a build.
    pub channel: String,
    pub checking: bool,
    /// The engine has read the list of releases at least once.
    pub checked: bool,
    /// Why the last check failed.
    pub error: Option<String>,
    /// This version is older than the oldest one still supported.
    pub unsupported: bool,
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
                path: available
                    .get("path")
                    .and_then(Value::as_array)
                    .map(|path| path.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default(),
                skipped: flag(available, "skipped"),
            })
        });
        let action = value.get("action").and_then(|action| match action.get("kind").and_then(Value::as_str)? {
            "store" => Some(UpdateAction::Store(text(action, "url")?)),
            "command" => Some(UpdateAction::Command(text(action, "command")?)),
            "download" => Some(UpdateAction::Download(text(action, "url")?)),
            _ => None,
        });
        Self {
            channel: text(value, "channel").unwrap_or_default(),
            checking: flag(value, "checking"),
            checked: value.get("checkedAt").is_some_and(|at| !at.is_null()),
            error: crate::engine::text_in(value, "error"),
            unsupported: flag(value, "unsupported"),
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
            "scoop" => "model.update.channel.scoop",
            "aur" => "model.update.channel.aur",
            "deb" => "model.update.channel.deb",
            "appimage" => "model.update.channel.appimage",
            "portable" | "tarball" => "model.update.channel.portable",
            "installer" | "dmg" => "model.update.channel.installer",
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
            "error": null, "unsupported": true,
            "action": { "kind": "download", "url": "https://bifurc.app/download" },
            "available": { "version": "0.6.0", "summary": "Faster.", "notesUrl": null, "critical": true,
                           "path": ["0.5.0", "0.6.0"], "next": "0.5.0", "skipped": false },
        }));
        assert!(status.checked && status.unsupported && !status.checking);
        assert_eq!(status.action, Some(UpdateAction::Download("https://bifurc.app/download".to_owned())));
        let update = status.announced().expect("an update");
        assert_eq!((update.version.as_str(), update.next(), update.critical), ("0.6.0", "0.5.0", true));
        assert_eq!(
            update.steps().as_deref(),
            Some("This version cannot be updated to 0.6.0 in one step. Install 0.5.0 first, then 0.6.0.")
        );
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
            "available": { "version": "0.6.0", "path": ["0.6.0"], "skipped": true },
            "action": { "kind": "command", "command": "scoop update bifurc" },
        }));
        assert!(status.available.is_some() && status.announced().is_none());
        assert_eq!(status.available.as_ref().and_then(AvailableUpdate::steps), None);
        assert_eq!(status.action, Some(UpdateAction::Command("scoop update bifurc".to_owned())));
    }
}
