# ratatui-dnd

Drag and drop for [ratatui](https://ratatui.rs): attach sorting to lists
you already draw, with mouse or keyboard.

Think [SortableJS](https://github.com/SortableJS/Sortable), but for the
terminal. This crate does not own your list, your kanban, or your grid —
you keep drawing them however you do, with raw `Layout`, ratatui's own
`List`, or any widget that can say where its rows ended up. You register
those rectangles each frame, and in return you get lift, a ghost that
hangs from the grab point, the gap where a drop would land, and the drop
itself, already resolved to a container and a slot.

- **Layout-agnostic.** Slots are measured from the rectangles you
  actually drew, so vertical lists of any row heights, horizontal
  strips, and row-major grids all work from one rule, without being
  told which one they are.
- **Containers are first-class.** Drag between columns, drop just past
  a border and still land in the nearest container — what SortableJS
  calls a group.
- **Keyboard, too.** Everything a mouse drag does can be done without a
  mouse: lift, step the item through slots and across containers, drop,
  or let go.
- **Machine-friendly.** With the `serde` feature the resolved events
  serialize, and the kanban example reads a board as JSON and prints
  the sorted board back — a program can hand a pile of work to a human,
  let them arrange it, and read the result.

## How it fits your code

```rust
use ratatui_dnd::{Act, Sortable};

let mut sort: Sortable<&'static str, u64> = Sortable::new();

// While rendering, say where things actually are. While something is
// held, leave it out — it is in the hand, not in a list.
sort.container("todo", lane_area, &rows); // rows: &[(id, Rect)]

// Ask where the gap should open, and where the ghost rides.
if let Some((lane, slot)) = sort.over() { /* draw a hole at slot */ }
if let Some(g) = sort.ghost(frame.area()) { /* draw the held row at g */ }

// Feed events through; a drop comes back resolved.
match sort.on_mouse(mouse_event) {
    Act::Drop { key, container, slot } => { /* move it in your model */ }
    Act::Click(key) => { /* a press that never became a drag */ }
    _ => {}
}

// The keyboard does the same job step by step.
sort.lift(id);
sort.shift(1);            // next slot (use ±columns to walk a grid by rows)
sort.shift_container(1);  // next container
if let Some((key, container, slot)) = sort.put() { /* move it */ }
```

Two layers, kept apart on purpose:

- `interact` — the ground floor: a drag state machine over raw mouse
  events (`Drag`), and a per-frame map from screen cells back to what
  was drawn on them (`Hits`). It knows nothing about sorting; scrubber
  heads, chart brushes, and resize handles sit on it just as well.
- `sort` — one tenant of that floor: containers, measured slots, the
  keyboard carry.

## Examples

```sh
cargo run --example kanban   # three lanes; also reads/prints the board as JSON
cargo run --example list     # ratatui's own List widget, made sortable
cargo run --example grid     # a row-major grid, same rule as a list
```

Every example speaks both mouse (drag things) and keyboard (arrows move,
space lifts and drops, esc lets go).

Mouse events reach a terminal program only when capture is on:

```rust
crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
```

## License

MIT or Apache-2.0, at your option.
