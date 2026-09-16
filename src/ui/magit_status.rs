//! The magit status buffer view: renders the status section tree as a flat
//! list of theme-colored rows (section headings, collapsed/expanded, and
//! diff body lines). All fold/cursor logic lives in the model; this view is
//! a pure renderer.

use iocraft::prelude::*;

use crate::model::sections::MagitRow;
use crate::theme;
use crate::ui::diff_view::row_face;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct MagitStatusViewProps {
    pub rows: Vec<MagitRow>,
}

/// The magit status buffer (`C-x g`): the section tree, rendered with
/// collapsible sections and theme-colored diffs.
#[component]
pub fn MagitStatusView(
    props: &MagitStatusViewProps,
    mut _hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(background_color: face_bg(t.view)) {
                Text(
                    content: "*magit-status*",
                    color: face_color(t.view_title),
                    weight: face_weight(t.view_title),
                )
                #(props.rows.iter().enumerate().map(|(i, row)| {
                    let face = row_face(row.role, row.selected, &t);
                    element! {
                        Text(
                            key: i.to_string(),
                            content: &row.text,
                            color: face_color(face),
                            weight: face_weight(face),
                        )
                    }
                }))
                Text(
                    content: "s stage · u unstage · TAB fold · RET visit · n/p move · g refresh · q back",
                    color: face_color(t.preview),
                )
            }
        }
    }
}
