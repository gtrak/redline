//! Picker (helm-style) component: a prompt line, a nucleo-filtered
//! candidate list, and a live preview pane (helm follow-mode, always
//! on in redline v1). Consumers: the `M-x` command palette, the file
//! pickers (find-file / recent-files — preview is the file's first
//! page as plain text), the buffer pickers (switch/kill — preview is
//! the buffer's head), and the project switcher (preview is the root
//! path). The store owns the picker state (query, filtered list,
//! selection, preview); this component only renders it.
//!
//! The list is drawn with a canvas component: element! `Text` children
//! are flex-positioned, and we need exact row/column placement for the
//! list and the preview column.

use iocraft::{prelude::*, Component, ComponentDrawer, ComponentUpdater};

use crate::app::store::PickerCandidate;
use crate::theme;
use crate::ui::color;

#[derive(Default, Props)]
struct PickerCanvasProps {
    pub prompt: String,
    pub query: String,
    pub selected: usize,
    pub candidates: Vec<PickerCandidate>,
    pub total: usize,
    pub preview: String,
}

/// Canvas-backed picker surface: prompt row, candidate list rows,
/// preview column (selected candidate), and a count row.
struct PickerCanvas {
    prompt: String,
    query: String,
    selected: usize,
    candidates: Vec<PickerCandidate>,
    total: usize,
    preview: String,
}

impl PickerCanvas {
    fn from_props(props: &PickerCanvasProps) -> Self {
        Self {
            prompt: props.prompt.clone(),
            query: props.query.clone(),
            selected: props.selected,
            candidates: props.candidates.clone(),
            total: props.total,
            preview: props.preview.clone(),
        }
    }
}

impl Component for PickerCanvas {
    type Props<'a> = PickerCanvasProps;

    fn new(props: &Self::Props<'_>) -> Self {
        Self::from_props(props)
    }

    fn update(
        &mut self,
        props: &mut Self::Props<'_>,
        _hooks: Hooks,
        updater: &mut ComponentUpdater,
    ) {
        *self = Self::from_props(props);
        updater.set_layout_style(iocraft::taffy::style::Style {
            size: iocraft::taffy::geometry::Size {
                width: iocraft::taffy::style::Dimension::Percent(1.0),
                height: iocraft::taffy::style::Dimension::Length(12.0),
            },
            ..Default::default()
        });
    }

    fn draw(&mut self, drawer: &mut ComponentDrawer<'_>) {
        let layout = drawer.layout();
        let mut canvas = drawer.canvas();
        let t = theme::current();
        let w = layout.size.width.max(1.0) as usize;
        let h = layout.size.height.max(1.0) as usize;

        // Row 0: prompt + query.
        let prompt = format!("{}{}", self.prompt, self.query);
        canvas.set_text(0, 0, &truncate(&prompt, w), text_style(t.prompt.foreground, false, true));
        // Rows 1..h-2: candidate list (left) + preview (right) for the
        // selected candidate; last row: count.
        let list_h = h.saturating_sub(2);
        if list_h > 0 {
            let win = list_h.min(self.candidates.len());
            let start = self.selected.saturating_sub(win.saturating_sub(1));
            let split = ((w as i32) * 2 / 3) as isize;
            let preview_x = split + 1;
            let preview_w = (w as i32 - split as i32).saturating_sub(1);
            for (row, i) in (start..start + win).enumerate() {
                if let Some(candidate) = self.candidates.get(i) {
                    let selected = i == self.selected;
                    let face = if selected {
                        t.list_item_selected
                    } else {
                        t.list_item
                    };
                    let label = truncate(&candidate.display, split as usize);
                    canvas.set_text(
                        1,
                        1 + row as isize,
                        &label,
                        text_style(face.foreground, selected, selected),
                    );
                }
            }
            // Preview pane: the selected candidate's preview text, one
            // line per row (clipped to the visible rows).
            if preview_w > 1 {
                for (row, line) in self.preview.lines().take(list_h).enumerate() {
                    canvas.set_text(
                        preview_x,
                        1 + row as isize,
                        &truncate(line, preview_w as usize),
                        text_style(t.preview.foreground, false, false),
                    );
                }
            }
        }

        // Last row: candidate count, right-aligned.
        if h > 2 {
            let count = format!("{} of {}", self.candidates.len(), self.total);
            let x = (w as i32).saturating_sub(count.len() as i32 + 1) as isize;
            canvas.set_text(x, h as isize - 1, &count, text_style(t.minibuffer.foreground, false, false));
        }
    }
}

/// One canvas text style. `invert` (reverse-video) is the picker's cursor
/// highlight: it swaps the face's foreground (which becomes the bar) with
/// the parent background (which becomes the text color), so a bright
/// foreground face yields a bright, clearly-visible selection bar.
fn text_style(foreground: theme::Color, invert: bool, bold: bool) -> CanvasTextStyle {
    let mut style = CanvasTextStyle::default();
    style.color = Some(color(foreground));
    if invert {
        style.invert = true;
    }
    if bold {
        style.weight = Weight::Bold;
    }
    style
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max].iter().collect()
    }
}

#[derive(Default, Props)]
pub struct PickerProps {
    pub prompt: String,
    pub query: String,
    pub selected: usize,
    pub candidates: Vec<PickerCandidate>,
    pub total: usize,
    pub preview: String,
}

/// Renders the picker overlay from the store's picker state.
#[component]
pub fn Picker(props: &PickerProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(
            flex_shrink: 0.0,
            background_color: color(t.view.background),
        ) {
            PickerCanvas(
                prompt: props.prompt.clone(),
                query: props.query.clone(),
                selected: props.selected,
                candidates: props.candidates.clone(),
                total: props.total,
                preview: props.preview.clone(),
            )
        }
    }
}
