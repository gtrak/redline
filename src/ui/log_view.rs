//! The magit log view (issue 08): a paged one-line commit list. Rendering is
//! shared with the other magit-family views via [`MagitRowsView`]; the store
//! pre-computes the rows (header, one row per commit, paging indicator).

use iocraft::prelude::*;

use crate::model::sections::MagitRow;
use crate::ui::rows_view::MagitRowsView;

#[derive(Default, Props)]
pub struct LogViewProps {
    pub title: String,
    pub rows: Vec<MagitRow>,
}

/// The log buffer. `n`/`p` page, arrows move the in-page selection, `RET`
/// opens the selected commit's diff (handled by the store; this only
/// renders).
#[component]
pub fn LogView(props: &LogViewProps, _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    element! {
        MagitRowsView(
            title: props.title.clone(),
            rows: props.rows.clone(),
            help: "n next · p prev · RET diff · q back".to_string(),
        )
    }
}
