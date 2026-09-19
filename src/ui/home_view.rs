//! The home view (plan 004 issue 06a): rendered when no buffer is current
//! (boot state, and after the last buffer is killed). The header shows the
//! project name + dirty counts + "redline"; the body shows the derived
//! command groups (the store's `home_rows` — live keymap × command
//! registry, the same grouping as the transient menu, so it can never
//! drift from the keymap; the store bounds the rows to the viewport, like
//! the file view bounds its lines); the footer is the standard help line.
//! A pure renderer over the store's derived values.

use iocraft::prelude::*;

use crate::app::store::TransientMenuRow;
use crate::theme;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct HomeViewProps {
    /// The header: `redline · {project}` + dirty counts.
    pub title: String,
    /// The derived body rows (category header rows + `key  docs` rows),
    /// already viewport-bounded by the store.
    pub rows: Vec<TransientMenuRow>,
    /// The footer help line.
    pub help: String,
}

#[component]
pub fn HomeView(props: &HomeViewProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden, width: iocraft::Size::Percent(100.0)) {
            View(flex_direction: FlexDirection::Column, flex_grow: 1.0_f32, background_color: face_bg(t.view)) {
                View(overflow: Overflow::Hidden) {
                    Text(
                        content: &props.title,
                        color: face_color(t.view_title),
                        weight: face_weight(t.view_title),
                        wrap: TextWrap::NoWrap,
                    )
                }
                #({
                    // Same face convention as the transient menu: `[category]`
                    // headers in the hunk-header face, entries in the plain
                    // view face. The store bounds the row list to the
                    // viewport (title + help aside), and each row is a
                    // NoWrap text in a hidden-overflow View (the status line's
                    // pattern): a long doc CLIPS on one row instead of
                    // wrapping, so the budget holds and the status line
                    // never gets pushed off the frame.
                    props.rows.iter().enumerate().map(|(i, row)| {
                        let (content, face) = if row.is_header {
                            (format!("[{}]", row.label), t.diff_hunk_header)
                        } else {
                            (format!("{}  {}", row.key_display, row.label), t.view)
                        };
                        element! {
                            View(key: i.to_string(), overflow: Overflow::Hidden) {
                                Text(
                                    content: content,
                                    color: face_color(face),
                                    weight: face_weight(face),
                                    wrap: TextWrap::NoWrap,
                                )
                            }
                        }
                    })
                })
                View(overflow: Overflow::Hidden) {
                    Text(
                        content: &props.help,
                        color: face_color(t.preview),
                        wrap: TextWrap::NoWrap,
                    )
                }
            }
        }
    }
}
