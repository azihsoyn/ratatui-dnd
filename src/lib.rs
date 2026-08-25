//! Sorting for things you already draw.
//!
//! [SortableJS](https://github.com/SortableJS/Sortable) for the terminal:
//! this crate does not own your list, your kanban, or your grid. You keep
//! drawing them however you do — with raw `Layout`, ratatui's `List`, or
//! any widget that can say where its rows ended up — and register those
//! rectangles each frame. In return you get lift, a ghost, the gap where
//! a drop would land, and the drop itself, driven by mouse or keyboard.
//!
//! Two layers, kept apart on purpose:
//!
//! - [`interact`] is the ground floor: a drag state machine over raw
//!   mouse events and a per-frame map from screen cells back to what was
//!   drawn on them. It knows nothing about sorting, and is meant to be
//!   just as usable for scrubbers, brushes, and resize handles.
//! - [`sort`] is one tenant of that floor: containers, insertion slots
//!   measured from the rectangles you actually drew, and a keyboard
//!   carry that does what a mouse drag does without a mouse.

pub mod interact;
pub mod sort;

pub use interact::{Did, Drag, Hits};
pub use sort::{Act, Sortable, slot};
