# Chrome and components

Catalog: https://gpui-kit.com/component/

## TitleBar

```
use gpui_kit::component::TitleBar;

WindowOptions {
    window_bounds: Some(...),
    window_min_size: Some(...),
    window_background: WindowBackgroundAppearance::Opaque,
    focus: true,
    ..TitleBar::window_options()
}

TitleBar::new().child(
    h_flex()
        .w_full()
        .h_full()
        .items_center()
        .child(wordmark)
        .child(div().flex_1())
        .child(trailing_actions) // ToggleGroup, gear Button with IconName::Settings
)
```

| Constant | macOS | Notes |
| --- | --- | --- |
| `TITLE_BAR_HEIGHT` | 34px | Do not use 52px custom bars |
| `TITLE_BAR_LEFT_PADDING` | 80px | Traffic lights |
| traffic lights | (9, 9) | Set by `TitleBar::window_options()` |

TitleBar owns drag and double-click. Do not bind ordinary clicks on the bar to rare commands.

## Tabs

```
use gpui_kit::component::tab::{Tab, TabBar};

TabBar::new("nav")
    .underline()
    .small()
    .selected_index(if search { 0 } else { 1 })
    .on_click(cx.listener(|this, index, window, cx| { ... }))
    .child(Tab::new().label("One"))
    .child(Tab::new().label("Two"))
```

Put TabBar inside TitleBar when the product has two peer screens. Loupe does not: Search is the window, Settings is a gear (`IconName::Settings`) in the TitleBar. Do not add a Search tab so the chrome stays one surface.

## Carousel

https://gpui-kit.com/component/carousel

```
let state = cx.new(|_| CarouselState::new(n).with_looping(true));
Carousel::new("welcome-carousel", &state)
    .child(CarouselContent::new(&state).w_full().h(px(72.)).children(slides))
    .child(CarouselPagination::new().children(dots));
```

Kit has looping, not autoplay. Advance with `select_next` on a timer while the empty Search is showing. `set_selected_index` is silent.

## StatusBar

```
use gpui_kit::component::status_bar::StatusBar;

StatusBar::new()
    .left(status_text)
    .right(h_flex().gap_3().children(key_chips))
```

Ghost `xsmall` Button only for clickable items. Plain strings for read-only labels.

## Search field

`Input::new(&self.query_input).cleanable(true)` already has a clear control. Do not add a second X.

Placeholder names the task (`Describe a photo`), not a sample query that looks typed.

## Scrollable

```
use gpui_kit::component::ScrollableElement;

v_flex()
    .id("hits")
    .h(px(work_h))
    .w_full()
    .track_scroll(&self.hits_scroll)
    .overflow_y_scrollbar()
```

`overflow_y_scroll()` has no kit scrollbar. Prefer `overflow_y_scrollbar()`. Any list that can grow uses `v_virtual_list` (see [base.md](base.md)). Overlay `Scrollbar::vertical` on a `relative()` parent.

## Toolbar

Panel headers and command strips. Transparent. Host supplies background. `content(div().flex_1())` pushes trailing actions. Icon-only buttons need `tooltip`.

## Empty / no results

`gpui_kit::component::empty::Empty` with `EmptyHeader` / `EmptyContent`. Parent is `h(px(work_h))`. Suggested queries are `Button::small().ghost()` in `EmptyContent`. One muted count line as `EmptyDescription` or a trailing child.

## Split preview

`h_resizable("sheet").size(px(work_h))` plus `resizable_panel` for the sheet and the preview. Do not hard-code the preview as a `w(px(320.))` flex sibling.

## View switch

`ToggleGroup` segmented outline in the TitleBar for contact sheet vs list. Keep exclusive state on the view.

## Theme

`Theme::change(ThemeMode::Dark, None, cx)` then `Theme::update` for product tokens. Hairlines `border`, selection `primary`, type `foreground` / `muted_foreground`. Loupe palettes are Darkroom and Paper. Do not take `gpui-themes` from pierreaubert/gpui-toolkit; that crate targets a different GPUI kit.
