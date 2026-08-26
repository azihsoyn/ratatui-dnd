//! A kanban board sorted with the mouse or the keyboard.
//!
//! Also the JSON side of the crate's pitch: feed it a board and it
//! prints the sorted board back when you quit, so a program can hand a
//! pile of work to a human, let them arrange it, and read the result.
//!
//!     cargo run --example kanban              # a seeded board
//!     cargo run --example kanban -- board.json
//!
//! where board.json is `[{"title": "todo", "cards": ["…", …]}, …]`.
//!
//! Mouse: drag cards. Keyboard: arrows move, space lifts, arrows carry,
//! space drops, esc lets go. x marks a card; grab any marked card and
//! every marked card moves with it. q quits and prints the board.

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui_dnd::{Act, Sortable};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

const TICK: Duration = Duration::from_millis(100);
const CARD: u16 = 3;
const PITCH: u16 = CARD + 1;

#[derive(Serialize, Deserialize)]
struct Lane {
    title: String,
    cards: Vec<String>,
}

struct Card {
    id: u64,
    text: String,
}

struct App {
    titles: Vec<String>,
    lanes: Vec<Vec<Card>>,
    sort: Sortable<usize, u64>,
    /// The keyboard's place on the board: (lane, card).
    cursor: (usize, usize),
    /// Cards marked with `x`. The crate holds one handle; what a grab
    /// of it takes along is the model's business, so a marked card
    /// dragged anywhere brings the whole marked set.
    marked: HashSet<u64>,
}

impl App {
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
                        Card { id, text }
                    })
                    .collect()
            })
            .collect();
        App {
            titles,
            lanes,
            sort: Sortable::new(),
            cursor: (0, 0),
            marked: HashSet::new(),
        }
    }

    fn to_lanes(&self) -> Vec<Lane> {
        self.titles
            .iter()
            .zip(&self.lanes)
            .map(|(title, cards)| Lane {
                title: title.clone(),
                cards: cards.iter().map(|c| c.text.clone()).collect(),
            })
            .collect()
    }

    fn seed() -> Self {
        let lane = |title: &str, cards: &[&str]| Lane {
            title: title.into(),
            cards: cards.iter().map(|c| c.to_string()).collect(),
        };
        App::from(vec![
            lane(
                "todo",
                &[
                    "let a card be picked up",
                    "open a gap under the ghost",
                    "drop across columns",
                    "carry a card without a mouse",
                ],
            ),
            lane(
                "doing",
                &["hit-test the cards", "measure slots from real rects"],
            ),
            lane("done", &["draw three columns"]),
        ])
    }

    fn find(&self, id: u64) -> Option<(usize, usize)> {
        self.lanes
            .iter()
            .enumerate()
            .find_map(|(li, l)| l.iter().position(|c| c.id == id).map(|i| (li, i)))
    }

    /// Everything a grab of `id` takes along: the marked set when the
    /// grabbed card is marked, otherwise just the card. Board order,
    /// so a scattered selection lands in the order it was read.
    fn party(&self, id: u64) -> Vec<u64> {
        if !self.marked.contains(&id) {
            return vec![id];
        }
        self.lanes
            .iter()
            .flat_map(|l| l.iter())
            .filter(|c| self.marked.contains(&c.id))
            .map(|c| c.id)
            .collect()
    }

    /// Move the grabbed card — and whoever travels with it — and put
    /// the keyboard's cursor where they landed.
    fn apply(&mut self, id: u64, lane: usize, slot: usize) {
        let party = self.party(id);
        let mut moved = Vec::new();
        for l in &mut self.lanes {
            let (take, keep) = std::mem::take(l)
                .into_iter()
                .partition(|c: &Card| party.contains(&c.id));
            *l = keep;
            moved.extend(take);
        }
        let slot = slot.min(self.lanes[lane].len());
        for (n, card) in moved.into_iter().enumerate() {
            self.lanes[lane].insert(slot + n, card);
        }
        self.cursor = (lane, slot);
    }

    fn on_mouse(&mut self, m: event::MouseEvent) {
        match self.sort.on_mouse(m) {
            Act::Drop {
                key,
                container,
                slot,
            } => self.apply(key, container, slot),
            Act::Click(id) => {
                if let Some(at) = self.find(id) {
                    self.cursor = at;
                }
            }
            _ => {}
        }
    }

    /// True when the key asked to leave.
    fn on_key(&mut self, code: KeyCode) -> bool {
        if self.sort.held().is_some() {
            match code {
                KeyCode::Up | KeyCode::Char('k') => self.sort.shift(-1),
                KeyCode::Down | KeyCode::Char('j') => self.sort.shift(1),
                KeyCode::Left | KeyCode::Char('h') => self.sort.shift_container(-1),
                KeyCode::Right | KeyCode::Char('l') => self.sort.shift_container(1),
                KeyCode::Char(' ') | KeyCode::Char('　') | KeyCode::Enter => {
                    if let Some((id, lane, slot)) = self.sort.put() {
                        self.apply(id, lane, slot);
                    }
                }
                KeyCode::Esc => self.sort.cancel(),
                _ => {}
            }
            return false;
        }
        let (l, i) = self.cursor;
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor.1 = i.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor.1 = (i + 1).min(self.lanes[l].len().saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Char('h') if l > 0 => self.step_lane(l - 1),
            KeyCode::Right | KeyCode::Char('l') if l + 1 < self.lanes.len() => {
                self.step_lane(l + 1);
            }
            KeyCode::Char(' ') | KeyCode::Char('　') | KeyCode::Enter => {
                if let Some(card) = self.lanes[l].get(i) {
                    self.sort.lift(card.id);
                }
            }
            KeyCode::Char('x') => {
                if let Some(card) = self.lanes[l].get(i)
                    && !self.marked.remove(&card.id)
                {
                    self.marked.insert(card.id);
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => return true,
            _ => {}
        }
        false
    }

    fn step_lane(&mut self, to: usize) {
        self.cursor = (
            to,
            self.cursor.1.min(self.lanes[to].len().saturating_sub(1)),
        );
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let [board, status] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
        let areas = Layout::horizontal(vec![Constraint::Fill(1); self.lanes.len()])
            .spacing(1)
            .split(board);

        // Last frame's rects say where the gap goes; this frame's are
        // registered below, after the question is asked.
        let over = self.sort.over();
        let held = self.sort.held().copied();
        // Everything travelling with the grab leaves the flow, not just
        // the card in hand.
        let lifting = held.map(|id| self.party(id)).unwrap_or_default();

        for (li, cards) in self.lanes.iter().enumerate() {
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" {} · {} ", self.titles[li], cards.len()))
                .title_style(
                    Style::default()
                        .fg(Color::Gray)
                        .add_modifier(Modifier::BOLD),
                );
            let body = block.inner(areas[li]);
            f.render_widget(block, areas[li]);

            let slot = over.filter(|(l, _)| *l == li).map(|(_, s)| s);
            let mut spots: Vec<(u64, Rect)> = Vec::new();
            let mut row = 0u16;
            for (i, card) in cards.iter().enumerate() {
                if lifting.contains(&card.id) {
                    continue;
                }
                let mut shown = row;
                if slot.is_some_and(|s| row >= s as u16) {
                    shown += 1;
                }
                let r =
                    Rect::new(body.x, body.y + shown * PITCH, body.width, CARD).intersection(body);
                if r.height >= 2 {
                    let edge = if held.is_none() && self.cursor == (li, i) {
                        Color::White
                    } else if self.marked.contains(&card.id) {
                        Color::Magenta
                    } else {
                        Color::Gray
                    };
                    draw_card(f, r, &card.text, edge);
                    spots.push((card.id, r));
                }
                row += 1;
            }
            self.sort.container(li, body, &spots);

            if let Some(s) = slot {
                let r = Rect::new(body.x, body.y + s as u16 * PITCH, body.width, CARD)
                    .intersection(body);
                if r.height >= 2 {
                    match self.sort.carried().copied().and_then(|id| self.text_of(id)) {
                        // Carried by the keyboard: the card itself sits in
                        // the gap and moves with it.
                        Some(text) => draw_card(f, r, &label(&text, lifting.len()), Color::Yellow),
                        // Hanging from the mouse: the gap is a hole, the
                        // card rides the cursor.
                        None => f.render_widget(
                            Block::bordered()
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(Color::Cyan)),
                            r,
                        ),
                    }
                }
            }
        }

        if let Some(g) = self.sort.ghost(f.area())
            && let Some(text) = held.and_then(|id| self.text_of(id))
        {
            f.render_widget(Clear, g);
            draw_card(f, g, &label(&text, lifting.len()), Color::Yellow);
        }

        let hint = if self.sort.held().is_some() {
            " carrying: arrows move · space drops · esc lets go"
        } else {
            " drag with the mouse, or: arrows move · space lifts · x marks · q quits and prints JSON"
        };
        f.render_widget(
            Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
            status,
        );
    }

    fn text_of(&self, id: u64) -> Option<String> {
        self.find(id).map(|(l, i)| self.lanes[l][i].text.clone())
    }
}

/// What the card in hand says: its own text, and how many came along.
fn label(text: &str, lifting: usize) -> String {
    match lifting {
        0 | 1 => text.to_string(),
        n => format!("{text} ×{n}"),
    }
}

fn draw_card(f: &mut ratatui::Frame, r: Rect, text: &str, edge: Color) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(edge));
    let inner = block.inner(r);
    f.render_widget(block, r);
    f.render_widget(Paragraph::new(text), inner);
}

fn main() -> Result<()> {
    let app = match std::env::args().nth(1) {
        Some(path) => App::from(serde_json::from_str(&std::fs::read_to_string(path)?)?),
        None => App::seed(),
    };
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
    let out = run(&mut terminal, app);
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    println!("{}", serde_json::to_string_pretty(&out?)?);
    Ok(())
}

fn run(terminal: &mut ratatui::DefaultTerminal, mut app: App) -> Result<Vec<Lane>> {
    loop {
        terminal.draw(|f| app.render(f))?;
        if !event::poll(TICK)? {
            continue;
        }
        loop {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                        return Ok(app.to_lanes());
                    }
                    if app.on_key(k.code) {
                        return Ok(app.to_lanes());
                    }
                }
                Event::Mouse(m) => app.on_mouse(m),
                _ => {}
            }
            if !event::poll(Duration::ZERO)? {
                break;
            }
        }
    }
}
