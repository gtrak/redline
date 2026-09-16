//! Shared renderer for the magit-family views (status, log, blame,
//! commit-diff, commit editor): a titled, theme-colored list of
//! [`MagitRow`]s plus a help line. Issue 07's status view is the original
//! consumer; issue 08 reuses it for log / blame / commit-diff / commit-editor.

use iocraft::prelude::*;

use crate::model::sections::MagitRow;
use crate::theme;
use crate::ui::diff_view::row_face;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct MagitRowsViewProps {
    pub title: String,
    pub rows: Vec<MagitRow>,
    pub help: String,
}

/// The generic magit-family buffer: a title, the colored rows, and a help
/// line. Each row's face comes from [`row_face`] (diff coloring is shared
/// with the status buffer).
#[component]
pub fn MagitRowsView(
    props: &MagitRowsViewProps,
    mut _hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(background_color: face_bg(t.view)) {
                Text(
                    content: &props.title,
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
                    content: &props.help,
                    color: face_color(t.preview),
                )
            }
        }
    }
}
