//! Root frame rendering: the view dispatch (`render_view`), the frame
//! assembly (`render_frame`), and the static-render seam (the
//! `StaticRenderWidth` context pin and the width-bounded static render
//! helper used by the app-side tests).

#[cfg(test)]
use std::sync::{Arc, Mutex};
use iocraft::prelude::*;
use iocraft::Size;

#[cfg(test)]
use crate::app::store::AppStore;

use crate::app::store::ViewId;
use crate::ui::blame_view::BlameView;
use crate::ui::commit_editor::CommitEditorView;
use crate::ui::file_view::FileView;
use crate::ui::home_view::HomeView;
use crate::ui::log_view::LogView;
use crate::ui::magit_status::MagitStatusView;
use crate::ui::picker::Picker;
use crate::ui::results_view::ResultsView;
use crate::ui::rows_view::MagitRowsView;
use crate::ui::transient_menu::TransientMenuView;
use crate::ui::tree::TreeSidebar;
use crate::ui::buffer_view::BufferListView;

#[cfg(test)]
use super::Root;
use super::snapshot::Snapshot;
use super::widgets::{Minibuffer, StatusLine};

/// Static-render width pin (loop-03): provided by the `render_at_width`
/// test helper. On the static render path there is no live terminal, so the
/// root View would otherwise resolve content-wide (`Size::Auto`) — which is
/// exactly why the df95113 width chain was invisible to static renders (an
/// unbounded canvas hides off-screen content). Providing this context makes
/// the root resolve at the given terminal width, so content-sized layers
/// that escape the window render off-screen and their absence is catchable.
#[derive(Clone, Copy)]
pub struct StaticRenderWidth(pub u16);
/// The view dispatch: the top-of-stack view for the current `ViewId`
/// (the root view is never rendered here — the root's own title/help
/// come from the view itself).
pub(super) fn render_view(snap: &Snapshot) -> Option<AnyElement<'static>> {
    match snap.view {
        ViewId::Home => Some(element! {
            HomeView(
                title: snap.home_title.clone(),
                rows: snap.home_rows.clone(),
                help: "C-x C-c quit · ? menu".to_string(),
            )
        }
        .into()),
        ViewId::Buffer => Some(element! {
            FileView(
                title: snap.file_view_title.clone(),
                rows: snap.file_view_rows.clone(),
                total_rows: snap.file_view_total_rows,
                top_line: snap.file_view_top_line,
                viewport_lines: snap.file_view_viewport_lines,
                changed_on_disk: snap.file_view_changed_on_disk,
                buffer_editable: snap.file_view_current_buffer_editable,
                region_lines: snap.region_lines,
                point_line: snap.file_view_point_line,
                notes_folded: snap.file_view_notes_folded,
            )
        }
        .into()),
        ViewId::BufferList => Some(element! {
            BufferListView(
                rows: snap.buffer_rows.clone(),
                selected: snap.buffer_list_selected,
            )
        }
        .into()),
        ViewId::MagitStatus => Some(element! {
            MagitStatusView(
                rows: snap.magit_rows.clone(),
                top_row: snap.magit_top_row,
                total_rows: snap.magit_total_rows,
            )
        }
        .into()),
        ViewId::Log => Some(element! {
            LogView(title: snap.log_title.clone(), rows: snap.log_rows.clone())
        }
        .into()),
        ViewId::Blame => Some(element! {
            BlameView(title: snap.blame_title.clone(), rows: snap.blame_rows.clone())
        }
        .into()),
        ViewId::CommitDiff => Some(element! {
            MagitRowsView(
                title: snap.commit_diff_title.clone(),
                rows: snap.commit_diff_rows.clone(),
                top_row: snap.commit_diff_top_row,
                total_rows: snap.commit_diff_total_rows,
                help: "commit diff (read-only) · C-n/C-p · C-v/M-v · M->/M-< · q back".to_string(),
            )
        }
        .into()),
        ViewId::CommitEditor => Some(element! {
            CommitEditorView(
                title: snap.commit_editor_title.clone(),
                rows: snap.commit_editor_rows.clone(),
                current_line: snap.commit_editor_cursor_line,
            )
        }
        .into()),
        ViewId::Search => Some(element! {
            ResultsView(
                title: snap.search_title.clone(),
                rows: snap.search_rows.clone(),
                top_row: snap.search_top_row,
                total_rows: snap.search_total_rows,
                selected_row: snap.search_selected_row,
                running: snap.search_running,
                error: snap.search_error.clone(),
            )
        }
        .into()),
    }
}

/// The frame assembly: the root `View` (row layout: optional tree
/// sidebar + the main view column with picker/menu overlays), the
/// `Minibuffer` line, and the `StatusLine`.
pub(super) fn render_frame(
    snap: Snapshot,
    main_view: Option<AnyElement<'static>>,
    term_w: Size,
    term_h: u32,
) -> impl Into<AnyElement<'static>> {
    element! {
        View(flex_direction: FlexDirection::Column, width: term_w, height: term_h) {
            // The row and the inner column pin their width to the root's
            // resolved width (100% each). Without this, a content-wide child
            // (the home view's NoWrap command rows, which can run well past
            // 80 cols) pins the column's width to its own content width and
            // full-width children (the picker's right-aligned count line)
            // render off-screen. In the static render path (root width Auto)
            // 100% resolves to the same content width as before — no change.
            View(flex_direction: FlexDirection::Row, flex_grow: 1.0f32, width: iocraft::Size::Percent(100.0)) {
                #(if snap.tree_visible {
                    Some(element! {
                        TreeSidebar(
                            rows: snap.tree_rows.clone(),
                            selected: snap.tree_selected,
                        )
                    })
                } else {
                    None
                })
                View(flex_direction: FlexDirection::Column, flex_grow: 1.0f32, width: iocraft::Size::Percent(100.0)) {
                    #(main_view)
                    #(if snap.picker {
                        Some(element! {
                            Picker(
                                prompt: snap.prompt,
                                query: snap.query,
                                selected: snap.selected,
                                candidates: snap.candidates.clone(),
                                total: snap.total,
                                preview: snap.preview,
                                viewport: snap.file_view_viewport_lines as u32,
                            )
                        })
                    } else {
                        None
                    })
                    #(if snap.menu_open && !snap.picker {
                        Some(element! {
                            TransientMenuView(
                                rows: snap.menu_rows.clone(),
                                height: snap.menu_height,
                            )
                        })
                    } else {
                        None
                    })
                }
            }
            Minibuffer(message: snap.message)
            StatusLine(
                project: snap.project,
                view: snap.view_name,
                mode: snap.buffer_mode,
                pending: snap.pending,
                activity: snap.activity,
                dirty: snap.dirty,
                which_function: snap.which_function,
                indexing: snap.indexing,
                resolving: snap.resolving,
                crate_indexing: snap.crate_indexing,
                searching: snap.searching,
                position: snap.position,
                annotations: snap.annotations,
                region_size: snap.region_size,
            )
        }
    }
}

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
