use crate::{ComponentUpdater, Hook, Hooks};

mod private {
    pub trait Sealed {}
    impl Sealed for crate::Hooks<'_, '_> {}
}

/// `UseCursorPosition` is a hook that declares where the terminal's hardware
/// cursor should sit at the end of each frame.
///
/// While a position is declared, the backend shows the cursor and moves it
/// there at the end of the frame — after the canvas's own cursor park,
/// before the frame's synchronized-update close — including on frames that
/// rewrote no canvas (a point move that leaves the canvas visually
/// unchanged). Passing `None` (or simply not calling this hook) restores the
/// backend's stock behavior: the cursor stays where the canvas park left it.
///
/// # Example
///
/// ```
/// # use iocraft::prelude::*;
/// #[component]
/// fn Example(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
///     hooks.use_cursor_position(Some((3, 1)));
///     element! {
///         Text(content: "cursor sits at column 3, row 1")
///     }
/// }
/// ```
pub trait UseCursorPosition: private::Sealed {
    /// Declares the hardware cursor position `(column, row)` (0-based,
    /// `crossterm` `MoveTo` order) for subsequent frames, or `None` to
    /// clear the declaration (the backend emits nothing after the canvas).
    ///
    /// The declaration takes effect on the next frame drawn; it never
    /// forces a re-render by itself.
    fn use_cursor_position(&mut self, position: Option<(u16, u16)>);
}

/// The hook state: the position declared by the most recent call. It is
/// pushed to the terminal during `post_component_update`, which runs
/// inside the render phase — after `?2026h`, before the frame's `?2026l`
/// close (the backend holds it until its own `end_frame` emit).
#[derive(Default)]
struct UseCursorPositionState {
    position: Option<(u16, u16)>,
}

impl Hook for UseCursorPositionState {
    // The position is carried into the next frame write; the hook never
    // causes a re-render on its own (default `poll_change` stays Pending).
    fn post_component_update(&mut self, updater: &mut ComponentUpdater) {
        // Static render path: no terminal, nothing to push.
        if let Some(terminal) = updater.terminal_mut() {
            terminal.set_cursor_position(self.position);
        }
    }
}

impl UseCursorPosition for Hooks<'_, '_> {
    fn use_cursor_position(&mut self, position: Option<(u16, u16)>) {
        let state = self.use_hook(UseCursorPositionState::default);
        state.position = position;
    }
}
