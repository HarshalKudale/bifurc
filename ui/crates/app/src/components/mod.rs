//! Patterns the screens share: the variable-aware editor, the name/value table, and page furniture.

pub mod dialogs;
pub mod kv_table;
pub mod page;
pub mod var_editor;

pub use kv_table::{KeyValueEvent, KeyValueTable};
pub use var_editor::{Syntax, VarEditor};
