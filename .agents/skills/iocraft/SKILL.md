---
name: iocraft
description: A Rust library for building beautiful terminal UIs (TUIs), CLI output, and text-based interfaces using a declarative, React-like API. Use when working with iocraft components, hooks, layout, styling, event handling, or render loops. Covers the element! macro, built-in components (View, Text, TextInput, Button, ScrollView), hooks (use_state, use_effect, use_future, use_terminal_events), flexbox layout, custom components, and testing patterns.
hidden: true
---

# iocraft Skill

## Overview

`iocraft` is a Rust library for building beautiful terminal UIs (TUIs), CLI output, and text-based interfaces using a declarative, React-like API. It uses `taffy` for flexbox layout and `crossterm` for terminal interaction.

Key characteristics:
- React/SwiftUI-inspired syntax via the `element!` macro
- Flexbox layout powered by `taffy`
- Stateful components with hooks
- Event-driven interactivity (keyboard, mouse, resize)
- Supports static output, inline render loops, and fullscreen apps
- Cross-platform: Unix and Windows terminals

## Core Concepts

### Elements and Components

UI is built from **elements** created with the `element!` macro. Elements are descriptions of components that haven't been instantiated yet.

```rust
use iocraft::prelude::*;

// Simple element with no props
element!(View)

// Element with props
element!(View(width: 80, height: 24, background_color: Color::Green))

// Element with children
element! {
    View(border_style: BorderStyle::Round) {
        Text(content: "Hello, world!")
    }
}
```

### The `element!` Macro

Syntax:
```rust
element! {
    ComponentName(prop1: value1, prop2: value2) {
        ChildComponent
        #(if condition { Some(element!(...)) } else { None })
        #(iterator.map(|item| element!(Item(key: item.id, ...))))
    }
}
```

- Props are set with `name: value` syntax inside parentheses
- Children go inside braces
- Use `#(expr)` for conditional children or iterators
- When rendering lists via iterators, always provide a `key` prop for stateful children
- Percentage values use the `pct` suffix: `width: 50pct`

### Rendering Output

Static (one-shot) rendering:
```rust
// Print to stdout
element!(...).print();

// Print to stderr
element!(...).eprint();

// Render to string
let s = element!(...).to_string();

// Render to Canvas with max width
let canvas = element!(...).render(Some(80));
```

Dynamic rendering (interactive):
```rust
// Inline render loop (renders below previous output)
element!(MyApp).render_loop().await.unwrap();

// Fullscreen render loop (alternate buffer, no scrolling)
element!(MyApp).fullscreen().await.unwrap();

// With configuration
element!(MyApp)
    .render_loop()
    .enable_mouse_capture()
    .ignore_ctrl_c()
    .await
    .unwrap();
```

**Note:** Dynamic rendering returns a future that must be awaited. Works with any async runtime (`tokio::main`, `smol::block_on`, `async_std::main`, etc.).

## Built-in Components

All built-in components are in `iocraft::components` and re-exported in `iocraft::prelude`.

### `View`

The fundamental layout container. Supports borders, background colors, and all flexbox properties.

```rust
element! {
    View(
        width: 80,
        height: 24,
        padding: 2,
        margin: 1,
        background_color: Color::DarkGrey,
        border_style: BorderStyle::Round,   // None, Single, Double, Round, Bold, DoubleLeftRight, DoubleTopBottom, Classic, Custom(...)
        border_color: Color::Blue,
        border_edges: Edges::Top | Edges::Left,  // default: all edges
        flex_direction: FlexDirection::Column,   // Row, Column, RowReverse, ColumnReverse
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        flex_wrap: FlexWrap::Wrap,
        gap: 1,
        column_gap: 2,
        row_gap: 1,
        overflow: Overflow::Hidden,  // Visible, Clip, Hidden, Scroll
        position: Position::Relative,  // Relative, Absolute
        top: 0, left: 0, right: 0, bottom: 0,  // for absolute positioning
    ) {
        Text(content: "Content")
    }
}
```

### `Text`

Renders text content.

```rust
element! {
    Text(
        content: "Hello",
        color: Color::Blue,
        weight: Weight::Bold,        // Normal, Bold, Light
        wrap: TextWrap::Wrap,        // Wrap, NoWrap
        align: TextAlign::Center,    // Left, Right, Center
        decoration: TextDecoration::Underline,  // None, Underline
        italic: true,
        invert: true,                // swap foreground/background
    )
}
```

`Text` automatically strips ANSI escape sequences from content.

### `TextInput`

Interactive text input field.

```rust
#[component]
fn InputExample(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let mut value = hooks.use_state(|| "".to_string());

    element! {
        View(
            border_style: BorderStyle::Round,
            border_color: Color::Blue,
        ) {
            TextInput(
                has_focus: true,
                value: value.to_string(),
                on_change: move |new_value| value.set(new_value),
                multiline: false,
                cursor_color: Some(Color::Grey),
            )
        }
    }
}
```

Props:
- `value: String` - current value
- `has_focus: bool` - whether to process keyboard input
- `on_change: HandlerMut<'static, String>` - callback when value changes
- `multiline: bool` - enables multiline with Enter key
- `auto_grow: bool` - with `multiline`, auto-grow height to fit wrapped content
- `cursor_color: Option<Color>`
- `handle: Option<Ref<TextInputHandle>>` - imperative control handle

`TextInputHandle` methods:
- `set_cursor_offset(usize)` - set cursor position (byte offset)
- `cursor_offset() -> usize` - get current cursor position

### `Button`

Clickable/pressable button.

```rust
element! {
    Button(
        handler: move || { /* do something */ },
        has_focus: true,
    ) {
        View(border_style: BorderStyle::Round, border_color: Color::Blue) {
            Text(content: "Click me!")
        }
    }
}
```

Triggered by: mouse click (fullscreen mode), Enter key, or Space key when focused.

### `Checkbox`

A controlled component for toggling a boolean value. New in iocraft 0.9.x (not present in 0.8).

Props:
- `checked: bool` - current checked state
- `on_change: HandlerMut<'static, bool>` - called with the new (toggled) value
- `has_focus: bool` - process keyboard input (Enter/Space)

### `ScrollView`

Scrollable content container with scrollbar.

```rust
element! {
    View(width: 80, height: 20) {
        ScrollView(
            auto_scroll: true,        // pin to bottom as content grows
            scroll_step: 3,           // lines per mouse wheel tick
            scrollbar: true,          // show scrollbar (default true)
            scrollbar_thumb_color: Color::White,
            scrollbar_track_color: Color::DarkGrey,
            keyboard_scroll: true,    // arrow keys, Page Up/Down, Home/End
            handle: Some(scroll_handle),
        ) {
            Text(content: "Lots of content...")
        }
    }
}
```

`ScrollViewHandle` methods:
- `scroll_to_top()` / `scroll_to_bottom()`
- `scroll_to(offset: i32)` / `scroll_by(delta: i32)`
- `scroll_offset() -> i32`
- `content_height() -> u16` / `viewport_height() -> u16`
- `is_auto_scroll_pinned() -> bool`

### `ContextProvider`

Passes context down the component tree.

```rust
struct UserInfo { name: String }

#[component]
fn Child(hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let info = hooks.use_context::<UserInfo>();
    element! { Text(content: format!("Hello, {}!", info.name)) }
}

fn main() {
    element! {
        ContextProvider(value: Context::owned(UserInfo { name: "Alice".into() })) {
            Child
        }
    }
    .print();
}
```

Context creation:
- `Context::owned(value)` - owned value
- `Context::from_ref(&value)` - immutable reference
- `Context::from_mut(&mut value)` - mutable reference

### `Fragment`

Groups children without adding a layout node.

```rust
element! {
    Fragment {
        Text(content: "A")
        Text(content: "B")
    }
}
```

### `MixedText`

Renders text with mixed styling segments.

### 0.9.1 Additions

- `Checkbox` component (see above).
- `Text` has a new `hyperlink: Option<String>` prop (OSC 8 hyperlink target).
- `TextInput` has a new `auto_grow: bool` prop (requires `multiline`) and accepts text style props (`color`, `weight`, `decoration`, `italic`, `invert`).
- `Color` and the event types (`KeyEvent`, `KeyCode`, `KeyModifiers`, `KeyEventKind`, `MouseButton`, `MouseEventKind`) are now owned by iocraft (mirroring crossterm's model) rather than re-exports; `From`/`Into` conversions to `crossterm::event`/`crossterm::style` types are provided when the `crossterm` feature is enabled.
- New variants: `Color::Reset`, `Overflow::Clip`, `MouseEventKind::ScrollLeft`/`ScrollRight`.

## Hooks

Hooks are called on `Hooks` object passed to components. They follow React's Rules of Hooks: must be called in the same order every render, no conditional/loop hook calls.

### `use_state`

```rust
let mut count = hooks.use_state(|| 0);
count.set(5);
count += 1;                    // works via AddAssign
let val = count.get();         // for Copy types
let val = count.to_string();   // works via Display
```

`State<T>` is `Copy` and causes re-render on mutation. Supports `get()`, `set()`, `read()`, `write()`, and arithmetic operators for numeric types.

### `use_effect`

```rust
hooks.use_effect(
    move || { println!("Effect ran!"); },
    (&dependency1, &dependency2),  // re-runs when hash changes
);
```

Use `()` as dependency to run exactly once after first mount.

### `use_future`

```rust
hooks.use_future(async move {
    tokio::time::sleep(Duration::from_secs(1)).await;
    state.set(true);
});
```

Spawns an async task bound to the component lifetime. Only spawned once. Use your runtime's timer/sleep API inside the future.

### `use_ref`

```rust
let my_ref = hooks.use_ref(|| MyType::new());
my_ref.write().some_method();
let val = my_ref.get();
```

`Ref<T>` provides interior mutability similar to `RefCell`.

### `use_ref_default`

```rust
let handle = hooks.use_ref_default::<TextInputHandle>();
```

Shorthand for `hooks.use_ref(T::default)`.

### `use_context` / `use_context_mut`

```rust
let ctx = hooks.use_context::<MyContextType>();
let mut ctx = hooks.use_context_mut::<MyContextType>();
let maybe = hooks.try_use_context::<MyContextType>();
```

### `use_terminal_events`

```rust
hooks.use_terminal_events(move |event| {
    match event {
        TerminalEvent::Key(KeyEvent { code: KeyCode::Char('q'), kind, .. }) if kind != KeyEventKind::Release => {
            should_exit.set(true);
        }
        TerminalEvent::Key(KeyEvent { code: KeyCode::Up, .. }) => { /* ... */ }
        TerminalEvent::Resize(width, height) => { /* ... */ }
        TerminalEvent::FullscreenMouse(mouse) => { /* ... */ }
        _ => {}
    }
});
```

`use_local_terminal_events` is similar but only receives events within the component's bounds (and translates mouse coordinates to local space).

Terminal event types (owned by iocraft in 0.9.x — they mirror crossterm's model, with `From`/`Into` conversions to crossterm types when the `crossterm` feature is enabled):
- `KeyEvent { code, modifiers, kind }` — constructible via `KeyEvent::new(kind, code)`
  - `KeyCode::Char(c)`, `KeyCode::Up/Down/Left/Right`, `KeyCode::Enter`, `KeyCode::Backspace`, `KeyCode::Delete`, `KeyCode::Home`, `KeyCode::End`, `KeyCode::PageUp`, `KeyCode::PageDown`, `KeyCode::Tab`, `KeyCode::BackTab`, `KeyCode::Esc`, etc.
  - `KeyEventKind::Press`, `Release`, `Repeat`
  - `KeyModifiers::CONTROL`, `SHIFT`, `ALT`, etc.
- `FullscreenMouseEvent { kind, row, column, modifiers }`
  - `MouseEventKind::Down(button)`, `Up(button)`, `Drag(button)`, `Moved`, `ScrollUp`, `ScrollDown`, `ScrollLeft`, `ScrollRight`
  - `MouseButton::Left`, `Right`, `Middle`

### `use_terminal_size`

```rust
let (width, height) = hooks.use_terminal_size();
```

Returns terminal dimensions as `(u16, u16)`.

### `use_component_rect`

```rust
let rect: Option<taffy::Rect<i32>> = hooks.use_component_rect();
```

Gets the component's canvas position and size as a `taffy::Rect<i32>` (`left`/`top`/`right`/`bottom`), or `None` on the first frame. Note that using this hook causes an immediate second render, or a re-render whenever the rect changes.

### `use_memo`

```rust
let computed = hooks.use_memo(
    move || expensive_computation(&data),
    &data,  // dependency - recomputes when hash changes
);
```

### `use_output`

```rust
let (stdout, stderr) = hooks.use_output();
stdout.println("Log message above the rendered TUI");
stderr.println("Log message above the rendered TUI (stderr)");
```

`use_output` returns a `(StdoutHandle, StderrHandle)` tuple; each handle offers `print` and `println`. Output is written above the rendered TUI without breaking the layout.

### `use_async_handler`

```rust
let handler = hooks.use_async_handler(move |value: String| async move {
    let result = fetch_data(value).await;
    result_state.set(result);
});
```

Creates an async handler function.

### `use_const`

```rust
let constant = hooks.use_const(|| "static string".to_string());
```

Caches a value computed once during the component's lifetime.

### 0.9.1 Additions

- `use_state_default::<T>()` - shorthand for `use_state(T::default)`.
- `use_const_default::<T>()` - shorthand for `use_const(T::default)`.
- `try_use_context_mut::<T>()` - mutable `Option<RefMut<T>>` context lookup.

## Custom Components

### Component Function Macro

```rust
#[component]
fn MyComponent(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let mut count = hooks.use_state(|| 0);
    element! {
        Text(content: format!("Count: {}", count))
    }
}
```

Components can take `hooks` and/or `props`:
```rust
#[derive(Default, Props)]
struct MyProps {
    label: String,
}

#[component]
fn MyComponent(props: &MyProps, mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    element! { Text(content: &props.label) }
}
```

Usage:
```rust
element! {
    View {
        MyComponent(label: "Hello")
        MyComponent(label: "World", key: "world")
    }
}
```

### Props Rules

- Derive `Default` and `Props` on props structs
- Mark library props as `#[non_exhaustive]` for forward compatibility
- Props must be covariant over lifetimes (the derive checks this)
- `key` is a reserved prop name and cannot be used
- Props implement `Send + Sync`
- Use references in props to avoid cloning: `name: &'a str`, `items: &'a Vec<T>`

### Low-level Component Implementation

For custom drawing or layout behavior, implement `Component` directly:

```rust
use iocraft::{Component, ComponentUpdater, ComponentDrawer, Hooks, Props};

#[derive(Default)]
struct MyCustomComponent;

#[derive(Default, Props)]
struct MyCustomProps {
    color: Option<Color>,
}

impl Component for MyCustomComponent {
    type Props<'a> = MyCustomProps;

    fn new(props: &Self::Props<'_>) -> Self {
        Self
    }

    fn update(&mut self, props: &mut Self::Props<'_>, _hooks: Hooks, updater: &mut ComponentUpdater) {
        updater.set_layout_style(taffy::style::Style {
            size: taffy::geometry::Size {
                width: taffy::style::Dimension::Length(10.0),
                height: taffy::style::Dimension::Length(5.0),
            },
            ..Default::default()
        });
        updater.update_children([], None);
    }

    fn draw(&mut self, drawer: &mut ComponentDrawer<'_>) {
        let mut canvas = drawer.canvas();
        let layout = drawer.layout();
        canvas.set_text(0, 0, "Hello", CanvasTextStyle::default());
    }
}
```

## Styling and Layout

### Layout Properties (available on `View` and via `with_layout_style_props`)

**Size:**
- `width`, `height`, `min_width`, `min_height`, `max_width`, `max_height`
- Types: `Size::Unset`, `Size::Auto`, `Size::Length(u32)`, `Size::Percent(f32)`
- Shorthand: `width: 10`, `width: 50pct`

**Spacing:**
- `padding`, `padding_top`, `padding_right`, `padding_bottom`, `padding_left`
- `margin`, `margin_top`, `margin_right`, `margin_bottom`, `margin_left`
- `gap`, `column_gap`, `row_gap`
- Types: `Padding::Unset`, `Length(u32)`, `Percent(f32)`

**Flexbox:**
- `display`: `Display::Flex` (default), `None`
- `flex_direction`: `Row`, `Column`, `RowReverse`, `ColumnReverse`
- `flex_wrap`: `Wrap`, `NoWrap`
- `flex_basis`: `Auto`, `Length(u32)`, `Percent(f32)`
- `flex_grow: f32`, `flex_shrink: Option<f32>`
- `align_items`: `FlexStart`, `FlexEnd`, `Center`, `Baseline`, `Stretch`
- `align_content`: `FlexStart`, `FlexEnd`, `Center`, `Stretch`, `SpaceBetween`, `SpaceAround`, `SpaceEvenly`
- `justify_content`: `FlexStart`, `FlexEnd`, `Center`, `SpaceBetween`, `SpaceAround`, `SpaceEvenly`

**Positioning:**
- `position`: `Relative`, `Absolute`
- `inset`, `top`, `right`, `bottom`, `left`
- Types: `Inset::Auto`, `Length(i32)`, `Percent(f32)`
- Absolute positioning uses negative values to overlap: `margin_top: -1`

**Overflow:**
- `overflow`, `overflow_x`, `overflow_y`: `Visible`, `Clip`, `Hidden`, `Scroll`

### Colors

Use `iocraft::Color` (re-exported in the prelude). It mirrors `crossterm::style::Color`, with `From`/`Into` conversions to crossterm's `Color` when the `crossterm` feature is enabled:
```rust
Color::Reset, Color::Black, Color::DarkGrey, Color::Grey, Color::White
Color::Red, Color::DarkRed, Color::Green, Color::DarkGreen
Color::Blue, Color::DarkBlue, Color::Cyan, Color::DarkCyan
Color::Magenta, Color::DarkMagenta, Color::Yellow, Color::DarkYellow
Color::Rgb { r, g, b }, Color::AnsiValue(u8)
```

### Text Styles

- `Weight::Normal`, `Weight::Bold`, `Weight::Light`
- `TextWrap::Wrap`, `TextWrap::NoWrap`
- `TextAlign::Left`, `TextAlign::Right`, `TextAlign::Center`
- `TextDecoration::None`, `TextDecoration::Underline`
- `italic: bool`, `invert: bool`

### Border Styles

- `BorderStyle::None`, `Single`, `Double`, `Round`, `Bold`
- `DoubleLeftRight`, `DoubleTopBottom`, `Classic`
- `Custom(BorderCharacters { top_left, top_right, bottom_left, bottom_right, left, right, top, bottom })`

## Testing

Use `mock_terminal_render_loop` to test interactive components:

```rust
use iocraft::prelude::*;
use futures::stream::{self, StreamExt};

#[component]
fn MyComponent(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let mut system = hooks.use_context_mut::<SystemContext>();
    let mut value = hooks.use_state(|| "".to_string());

    hooks.use_terminal_events(move |event| {
        if let TerminalEvent::Key(KeyEvent { code: KeyCode::Char('!'), kind: KeyEventKind::Press, .. }) = event {
            system.exit();
        }
    });

    element! { Text(content: &value) }
}

async fn test() {
    let canvases: Vec<_> = element!(MyComponent)
        .mock_terminal_render_loop(MockTerminalConfig::with_events(stream::iter(vec![
            TerminalEvent::Key(KeyEvent::new(KeyEventKind::Press, KeyCode::Char('!'))),
        ])))
        .collect()
        .await;

    assert_eq!(canvases.last().unwrap().to_string(), "!\n");
}
```

`MockTerminalConfig`:
- `MockTerminalConfig::default()` - default 80x24 terminal
- `MockTerminalConfig::with_events(stream)` - provide event stream

> **Dependency note (verified 2026-09):** `with_events` is generic over
> `futures_core::stream::Stream`, and collecting frames needs `StreamExt`
> — both come from the `futures`/`futures-util` crates. iocraft does **not**
> re-export `Stream`/`StreamExt` (its `iocraft::terminal` module, which holds
> the `TerminalEvents` stream type, is private), and `futures` is not a
> transitive-nameable dep. If your project doesn't list `futures` directly,
> you cannot drive `mock_terminal_render_loop`; verify keypress behavior at
> the app-state level and render frames statically (`element!(...).to_string()`
> takes `&mut self` — bind with `let mut app = ...`).

## System Context and Lifecycle

`SystemContext` is always available:

```rust
let mut system = hooks.use_context_mut::<SystemContext>();
system.exit();                  // stop the render loop
system.set_mouse_capture(true); // enable/disable mouse capture
```

## Event Handling Patterns

### Exit on 'q'
```rust
let mut system = hooks.use_context_mut::<SystemContext>();
let mut should_exit = hooks.use_state(|| false);

hooks.use_terminal_events(move |event| {
    if let TerminalEvent::Key(KeyEvent { code: KeyCode::Char('q'), kind, .. }) = event {
        if kind != KeyEventKind::Release {
            should_exit.set(true);
        }
    }
});

if should_exit.get() {
    system.exit();
}
```

### Focus Management
```rust
let mut focus = hooks.use_state(|| 0);
let max_focus = 3;

hooks.use_terminal_events(move |event| {
    if let TerminalEvent::Key(KeyEvent { code, kind, .. }) = event {
        if kind != KeyEventKind::Release {
            match code {
                KeyCode::Tab => focus.set((focus.get() + 1) % max_focus),
                KeyCode::BackTab => focus.set((focus.get() + max_focus - 1) % max_focus),
                _ => {}
            }
        }
    }
});
```

### Timer-based Updates
```rust
hooks.use_future(async move {
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        tick.set(tick.get() + 1);
    }
});
```

## Dependencies and Setup

Add to `Cargo.toml`:
```toml
[dependencies]
iocraft = "0.9.1"
tokio = { version = "1", features = ["full"] }  # or smol, async-std, etc.
```

Feature flags:
- `unstable-output-streams` - enables custom stdout/stderr handles for render loops (has crossterm caveats)

## Common Gotchas

<!-- not re-verified for 0.9.1 -->

1. **Hooks ordering:** Hooks must be called in the same order every render. No conditional hook calls. Panics will result from violations.

2. **State `Copy` semantics:** `State<T>` is `Copy`, so `let val = count.get()` works, but `count.set(new_val)` requires `mut` binding.

3. **Async runtime required:** `render_loop()` and `fullscreen()` return futures that must be awaited. Use `smol::block_on`, `tokio::main`, etc.

4. **Fullscreen exits on Ctrl+C by default.** The fullscreen render loop
   intercepts Ctrl+C and terminates the app UNLESS `.ignore_ctrl_c()` is
   called on the future (`fullscreen()` and `render_loop()` share
   `RenderLoopFuture`). Any app that binds C-c-prefixed key sequences MUST
   opt out, or the leading C-c kills the app before event handlers run.
   Verified against 0.9.1 `element.rs` (`RenderLoopFutureState.ignore_ctrl_c`).
4. **Terminal raw mode:** For interactive apps, call `hooks.use_terminal_events(|_| {})` even if you don't need events, so Ctrl+C is captured and terminal is restored properly on exit.

5. **Props references:** Props structs can contain borrowed data (`&'a str`, `&'a Vec<T>`) to avoid cloning. The `Props` derive checks covariance.

6. **Text ANSI stripping:** `Text` automatically strips ANSI escape codes from `content`. Use the `color`, `weight`, etc. props for styling instead.

7. **Mouse capture:** Mouse is automatically captured in fullscreen mode. For inline mode, call `.enable_mouse_capture()` on the render loop future.

8. **Overflow clipping:** Set `overflow: Overflow::Hidden` on containers to clip children that extend beyond bounds.

9. **Custom layout style props:** Use `#[with_layout_style_props]` on a props struct to automatically add all flexbox layout properties.

10. **Keys for lists:** Always provide `key` when rendering lists from iterators to maintain component state across renders.

## Architecture Pattern: Elm-style State Management

For complex TUIs with multiple screens, modals, and CRUD operations, an Elm/Redux-style architecture keeps the code testable and maintainable.

### Core Idea

Separate concerns into three layers:

1. **Event loop** (`use_terminal_events`) — reads keys, maps to `Action`, calls `update()`, executes `Effect`s
2. **Pure update function** — `update(state, action, data) -> (new_state, Vec<Effect>)`
3. **Effect executor** — spawns async tasks for side effects (DB, network, etc.)

```rust
// 1. Action enum — every user intent
pub enum Action {
    SelectNext, SelectPrev, Escape,
    GotoScreen(Screen), GoBack,
    PushModal(ModalState), PopModal,
    CreateItem { name: String },
    // ... etc
}

// 2. Effect enum — every async side effect
pub enum Effect {
    CreateItem { name: String, description: Option<String> },
    RefreshData,
    ShowToast(String),
    Quit,
}

// 3. Pure update function
pub fn update(state: AppState, action: Action, data: &AppData) -> (AppState, Vec<Effect>) {
    let mut new_state = state;
    let mut effects = Vec::new();
    match action {
        Action::Escape => {
            if !new_state.modal_stack.is_empty() {
                new_state.modal_stack.pop();  // close top modal
            } else if new_state.screen != Screen::Main {
                new_state.screen = new_state.screen_stack.pop().unwrap_or(Screen::Main);
            }
        }
        Action::CreateItem { name } => {
            new_state.modal_stack.pop();
            effects.push(Effect::CreateItem { name, description: None });
        }
        // ... etc
    }
    (new_state, effects)
}
```

### Modal Stack

Replace `Option<ModalState>` with `Vec<ModalState>` so `Esc` has a single unambiguous rule: **pop the top layer**. If the stack is empty, `Esc` navigates back.

```rust
pub struct AppState {
    pub modal_stack: Vec<ModalState>,
    // ...
}

impl AppState {
    pub fn current_modal(&self) -> Option<&ModalState> {
        self.modal_stack.last()
    }
    pub fn focus(&self) -> FocusArea {
        if !self.modal_stack.is_empty() { FocusArea::Modal }
        else if self.filter_mode { FocusArea::Filter }
        else { FocusArea::Main }
    }
}
```

### Key-to-Action Mapping

The event loop should be a thin, pure mapping function — no state mutation:

```rust
fn map_key_to_action(state: &AppState, code: KeyCode, data: &AppData) -> Action {
    // Precedence: filter mode > modal stack > screen-specific
    if state.filter_mode { /* filter keys */ }
    else if let Some(modal) = state.current_modal() { map_modal_key(modal, code) }
    else { map_screen_key(state.screen, code, state, data) }
}
```

### Async Effect Execution

Effects are executed outside the pure `update` function. Spawn them on your app's async runtime (`tokio::spawn`, `smol::spawn`, etc.):

```rust
// Effect executor — called from the event loop after update()
fn execute_effect(effect: Effect, ctx: &AppContext) {
    match effect {
        Effect::CreateItem { name, description } => {
            tokio::spawn(async move {
                let _ = db::create_item(name, description).await;
                // Trigger refresh to update UI
            });
        }
        // ...
    }
}
```

`use_future` itself is runtime-agnostic — it holds a future that runs on whichever async executor is active when the component renders.

### Timer-based Data Refresh

Use `use_future` with your runtime's sleep/timer to poll for external data changes:

```rust
hooks.use_future({
    let refresh_sender = refresh_sender.clone();
    async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let data = load_data().await;
            let _ = refresh_sender.send(data);
        }
    }
});
```

### Channels from async tasks to iocraft hooks

Use an async channel (e.g., `tokio::sync::mpsc` or `async_channel`) to send data from spawned tasks back into the component. Poll the receiver inside `use_future`:

```rust
// In use_future timer loop:
while let Ok(new_data) = channel_ref.write().receiver.try_recv() {
    dashboard_data.set(new_data);  // triggers re-render
}
```

> Note: `UnboundedReceiver::try_recv()` takes `&mut self`, so you need `let mut channel_ref = hooks.use_ref(...)` and call `.write()` instead of `.read()` on the ref.

### Quick Reference: Common Types

| Type | Values |
|------|--------|
| `Size` | `Unset`, `Auto`, `Length(u32)`, `Percent(f32)` |
| `Padding` | `Unset`, `Length(u32)`, `Percent(f32)` |
| `Margin` | `Unset`, `Auto`, `Length(i32)`, `Percent(f32)` |
| `Inset` | `Unset`, `Auto`, `Length(i32)`, `Percent(f32)` |
| `FlexBasis` | `Auto`, `Length(u32)`, `Percent(f32)` |
| `Gap` | `Unset`, `Length(u32)`, `Percent(f32)` |
| `Position` | `Relative`, `Absolute` |
| `Display` | `Flex`, `None` |
| `Overflow` | `Visible`, `Clip`, `Hidden`, `Scroll` |
| `FlexDirection` | `Row`, `Column`, `RowReverse`, `ColumnReverse` |
| `FlexWrap` | `Wrap`, `NoWrap` |
| `AlignItems` | `FlexStart`, `FlexEnd`, `Center`, `Baseline`, `Stretch` |
| `AlignContent` | `FlexStart`, `FlexEnd`, `Center`, `Stretch`, `SpaceBetween`, `SpaceAround`, `SpaceEvenly` |
| `JustifyContent` | `FlexStart`, `FlexEnd`, `Center`, `SpaceBetween`, `SpaceAround`, `SpaceEvenly` |
| `TextWrap` | `Wrap`, `NoWrap` |
| `TextAlign` | `Left`, `Right`, `Center` |
| `TextDecoration` | `None`, `Underline` |
| `Weight` | `Normal`, `Bold`, `Light` |
| `BorderStyle` | `None`, `Single`, `Double`, `Round`, `Bold`, `DoubleLeftRight`, `DoubleTopBottom`, `Classic`, `Custom(...)` |
| `Edges` | `Top`, `Right`, `Bottom`, `Left` (bitflags, combinable) |
