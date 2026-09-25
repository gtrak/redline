//! Color/face definitions consumed by the ui components. Plain Rust
//! (no iocraft) so the theme can be unit-tested in isolation; the
//! ui layer converts `Color` to `iocraft::Color`.
//!
//! Issue 03 extends the face map with **syntax faces**: one `Face`
//! per tree-sitter token category. The index into `syntax_faces`
//! matches the index into `redline_syntax::highlight::HIGHLIGHT_FACES`.

use redline_syntax::highlight::HIGHLIGHT_FACES;

/// Stub palette of terminal colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum Color {
    /// Terminal default (no explicit color).
    #[default]
    Default,
    Black,
    DarkGrey,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Grey,
    White,
    /// A truecolor (24-bit) RGB value (issue-current-line-highlight): the
    /// 16-color palette cannot express the current-line tint's low-contrast
    /// shade (a few percent off the view background — the palette's nearest
    /// steps are ~25%+). Emitted as the SGR `48;2;r;g;b` truecolor escape.
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
}

impl Face {
    pub const fn new(foreground: Color, background: Color, bold: bool) -> Self {
        Self {
            foreground,
            background,
            bold,
        }
    }
}

/// All faces the ui can ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// The theme's display name (config-derived; used in the cache key).
    pub name: String,
    pub status_line: Face,
    pub status_line_active: Face,
    pub minibuffer: Face,
    pub prompt: Face,
    pub list_item: Face,
    pub list_item_selected: Face,
    pub view: Face,
    pub view_title: Face,
    pub preview: Face,
    // Magit / diff faces (issue 07).
    pub section_heading: Face,
    pub section_heading_selected: Face,
    pub diff_add: Face,
    pub diff_delete: Face,
    pub diff_context: Face,
    pub diff_hunk_header: Face,
    /// Log-entry rows (issue 08).
    pub log_commit: Face,
    /// Blame-row rows (issue 08): a dim prefix + text line.
    pub blame: Face,
    /// The region face (mark/region highlighting; plan 004 issue 03).
    /// High-contrast background distinct from the cursor/status-line blue.
    pub region: Face,
    /// The current-line face (issue-current-line-highlight): a SUBTLE
    /// background tint on the display row carrying the point. A BACKDROP —
    /// it is painted before the region face and the match/jump bands, so it
    /// loses to every existing highlight per cell. The background is a
    /// truecolor value a few percent off the view background (deliberately
    /// NOT a palette step — "not too blatant"); it is config-derived like
    /// every other face (theme-derived, set once at startup), so the shade
    /// is tunable without a rebuild.
    pub current_line: Face,
    /// The search-match faces (issue match-highlight): the dim face for
    /// ALL matches of the active query (`lazy-highlight`'s analogue) and
    /// the prominent face for the match the cursor is on (`isearch`'s
    /// analogue — a background band so the selected match reads as the
    /// cursor's context). The current face must be the more prominent of
    /// the two.
    pub search_match: Face,
    pub search_match_current: Face,
    /// The landing-highlight face (jump-highlight): the transient band on
    /// the symbol a jump lands on. DELIBERATELY distinct from the search
    /// faces — the meaning differs ("I just landed here" vs "these match
    /// the query"), and the user will want to tell them apart: a yellow
    /// background band (inverted-text style) that no other face uses,
    /// visible against the default background in both themes (the
    /// truecolor fade interpolates its RGB toward the base background).
    pub jump_highlight: Face,
    /// One face per tree-sitter token category, indexed by
    /// `HIGHLIGHT_FACES`. See `Theme::syntax_face`.
    pub syntax_faces: Vec<Face>,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark("default")
    }
}

impl Theme {
    /// Build the dark theme (the default palette) with the given name.
    pub fn dark(name: &str) -> Self {
        Self {
            name: name.to_string(),
            status_line: Face::new(Color::Grey, Color::Blue, true),
            status_line_active: Face::new(Color::White, Color::Blue, true),
            minibuffer: Face::new(Color::Grey, Color::Black, false),
            prompt: Face::new(Color::Yellow, Color::Black, true),
            list_item: Face::new(Color::White, Color::Black, false),
            list_item_selected: Face::new(Color::White, Color::Blue, true),
            view: Face::new(Color::White, Color::Black, false),
            view_title: Face::new(Color::Cyan, Color::Black, true),
            preview: Face::new(Color::Grey, Color::Black, false),
            section_heading: Face::new(Color::Cyan, Color::Black, true),
            section_heading_selected: Face::new(Color::White, Color::Blue, true),
            diff_add: Face::new(Color::Green, Color::Black, false),
            diff_delete: Face::new(Color::Red, Color::Black, false),
            diff_context: Face::new(Color::DarkGrey, Color::Black, false),
            diff_hunk_header: Face::new(Color::Magenta, Color::Black, true),
            log_commit: Face::new(Color::White, Color::Black, false),
            blame: Face::new(Color::DarkGrey, Color::Black, false),
            region: Face::new(Color::White, Color::DarkGrey, false),
            // 35/255 ≈ 13.7% above the view background (0,0,0) per channel.
            current_line: Face::new(Color::White, Color::Rgb(35, 35, 35), false),
            search_match: Face::new(Color::Grey, Color::Black, false),
            search_match_current: Face::new(Color::White, Color::Blue, true),
            jump_highlight: Face::new(Color::Black, Color::Yellow, true),
            syntax_faces: dark_syntax_faces(),
        }
    }

    /// Build the light theme (light background, dark text).
    pub fn light(name: &str) -> Self {
        let bg = Color::White;
        let fg = Color::Black;
        Self {
            name: name.to_string(),
            status_line: Face::new(Color::Black, Color::Grey, true),
            status_line_active: Face::new(Color::Black, Color::Blue, true),
            minibuffer: Face::new(Color::Black, bg, false),
            prompt: Face::new(Color::Magenta, bg, true),
            list_item: Face::new(fg, bg, false),
            list_item_selected: Face::new(Color::White, Color::Blue, true),
            view: Face::new(fg, bg, false),
            view_title: Face::new(Color::Blue, bg, true),
            preview: Face::new(Color::DarkGrey, bg, false),
            section_heading: Face::new(Color::Blue, bg, true),
            section_heading_selected: Face::new(Color::White, Color::Blue, true),
            diff_add: Face::new(Color::Green, bg, false),
            diff_delete: Face::new(Color::Red, bg, false),
            diff_context: Face::new(Color::DarkGrey, bg, false),
            diff_hunk_header: Face::new(Color::Magenta, bg, true),
            log_commit: Face::new(fg, bg, false),
            blame: Face::new(Color::DarkGrey, bg, false),
            region: Face::new(Color::Black, Color::Grey, false),
            // 35/255 ≈ 13.7% below the view background (255,255,255) per
            // channel (the dark theme's lift, mirrored).
            current_line: Face::new(fg, Color::Rgb(220, 220, 220), false),
            search_match: Face::new(Color::DarkGrey, bg, false),
            search_match_current: Face::new(Color::White, Color::Blue, true),
            jump_highlight: Face::new(Color::Black, Color::Yellow, true),
            syntax_faces: light_syntax_faces(),
        }
    }
}

impl Theme {
    /// The face for a tree-sitter highlight face index (into
    /// `HIGHLIGHT_FACES`). Out-of-range indices fall back to the
    /// default view face.
    pub fn syntax_face(&self, index: usize) -> Face {
        self.syntax_faces
            .get(index)
            .copied()
            .unwrap_or(self.view)
    }

    /// The theme's display name (used in the cache key).
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Default syntax faces: one `Face` per entry in `HIGHLIGHT_FACES`.
/// A dark-theme palette tuned for terminal readability.
fn dark_syntax_faces() -> Vec<Face> {
    let bg = Color::Black;
    HIGHLIGHT_FACES
        .iter()
        .map(|name| match *name {
            "comment" => Face::new(Color::DarkGrey, bg, false),
            "string" | "string.quote" => Face::new(Color::Yellow, bg, false),
            "function" | "function.builtin" => Face::new(Color::Cyan, bg, false),
            "keyword" => Face::new(Color::Magenta, bg, true),
            "type" | "type.builtin" => Face::new(Color::Blue, bg, false),
            "variable" | "variable.builtin" | "variable.other" => {
                Face::new(Color::White, bg, false)
            }
            "number" | "constant" | "constant.builtin" => {
                Face::new(Color::Yellow, bg, false)
            }
            "operator" => Face::new(Color::Grey, bg, false),
            "punctuation" => Face::new(Color::Grey, bg, false),
            "label" => Face::new(Color::Cyan, bg, true),
            "attribute" | "constructor" | "namespace" => Face::new(
                Color::Magenta,
                bg,
                false,
            ),
            "property" | "property.builtin" => Face::new(Color::White, bg, false),
            "tag" | "tag.builtin" => Face::new(Color::Red, bg, false),
            "regex" => Face::new(Color::Yellow, bg, false),
            "special" | "embedded" => Face::new(Color::Cyan, bg, true),
            "error" => Face::new(Color::Red, bg, true),
            "escape" => Face::new(Color::Yellow, bg, true),
            "delimiter" => Face::new(Color::Grey, bg, false),
            "module" => Face::new(Color::Magenta, bg, false),
            "boolean" => Face::new(Color::Yellow, bg, false),
            "text.title" => Face::new(Color::Cyan, bg, true),
            "text.literal" => Face::new(Color::Yellow, bg, false),
            "text.uri" => Face::new(Color::Cyan, bg, false),
            "text.reference" => Face::new(Color::Grey, bg, false),
            _ => Face::new(Color::White, bg, false),
        })
        .collect()
}

/// Light-theme syntax faces.
fn light_syntax_faces() -> Vec<Face> {
    let bg = Color::White;
    HIGHLIGHT_FACES
        .iter()
        .map(|name| match *name {
            "comment" => Face::new(Color::DarkGrey, bg, false),
            "string" | "string.quote" => Face::new(Color::Magenta, bg, false),
            "function" | "function.builtin" => Face::new(Color::Blue, bg, false),
            "keyword" => Face::new(Color::Magenta, bg, true),
            "type" | "type.builtin" => Face::new(Color::Cyan, bg, false),
            "variable" | "variable.builtin" | "variable.other" => {
                Face::new(Color::Black, bg, false)
            }
            "number" | "constant" | "constant.builtin" => {
                Face::new(Color::Magenta, bg, false)
            }
            "operator" => Face::new(Color::DarkGrey, bg, false),
            "punctuation" => Face::new(Color::DarkGrey, bg, false),
            "label" => Face::new(Color::Blue, bg, true),
            "attribute" | "constructor" | "namespace" => Face::new(
                Color::Magenta,
                bg,
                false,
            ),
            "property" | "property.builtin" => Face::new(Color::Black, bg, false),
            "tag" | "tag.builtin" => Face::new(Color::Red, bg, false),
            "regex" => Face::new(Color::Magenta, bg, false),
            "special" | "embedded" => Face::new(Color::Blue, bg, true),
            "error" => Face::new(Color::Red, bg, true),
            "escape" => Face::new(Color::Magenta, bg, true),
            "delimiter" => Face::new(Color::DarkGrey, bg, false),
            "module" => Face::new(Color::Magenta, bg, false),
            "boolean" => Face::new(Color::Magenta, bg, false),
            "text.title" => Face::new(Color::Blue, bg, true),
            "text.literal" => Face::new(Color::Magenta, bg, false),
            "text.uri" => Face::new(Color::Blue, bg, false),
            "text.reference" => Face::new(Color::DarkGrey, bg, false),
            _ => Face::new(Color::Black, bg, false),
        })
        .collect()
}

/// Process-wide current theme, set once at startup from config. The
/// ui components read it without plumbing the theme through props.
static CURRENT: std::sync::OnceLock<std::sync::Mutex<Theme>> = std::sync::OnceLock::new();

fn current_slot() -> &'static std::sync::Mutex<Theme> {
    CURRENT.get_or_init(|| std::sync::Mutex::new(Theme::default()))
}

pub fn set_current(theme: Theme) {
    *current_slot().lock().unwrap() = theme;
}

pub fn current() -> Theme {
    current_slot().lock().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_faces_covers_all_highlight_faces() {
        let t = Theme::default();
        assert_eq!(
            t.syntax_faces.len(),
            HIGHLIGHT_FACES.len(),
            "syntax_faces must have one entry per HIGHLIGHT_FACES entry"
        );
    }

    #[test]
    fn syntax_face_index_in_range() {
        let t = Theme::default();
        for i in 0..HIGHLIGHT_FACES.len() {
            // All in-range indices return a valid face.
            let _face = t.syntax_face(i);
        }
    }

    #[test]
    fn syntax_face_out_of_range_falls_back_to_view() {
        let t = Theme::default();
        let fallback = t.syntax_face(9999);
        assert_eq!(fallback, t.view, "out-of-range index should fall back to view face");
    }

    #[test]
    fn comment_face_is_distinguished() {
        let t = Theme::default();
        let comment_idx = HIGHLIGHT_FACES
            .iter()
            .position(|f| *f == "comment")
            .unwrap();
        let comment_face = t.syntax_face(comment_idx);
        assert_ne!(
            comment_face.foreground,
            t.view.foreground,
            "comment face should differ from the default view face"
        );
    }

    #[test]
    fn keyword_face_is_bold() {
        let t = Theme::default();
        let keyword_idx = HIGHLIGHT_FACES
            .iter()
            .position(|f| *f == "keyword")
            .unwrap();
        assert!(t.syntax_face(keyword_idx).bold, "keyword should be bold");
    }

    #[test]
    fn theme_name_is_stable() {
        let t = Theme::default();
        assert_eq!(t.name(), "default");
    }

    /// Issue match-highlight: the two search faces exist in both themes,
    /// and the SELECTED match is the more prominent one: it carries a
    /// background distinct from the view's (the inverse-video band the
    /// renderer paints under the matched text) and is bold, while the
    /// all-match face is a plain foreground-only variation.
    #[test]
    fn search_match_faces_distinguished_current_is_prominent() {
        let themes = [("dark", Theme::dark("d")), ("light", Theme::light("l"))];
        for (name, t) in themes {
            assert_ne!(
                t.search_match, t.view,
                "{name}: the all-match face must differ from the view face"
            );
            assert_ne!(
                t.search_match_current, t.search_match,
                "{name}: the selected face must be distinct from the all-match face"
            );
            assert_ne!(
                t.search_match_current.background, t.view.background,
                "{name}: the selected match needs its own background band"
            );
            assert!(
                t.search_match_current.bold,
                "{name}: the selected match is bold (the prominent one)"
            );
            assert!(
                !t.search_match.bold,
                "{name}: the all-match face stays unbolded (the dim one)"
            );
        }
    }

    /// issue-current-line-highlight: the current-line tint exists in both
    /// themes and is a LOW-CONTRAST shade: a background DISTINCT from the
    /// view background (it must be visible) but within a few percent of it
    /// per channel ("not too blatant" — a palette step like DarkGrey would
    /// read as a selected row). The shade is the theme's to own: the value
    /// lives in the face, and the file view's tint test changes it to prove
    /// the rendering follows the theme rather than a hard-coded constant.
    #[test]
    fn current_line_face_is_a_low_contrast_tint() {
        let themes = [("dark", Theme::dark("d")), ("light", Theme::light("l"))];
        for (name, t) in themes {
            let tint = t.current_line.background;
            let rgb = match tint {
                Color::Rgb(r, g, b) => (r as i32, g as i32, b as i32),
                other => panic!(
                    "{name}: the current-line tint must be a truecolor shade (the 16-color palette cannot express a few-percent delta), found {other:?}"
                ),
            };
            // The view background's nominal RGB (Black -> (0,0,0), White ->
            // (255,255,255) — the same nominal values the truecolor
            // interpolation paths in src/ui/mod.rs use).
            let base = match t.view.background {
                Color::Black => (0i32, 0, 0),
                Color::White => (255, 255, 255),
                other => panic!("{name}: the view background is expected to be plain, found {other:?}"),
            };
            assert_ne!(
                tint, t.view.background,
                "{name}: the tint must differ from the view background or it is invisible"
            );
            let rgb: [i32; 3] = rgb.into();
            let base: [i32; 3] = base.into();
            for (i, (r, b)) in rgb.iter().zip(base).enumerate() {
                let delta = (r - b).abs();
                assert!(
                    (10..=50).contains(&delta),
                    "{name}: channel {i} delta {delta} is outside the low-contrast band (10..=50 of 255): tint={tint:?} view_bg={:?}",
                    t.view.background
                );
            }
            assert_eq!(
                t.current_line.foreground, t.view.foreground,
                "{name}: the tint is a backdrop only — the text keeps the view's foreground"
            );
            assert!(
                !t.current_line.bold,
                "{name}: the tint must not be bold (nothing about the point's row is more prominent)"
            );
        }
    }

    /// jump-highlight: the landing face exists in both themes and is
    /// DISTINCT from the search faces, the view face, and the region face
    /// (the meaning differs: "I just landed here"), and it carries its own
    /// background band (visible against the default background — the view
    /// background) while the text stays readable (dark text on the band).
    #[test]
    fn jump_highlight_face_distinct_with_own_band() {
        let themes = [("dark", Theme::dark("d")), ("light", Theme::light("l"))];
        for (name, t) in themes {
            assert_ne!(
                t.jump_highlight, t.view,
                "{name}: the landing face must differ from the view face"
            );
            assert_ne!(
                t.jump_highlight, t.search_match,
                "{name}: the landing face must be distinct from the all-match face"
            );
            assert_ne!(
                t.jump_highlight, t.search_match_current,
                "{name}: the landing face must be distinct from the selected-match face"
            );
            assert_ne!(
                t.jump_highlight.background, t.view.background,
                "{name}: the landing highlight needs its own background band"
            );
            assert_ne!(
                t.jump_highlight.background, t.search_match_current.background,
                "{name}: the landing band must not be the cursor/search blue band"
            );
            assert!(
                t.jump_highlight.bold,
                "{name}: the landing band is bold (prominent)"
            );
        }
    }
}
