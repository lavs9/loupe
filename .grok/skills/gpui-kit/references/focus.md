# Focus and window

https://gpui-kit.com/docs/focus
https://gpui-kit.com/docs/window

## Four operations

| Call | Does | Does not |
| --- | --- | --- |
| `cx.focus_handle()` | Retain a target. Default `tab_stop` is false. | Move focus. |
| `.track_focus(&handle)` | Register the handle on this frame's element. | Move focus. |
| `handle.focus(window, cx)` | Move focus. | Create a node. |
| `.tab_stop(true)` on the **handle** | Tab order. | Immediate focus. |

Retain handles on the Entity. Re-attach the same handle in `render`. Creating a handle per frame breaks identity.

Implement `Focusable` and still `.track_focus` on the element that has `key_context`.

Never `focus()` from `render`.

## Lost focus

When the focused element stops rendering (hits list replaced by empty state), dispatch jumps to the tree root and shortcuts die. Register once in `new`:

```
let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
    let target = window
        .focus_lost_restore_target(cx)
        .unwrap_or_else(|| this.query_input.focus_handle(cx));
    target.focus(window, cx);
});
```

Attach the view handle on the shell that owns `key_context`.

## Search again

Results live under a second handle (`results_focus`) and `key_context("PhotoResults")`. After arrow selection, the query is not focused. Bind `FocusSearch` on both contexts. Handler: `query_input.focus_handle(cx).focus(window, cx)` then `select_all`. Kit `Input` owns `cmd-f` while the field itself is focused; bind `secondary-f` on the window context and `/` on results.

## Window

- `viewport_size()`: layout
- `bounds()`: global native frame
- `inner_window_bounds()`: excludes platform insets where supported
- `scale_factor()`: device pixels at a native boundary only. Do not multiply layout sizes by it.

`gpui_kit::init(cx)` before `open_window`.
