# GPUI Base

Catalog: https://gpui-kit.com/base/
Primitives: https://gpui-kit.com/base/primitives
Virtual list: https://gpui-kit.com/base/virtual-list

`gpui-base` is behavior without chrome. `gpui-component` paints it. Prefer Component for Loupe. Reach for Base when you need the unstyled primitive (`v_virtual_list`, `h_resizable`, `Scrollbar`).

`open_window` already wraps Base `Root`. Do not add another.

## VirtualList

`item_sizes` is required and is the item count. Hold `Rc<Vec<Size<Pixels>>>` and `VirtualListScrollHandle` on the entity. The closure receives a visible `Range<usize>` and must return one element per index. Give the list a **bounded** parent (`h(px(work_h))`). An unbounded parent lays out every item.

A vertical list reads `height` only. Grid sheets virtualize **rows**, not tiles: `row = index / cols`, `scroll_to_item(row, ScrollStrategy::Center)`. Arrow keys use `± cols`, not measured bounds.

```
v_virtual_list(cx.entity(), "hits", sizes, |this, range, _, cx| {
    range.map(|ix| this.row(ix, cx)).collect()
})
.track_scroll(&self.hits_scroll)
.size_full()
```

The list draws no scrollbar. Overlay `Scrollbar::vertical(&handle)` in a `relative()` parent.

`VirtualListScrollHandle::scroll_to_item` takes `(index, ScrollStrategy)`. It is not `ScrollHandle::scroll_to_item(index)`.

Uniform rows of the same height can use `gpui_kit::uniform_list`. Variable rows and photo sheets use VirtualList.

## Resizable

```
h_resizable("sheet")
    .size(px(work_h))   // cross axis: height of a horizontal group
    .child(resizable_panel().child(list))
    .child(
        resizable_panel()
            .size(px(320.))
            .size_range(px(240.)..px(520.))
            .child(preview),
    )
```

Do not pin a preview column with `w(px(320.))` on a flex child. `h_resizable` size is the cross axis, not `Styled::h`.

## Empty

Import `gpui_kit::component::empty::{Empty, ...}`. `gpui_kit::Empty` is a different type that renders nothing.

Wrap Empty in an explicit `h(px(work_h))`. Empty's internal `flex_1` does not fill an unbounded view.

## ToggleGroup

Mutually exclusive views (grid / list) keep one `checked(true)` in view state. `on_click` receives `Vec<bool>`. Segmented + outline + xsmall in the TitleBar.

## Alert / Skeleton / img

Index failures and search errors: `Alert::error(id, sentence)`. Missing thumbs: `Skeleton`. Photos: `img(path).id(...).object_fit(...).with_loading(...).with_fallback(...)`.
