//! The transient menu (issue 002): a bottom-of-frame overlay listing the
//! active view's bindings as `[key] description` (leaf commands) and
//! `KEY …` (prefix submenus), grouped by registry category. The store owns
//! the menu state and derives the rows from the keymap × registry (no
//! hand-written table); this component is a pure renderer over the store's
//! [`TransientMenuRow`]s.
//!
//! Drawn with a canvas component (like the picker) for exact row/column
//! placement of the key column and the description column.

use iocraft::{prelude::*, Component, ComponentDrawer, ComponentUpdater};

use crate::app::store::TransientMenuRow;
use crate::theme;
use crate::ui::color;

#[derive(Default, Props)]
struct TransientMenuCanvasProps {
    pub rows: Vec<TransientMenuRow>,
    pub height: u32,
}

struct TransientMenuCanvas {
    rows: Vec<TransientMenuRow>,
    height: u32,
}

impl TransientMenuCanvas {
    fn from_props(props: &TransientMenuCanvasProps) -> Self {
        Self {
            rows: props.rows.clone(),
            height: props.height,
        }
    }
}

impl Component for TransientMenuCanvas {
    type Props<'a> = TransientMenuCanvasProps;

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
                height: iocraft::taffy::style::Dimension::Length(self.height as f32),
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

        // Row 0: the menu's title bar.
        canvas.set_text(
            0,
            0,
            &truncate(" Transient menu · C-g to close ", w),
            text_style(t.view_title.foreground, false, true),
        );

        // Two-column flow (magit transients lay out key + description pairs
        // across the width). Category headers occupy their own full-width row
        // and start the following entries on a fresh row.
        const COLS: usize = 2;
        let cell_w = (w / COLS).max(1);
        let mut y: isize = 1;
        let mut x: usize = 0;
        for row in &self.rows {
            if y as usize >= h {
                break;
            }
            if row.is_header {
                // A new category never shares a row with the last entry of
                // the previous group (which can leave the row half-filled
                // when that group has an odd number of entries).
                if x != 0 {
                    y += 1;
                }
                if y as usize >= h {
                    break;
                }
                // The category label spans the full width of its own row.
                canvas.set_text(
                    0,
                    y,
                    &truncate(&format!("[{row_label}]", row_label = row.label), w),
                    text_style(t.diff_hunk_header.foreground, false, true),
                );
                y += 1;
                x = 0;
                continue;
            }
            let text = if row.is_prefix {
                format!("{} …", row.key_display)
            } else {
                format!("[{}] {}", row.key_display, row.label)
            };
            let face = if row.is_prefix {
                t.section_heading
            } else {
                t.view
            };
            canvas.set_text(
                (x * cell_w) as isize,
                y,
                &truncate(&text, cell_w),
                text_style(face.foreground, false, row.is_prefix),
            );
            x += 1;
            if x >= COLS {
                x = 0;
                y += 1;
            }
        }
    }
}

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
pub struct TransientMenuViewProps {
    pub rows: Vec<TransientMenuRow>,
    pub height: u32,
}

/// Renders the transient menu overlay from the store's derived rows.
#[component]
pub fn TransientMenuView(
    props: &TransientMenuViewProps,
    mut _hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_shrink: 0.0, background_color: color(t.view.background)) {
            TransientMenuCanvas(rows: props.rows.clone(), height: props.height)
        }
    }
}
