//! The inline commit-message editor (issue 08): the first editable buffer.
//! The store owns the ropey-backed message + cursor and the magit bindings
//! (`C-c C-c` commit / `C-c C-k` abort); this renders the rows (comment lines
//! dimmed, the cursor line marked), via the shared [`MagitRowsView`].

use iocraft::prelude::*;

use crate::model::sections::MagitRow;
use crate::ui::rows_view::MagitRowsView;

#[derive(Default, Props)]
pub struct CommitEditorViewProps {
    pub title: String,
    pub rows: Vec<MagitRow>,
}

/// The commit editor. Edits and the commit/abort bindings are driven by the
/// store's key routing; the rows carry the cursor marker.
#[component]
pub fn CommitEditorView(
    props: &CommitEditorViewProps,
    _hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    element! {
        MagitRowsView(
            title: props.title.clone(),
            rows: props.rows.clone(),
            help: "type message · C-c C-c commit · C-c C-k abort · ESC/C-g abort".to_string(),
        )
    }
}
