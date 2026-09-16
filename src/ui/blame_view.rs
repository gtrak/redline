//! The blame view (issue 08): per-line hash/author/age prefix, aligned.
//! The store pre-computes the aligned rows; this only renders (shared
//! [`MagitRowsView`]).

use iocraft::prelude::*;

use crate::model::sections::MagitRow;
use crate::ui::rows_view::MagitRowsView;

#[derive(Default, Props)]
pub struct BlameViewProps {
    pub title: String,
    pub rows: Vec<MagitRow>,
}

/// The blame buffer (read-only).
#[component]
pub fn BlameView(props: &BlameViewProps, _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    element! {
        MagitRowsView(
            title: props.title.clone(),
            rows: props.rows.clone(),
            help: "blame (read-only) · q back".to_string(),
        )
    }
}
