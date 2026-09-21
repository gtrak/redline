//! The static-render seam: the `StaticRenderWidth` context pin and
//! the width-bounded static render helper used by the app-side tests.

#[cfg(test)]
use std::sync::{Arc, Mutex};
#[cfg(test)]
use iocraft::prelude::*;

#[cfg(test)]
use crate::app::store::AppStore;

#[cfg(test)]
use super::Root;

/// Static-render width pin (loop-03): provided by the `render_at_width`
/// test helper. On the static render path there is no live terminal, so the
/// root View would otherwise resolve content-wide (`Size::Auto`) — which is
/// exactly why the df95113 width chain was invisible to static renders (an
/// unbounded canvas hides off-screen content). Providing this context makes
/// the root resolve at the given terminal width, so content-sized layers
/// that escape the window render off-screen and their absence is catchable.
#[derive(Clone, Copy)]
pub struct StaticRenderWidth(pub u16);
/// Test support (loop-03): a WIDTH-BOUNDED static render of the root frame.
///
/// iocraft's `to_string()` renders width-UNBOUNDED (`render(None)`), so a
/// content-sized layer that escapes the terminal width renders off-screen
/// yet invisibly (the pre-df95113 picker count-line class: the count line
/// landed at col 109+ while an unbounded `contains` assertion still passed
/// — that is why the 004-06a picker-canvas bug was invisible to the static
/// suite). `render_at_width(store, 80)` pins BOTH the layout wrapper and the
/// root View to 80 columns — the live-terminal contract of the PTY matrix
/// (80x24) — so anything that would be off-screen there is clipped here too,
/// and its ABSENCE from the 80-col text is catchable statically. This is
/// the layout-correctness discriminator the below-PTY flow twins assert on.
#[cfg(test)]
pub fn render_at_width(store: AppStore, width: usize) -> String {
    let mut app = element! {
        ContextProvider(value: Context::owned(StaticRenderWidth(width as u16))) {
            ContextProvider(value: Context::owned(Arc::new(Mutex::new(store)))) {
                Root
            }
        }
    };
    let canvas = app.render(Some(width));
    canvas.get_text(0, 0, width, canvas.height())
}
