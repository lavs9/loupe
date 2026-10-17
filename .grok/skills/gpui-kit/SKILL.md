---
name: gpui-kit
description: Build and restyle GPUI Kit (gpui-kit 0.7.x) desktop windows. Layout, TitleBar, StatusBar, Tabs, Scrollable, focus, and theme. Use when editing GPUI Rust UI, gpui_kit, gpui-component, src/gui.rs, or when a GPUI window looks empty, misaligned, or ignores flex_1.
---

# GPUI Kit

Official docs are the API source of truth. Read them before inventing widgets.

- Design: https://gpui-kit.com/docs/design-guides
- Window: https://gpui-kit.com/docs/window
- Focus: https://gpui-kit.com/docs/focus
- Components: https://gpui-kit.com/component/
- Base: https://gpui-kit.com/base/
- Coding: https://gpui-kit.com/docs/coding-guides

Load [references/layout.md](references/layout.md) before any window shell or fill-the-window work.
Load [references/chrome.md](references/chrome.md) before TitleBar, tabs, status, or search fields.
Load [references/base.md](references/base.md) before lists, split panes, empty states, or loading placeholders.
Load [references/focus.md](references/focus.md) before keyboard routing.

## Non-negotiables

1. Compose kit components. Do not rebuild TitleBar, StatusBar, TabBar, Input, Scrollable, Dialog, Kbd, Empty, Alert, ToggleGroup, or a split pane from `div`s. Lists of any length use `v_virtual_list`.
2. `open_window` already wraps Base `Root`. Do not add another Root.
3. `h_flex()` is a row that **centers** on the cross axis. `v_flex()` is a column that **stretches**. They are not mirrors. A column in a bare `h_flex` does not fill height. Give it `h_full()` or give the row `items_stretch()`.
4. **Do not use `flex_1()` to fill the window.** GPUI view constraints are often unbounded on the block axis, so grow does nothing and content packs to the start. Size the work area from `window.viewport_size()`.
5. `px()` only for platform chrome and measured geometry (`TITLE_BAR_HEIGHT`, traffic-light inset, viewport remainder). Product spacing uses rem helpers (`p_4`, `gap_3`, `text_sm`) or `Size`.
6. Colors from `cx.theme()`. No hex at call sites except inside `Theme::update`.
7. `track_focus` on the element that owns `key_context`. Never call `focus()` from `render`. Restore with `cx.on_focus_lost`.
8. Prove layout with `ScrollHandle.bounds()` after a frame, written to a file. Screen Recording TCC blocks `screencapture` on this Mac.

## Window shell

```
let viewport = window.viewport_size();
let work_h = (f32::from(viewport.height) - TITLE_CHROME - SEARCH_BAR_HEIGHT - STATUS_BAR_HEIGHT).max(160.);

v_flex()
    .w(viewport.width)
    .h(viewport.height)
    .child(TitleBar::new().child(wordmark_and_gear))
    .child(search_bar.h(px(SEARCH_BAR_HEIGHT)))
    .child(work.h(px(work_h)).w_full())
    .child(StatusBar::new().left(status).right(shortcuts))
```

Open the window with `WindowOptions { ..TitleBar::window_options() }`.

macOS TitleBar is **34px** tall, **80px** left padding, traffic lights at **(9, 9)**. Align search field and status with that 80px spine.

## Verify

After a visual change: rebuild `--features gui`, restart the process, dump work-area bounds, confirm height ≈ `viewport.height - chrome`. Look at the live window. Do not claim a fill fix from compile alone.
