//! The main view (current buffer's plain text — highlighting arrives
//! with issue 03) and the `C-x C-b` buffer-list view.

use iocraft::prelude::*;

use crate::app::store::BufferRow;
use crate::theme;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct BufferViewProps {
    pub title: String,
    pub text: String,
}

/// The main view: the current buffer, titled, as plain text.
#[component]
pub fn BufferView(props: &BufferViewProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(background_color: face_bg(t.view)) {
                Text(
                    content: &props.title,
                    color: face_color(t.view_title),
                    weight: face_weight(t.view_title),
                )
                Text(content: &props.text, color: face_color(t.view))
                Text(
                    content: "C-x C-f find file · C-x b switch · C-x C-b list buffers · C-x k kill · C-c p p project · M-x commands · q quit",
                    color: face_color(t.preview),
                )
            }
        }
    }
}

#[derive(Default, Props)]
pub struct BufferListViewProps {
    pub rows: Vec<BufferRow>,
    pub selected: usize,
}

/// The `C-x C-b` list-buffers view: open buffers, MRU order, with the
/// current buffer marked.
#[component]
pub fn BufferListView(props: &BufferListViewProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(background_color: face_bg(t.view)) {
                Text(
                    content: "*list-buffers*",
                    color: face_color(t.view_title),
                    weight: face_weight(t.view_title),
                )
                #(props.rows.iter().enumerate().map(|(i, row)| {
                    let face = if i == props.selected {
                        t.list_item_selected
                    } else {
                        t.list_item
                    };
                    let marker = if row.current { "*" } else { " " };
                    element! {
                        Text(
                            key: format!("{i}"),
                            content: format!("{marker}{} ({} lines)", row.name, row.lines),
                            color: face_color(face),
                            weight: face_weight(face),
                        )
                    }
                }))
                Text(
                    content: "RET open · C-n/C-p or arrows move · q close",
                    color: face_color(t.preview),
                )
            }
        }
    }
}
