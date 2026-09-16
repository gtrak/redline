# 04 — File watching

Phase 2 · Browse · Depends on: 03

## Objective

A live repo: files agents write appear or refresh automatically, without
losing the reader's place. This is the vibecoding-critical issue.

## Key decisions

- **notify-debouncer** per open project; rapid events coalesce (agent
  churn must not flood the UI).
- **Project-change event bus**: file views reload; git status and the
  symbol index subscribe in later issues (05, 07).
- Locally-edited (light-edited) buffers are **never** auto-clobbered: show
  a "changed on disk" marker; `g` forces reload.

## Files

| Area | Change |
|---|---|
| `src/app/watcher.rs` | debounced watcher per project |
| `src/app/events.rs` | project-change bus + subscriptions |
| `src/ui/file_view.rs` | auto-reload with scroll anchor, conflict marker |
| config | `auto_reload` toggle, per-session suspend |

## Steps

1. Debounced watcher per project; map events onto the change bus.
2. FileView auto-reload keeping the scroll anchor (when the cursor line
   still exists).
3. Conflict path for edited buffers: marker + manual `g` reload.
4. Bus subscription stubs for git status (07) and symbol index (05).
5. Config toggle + suspend/resume command.

## Verification

- Edit a viewed file in another pane → the view updates within ~1s with
  scroll position preserved where possible.
- A locally-edited buffer shows the conflict marker instead of being
  clobbered; `g` reloads it.
- Many rapid saves coalesce — no UI freezes or event storms.
- Watcher can be disabled via config.
