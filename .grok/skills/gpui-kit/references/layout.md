# Layout

## Layers

| Layer | Crate | Use |
| --- | --- | --- |
| GPUI | `gpui_kit::*` | `div`, `px`, `Window`, `FocusHandle` |
| Base | `gpui_kit::base` | `h_flex`, `v_flex`, `StyledExt`, unstyled behavior |
| Component | `gpui_kit::component` | styled TitleBar, Input, TabBar, StatusBar, Scrollable |

`gpui-component` sits on `gpui-base`. Prefer Component for product UI.

## Flex asymmetry

From `gpui-base` StyledExt:

```
h_flex() == div().flex().flex_row().items_center()
v_flex() == div().flex().flex_col()   // stretch, no items_center
```

A row of full-height columns:

```
h_flex()
    .items_stretch()
    .w_full()
    .h(px(work_h))
    .child(list.h_full())
    .child(pane.w(px(320.)).h_full())
```

Bare `h_flex` centers children vertically. A taller child is clipped at the top.

## Fill the window

`window.viewport_size()` is the drawable content size in logical pixels (Window guide). `window.bounds()` is the native frame in global coordinates. Use viewport for layout.

Do **not** fill with `flex_1()` / `size_full()` on nested views. Those need a definite parent constraint. The window's `AnyView` child often receives unbounded block size, so `flex_grow` leaves the child at content height and leftover space sits below the last sibling (empty Search packed to the top, footer floating).

Compute chrome, then set the work region height:

```
const TITLE_CHROME: f32 = 34.;        // TitleBar::TITLE_BAR_HEIGHT
const SEARCH_BAR_HEIGHT: f32 = 52.;
const STATUS_BAR_HEIGHT: f32 = 36.;
const INDEX_BANNER_HEIGHT: f32 = 40.;

fn work_height(window: &Window, extra: f32) -> f32 {
    let h = f32::from(window.viewport_size().height);
    (h - TITLE_CHROME - extra).max(160.)
}
```

Empty states: wrap kit `Empty` in `v_flex().w_full().h(px(work_h))`.

Scroll regions: give the scrolling element that same explicit `h`. Short content: `.overflow_y_scrollbar()`. Photo sheets and long lists: `v_virtual_list` with that height as the parent bound, then `Scrollbar::vertical`. A virtual list inside an unbounded parent lays out every item.

Horizontal `flex_1()` inside a row with a definite width is fine (search Input growing). Vertical grow to fill the window is not.

## Alignment spines

Same leading inset from TitleBar through search field through StatusBar. On macOS that inset is **80px** (`TITLE_BAR_LEFT_PADDING`). Do not mix 80, 20, and `px_5` on one vertical edge.

TitleBar children use `h_full().items_center()`. Do not `items_end` tabs against a wordmark.

## Prove it

Track a `ScrollHandle` on the work region. After ~500ms write:

```
viewport WxH work origin=X,Y size=WxH
```

Work height must be within a few pixels of `viewport.height - chrome`. Content-sized ~200px in an 800px window means the fill is still broken.
