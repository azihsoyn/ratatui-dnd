//! The crate without a terminal: JSON in, JSON out.
//!
//! Sorting here is data all the way down — rectangles in, hooks out —
//! so nothing about it needs a screen. This example is the proof, and
//! the machine's door: feed a board and a script of events on stdin,
//! and it replays them against the same kanban geometry the `kanban`
//! example draws, printing every hook and the final board as JSON.
//! A program — or an AI — can verify a drag without owning a terminal.
//!
//!     cargo run --example headless --features serde <<'EOF'
//!     {
//!       "board": [
//!         {"title": "todo", "cards": ["a", "b", "c"]},
//!         {"title": "done", "cards": []}
//!       ],
//!       "script": [
//!         {"down": [2, 1]}, {"drag": [30, 2]}, {"up": [30, 2]},
//!         {"lift": 2}, {"shift": 1}, "put"
//!       ]
//!     }
//!     EOF
//!
//! Mouse steps take a cell; lanes are 30 cells wide, cards 3 tall on a
//! 4-cell pitch, so lane `n` starts at x = n * 32. Keyboard steps are
//! `{"lift": id}`, `{"shift": n}`, `{"lane": n}`, `"put"`, `"cancel"`.

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui_dnd::{Act, Hook, Sortable};
use serde::{Deserialize, Serialize};

const LANE_W: u16 = 30;
const LANE_GAP: u16 = 2;
const CARD: u16 = 3;
const PITCH: u16 = CARD + 1;

#[derive(Serialize, Deserialize)]
struct Lane {
    title: String,
    cards: Vec<String>,
}

#[derive(Deserialize)]
struct Input {
    board: Vec<Lane>,
    script: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Down([u16; 2]),
    Drag([u16; 2]),
    Up([u16; 2]),
    Lift(u64),
    Shift(isize),
    Lane(isize),
    Put,
    Cancel,
}

#[derive(Serialize)]
struct Output {
    hooks: Vec<Hook<usize, u64>>,
    board: Vec<Lane>,
}

struct Board {
    titles: Vec<String>,
    lanes: Vec<Vec<(u64, String)>>,
    sort: Sortable<usize, u64>,
}

impl Board {
    fn from(input: Vec<Lane>) -> Self {
        let mut id = 0;
        let titles = input.iter().map(|l| l.title.clone()).collect();
        let lanes = input
            .into_iter()
            .map(|l| {
                l.cards
                    .into_iter()
                    .map(|text| {
                        id += 1;
                        (id, text)
                    })
                    .collect()
            })
            .collect();
        Board {
            titles,
            lanes,
            sort: Sortable::new(),
        }
    }

    fn to_lanes(&self) -> Vec<Lane> {
        self.titles
            .iter()
            .zip(&self.lanes)
            .map(|(title, cards)| Lane {
                title: title.clone(),
                cards: cards.iter().map(|(_, t)| t.clone()).collect(),
            })
            .collect()
    }

    /// What rendering would have told the sortable: the same lanes the
    /// `kanban` example draws, minus the screen.
    fn register(&mut self) {
        let held = self.sort.held().copied();
        for (li, cards) in self.lanes.iter().enumerate() {
            let x = li as u16 * (LANE_W + LANE_GAP);
            let body = Rect::new(x, 0, LANE_W, 200);
            let spots: Vec<(u64, Rect)> = cards
                .iter()
                .filter(|(id, _)| held != Some(*id))
                .enumerate()
                .map(|(row, (id, _))| (*id, Rect::new(x, row as u16 * PITCH, LANE_W, CARD)))
                .collect();
            self.sort.container(li, body, &spots);
        }
    }

    fn apply(&mut self, id: u64, lane: usize, slot: usize) {
        let Some((src, idx)) = self
            .lanes
            .iter()
            .enumerate()
            .find_map(|(li, l)| l.iter().position(|(k, _)| *k == id).map(|i| (li, i)))
        else {
            return;
        };
        let card = self.lanes[src].remove(idx);
        let slot = slot.min(self.lanes[lane].len());
        self.lanes[lane].insert(slot, card);
    }

    fn mouse(&mut self, kind: MouseEventKind, at: [u16; 2]) {
        let ev = MouseEvent {
            kind,
            column: at[0],
            row: at[1],
            modifiers: KeyModifiers::empty(),
        };
        if let Act::Drop {
            key,
            container,
            slot,
        } = self.sort.on_mouse(ev)
        {
            self.apply(key, container, slot);
        }
    }

    fn step(&mut self, step: Step) {
        match step {
            Step::Down(at) => self.mouse(MouseEventKind::Down(MouseButton::Left), at),
            Step::Drag(at) => self.mouse(MouseEventKind::Drag(MouseButton::Left), at),
            Step::Up(at) => self.mouse(MouseEventKind::Up(MouseButton::Left), at),
            Step::Lift(id) => self.sort.lift(id),
            Step::Shift(n) => self.sort.shift(n),
            Step::Lane(n) => self.sort.shift_container(n),
            Step::Put => {
                if let Some((id, lane, slot)) = self.sort.put() {
                    self.apply(id, lane, slot);
                }
            }
            Step::Cancel => self.sort.cancel(),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input: Input = serde_json::from_reader(std::io::stdin())?;
    let mut board = Board::from(input.board);
    let mut hooks = Vec::new();
    for step in input.script {
        // A frame would have drawn between any two events; registering
        // here is that frame.
        board.register();
        board.step(step);
        hooks.extend(board.sort.hooks());
    }
    let out = Output {
        hooks,
        board: board.to_lanes(),
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
