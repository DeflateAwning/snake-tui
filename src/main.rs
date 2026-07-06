use crossterm::{
    event::{poll, read, Event, KeyCode},
    style::{Color, Stylize},
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod game;
mod tui;
mod util;

use crate::game::{Direction, Game, GameState, Tile, FIELD_COLS, FIELD_LINES};
use crate::tui::{Renderer, Window};

struct ColorStruct {
    r: u8,
    g: u8,
    b: u8,
}

fn interp_value(v1: u8, v2: u8, t: f32) -> u8 {
    ((1.0 - t) * v2 as f32 + t * v1 as f32) as u8
}

impl ColorStruct {
    fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    fn interpolate(&self, c: Self, t: f32) -> Self {
        Self {
            r: interp_value(self.r, c.r, t),
            g: interp_value(self.g, c.g, t),
            b: interp_value(self.b, c.b, t),
        }
    }

    fn to_crossterm(&self) -> Color {
        Color::Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

fn snake_color(v: u16) -> Color {
    let t: f32 = 1.0 - (v as f32 / (FIELD_LINES * FIELD_COLS / 4) as f32);

    ColorStruct::new(66, 168, 50)
        .interpolate(ColorStruct::new(242, 230, 61), t)
        .to_crossterm()
}

fn draw_tile(window: &Window, x: u16, y: u16, t: &Tile) -> Result<(), io::Error> {
    let tile_ch = match t {
        Tile::Snake(v) => ' '.on(snake_color(*v)),
        Tile::Apple => ' '.on_red(),
        _ => ' '.blue(),
    };

    window.inner().pixel_styled(x * 2, y, tile_ch)?;
    window.inner().pixel_styled(x * 2 + 1, y, tile_ch)?;

    Ok(())
}

fn draw_game(window: &mut Window, game: &Game) -> Result<(), io::Error> {
    let title = format!("Apples: {}", game.points());
    window.set_title(&title);
    window.draw_borders()?;

    for y in 0..game.field().len() {
        for x in 0..game.field()[0].len() {
            draw_tile(window, x as u16, y as u16, &game.field()[y][x])?;
        }
    }

    Ok(())
}

fn draw_main_menu(window: &mut Window) -> Result<(), io::Error> {
    window.set_title("Snake");
    window.draw_borders()?;

    window.print_centered_str(2, "Snake game in the terminal")?;
    window.print_centered_str(3, "written in Rust")?;
    window.print_centered_str(5, "Use arrow keys ← → ↑ ↓, WASD or HJKL to move")?;
    window.print_centered_str(7, "Press ESC to exit")?;
    Ok(())
}

fn draw_end_menu(window: &mut Window, points: u16) -> Result<(), io::Error> {
    window.set_title("Game Over");
    window.draw_borders()?;

    let p = format!("You ate {} apples", points);
    window.print_centered_str(2, &p)?;
    window.print_centered_str(4, "Use arrow keys ← → ↑ ↓, WASD or HJKL to restart")?;

    Ok(())
}

fn main() -> io::Result<()> {
    let mut renderer = Renderer::new();

    renderer.init()?;

    let renderer = Rc::new(RefCell::new(renderer));

    let mut game = Game::new();
    let mut win = Window::centered(
        renderer.clone(),
        (FIELD_COLS * 2 + 2) as u16,
        (FIELD_LINES + 1) as u16,
    );

    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();
    let mut pending_moves: VecDeque<Direction> = VecDeque::new();

    'main: loop {
        loop {
            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            if timeout == Duration::ZERO {
                break;
            }

            if !poll(timeout).unwrap() {
                break;
            }

            if let Ok(event) = read() {
                if let Event::Key(key) = event {
                    let dir = match key.code {
                        KeyCode::Esc => break 'main,
                        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('w') => {
                            Some(Direction::Up)
                        }
                        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('s') => {
                            Some(Direction::Down)
                        }
                        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('a') => {
                            Some(Direction::Left)
                        }
                        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('d') => {
                            Some(Direction::Right)
                        }
                        _ => None,
                    };

                    if let Some(dir) = dir {
                        // Cap the buffer so a queued turn can't outlive the snake's
                        // ability to actually reach it before crashing into itself.
                        if pending_moves.len() < 2 {
                            pending_moves.push_back(dir);
                        }
                    }
                }
            }
        }

        if last_tick.elapsed() < tick_rate {
            continue;
        }

        last_tick = Instant::now();

        if let Some(dir) = pending_moves.pop_front() {
            game.move_to(dir);
        }

        game.step();

        match game.state() {
            GameState::Starting => draw_main_menu(&mut win)?,
            GameState::Started => draw_game(&mut win, &game)?,
            GameState::Ended => {
                renderer.borrow_mut().clear()?;
                draw_end_menu(&mut win, game.points())?;
            }
        }

        renderer.borrow_mut().present()?;
    }

    renderer.borrow_mut().dispose()?;

    Ok(())
}
