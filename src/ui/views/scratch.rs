//! Placeholder scratch view: shows something so the app is not blank.
//! Later issues replace it with the buffer/file view.

use iocraft::prelude::*;

use crate::theme;
use crate::ui::{face_bg, face_color, face_weight};

#[derive(Default, Props)]
pub struct ScratchViewProps {
    pub text: String,
}

#[component]
pub fn ScratchView(props: &ScratchViewProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_grow: 1.0_f32, overflow: Overflow::Hidden) {
            View(background_color: face_bg(t.view)) {
                Text(
                    content: "*scratch*",
                    color: face_color(t.view_title),
                    weight: face_weight(t.view_title),
                )
                Text(content: &props.text, color: face_color(t.view))
                Text(
                    content: "M-x palette · C-x i insert demo text · q quit · C-g cancel",
                    color: face_color(t.preview),
                )
            }
        }
    }
}
