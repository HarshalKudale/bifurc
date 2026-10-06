//! Patterns the screens share: the variable-aware editor, the name/value table, and page furniture.

pub mod autosave;
pub mod binary;
pub mod body_pane;
pub mod certificate;
pub mod curl_import;
pub mod dialogs;
pub mod entity_bar;
pub mod kv_table;
pub mod page;
pub mod parts_table;
pub mod request_bar;
pub mod stream_log;
pub mod tab_strip;
pub mod tree_drag;
pub mod var_editor;

pub use autosave::Autosave;
pub use body_pane::BodyPane;
pub use curl_import::{CURL_TAB_BADGE, CURL_TAB_TITLE, CurlImport, CurlImportEvent, CurlTarget};
pub use entity_bar::{EntityAction, EntityBar, FolderPicker, entity_bar};
pub use kv_table::{KeyValueEvent, KeyValueTable};
pub use request_bar::request_bar;
pub use parts_table::{FileData, Part, PartValue, PartsEvent, PartsTable};
pub use tab_strip::{StripTab, TabAction, TabStrip, tab_strip};
pub use var_editor::{Syntax, VarEditor};
