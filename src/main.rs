use clap::{Parser, ValueEnum};
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

#[derive(Parser, Debug)]
#[command(author, version, about = "A cross platform snake game running in the terminal")]
struct Args {
    /// Game refresh rate in Hz (ticks per second, higher means faster)
    #[arg(short = 'r', long, default_value_t = 10.0)]
    frequency: f64,

    /// Snake color
    #[arg(short, long, value_enum, default_value_t = SnakeColor::Green)]
    color: SnakeColor,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum SnakeColor {
    Green,
    Red,
    Blue,
    Yellow,
    Magenta,
    Cyan,
    White,
    Rainbow,
}

impl SnakeColor {
    fn to_style(self) -> SnakeStyle {
        match self {
            SnakeColor::Green => SnakeStyle::Solid(ColorStruct::new(66, 168, 50)),
            SnakeColor::Red => SnakeStyle::Solid(ColorStruct::new(200, 60, 60)),
            SnakeColor::Blue => SnakeStyle::Solid(ColorStruct::new(60, 110, 200)),
            SnakeColor::Yellow => SnakeStyle::Solid(ColorStruct::new(200, 180, 40)),
            SnakeColor::Magenta => SnakeStyle::Solid(ColorStruct::new(170, 60, 170)),
            SnakeColor::Cyan => SnakeStyle::Solid(ColorStruct::new(50, 170, 170)),
            SnakeColor::White => SnakeStyle::Solid(ColorStruct::new(210, 210, 210)),
            SnakeColor::Rainbow => SnakeStyle::Rainbow,
        }
    }
}

enum SnakeStyle {
    Solid(ColorStruct),
    Rainbow,
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let hh = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hh % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hh as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

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

fn snake_color(v: u16, style: &SnakeStyle, frame: u64) -> Color {
    match style {
        SnakeStyle::Solid(base) => {
            let t: f32 = 1.0 - (v as f32 / (FIELD_LINES * FIELD_COLS / 4) as f32);

            base.interpolate(ColorStruct::new(242, 230, 61), t)
                .to_crossterm()
        }
        SnakeStyle::Rainbow => {
            let hue = (v as f32 * 18.0 + frame as f32 * 6.0) % 360.0;
            let (r, g, b) = hsv_to_rgb(hue, 0.85, 0.95);
            ColorStruct::new(r, g, b).to_crossterm()
        }
    }
}

fn draw_tile(
    window: &Window,
    x: u16,
    y: u16,
    t: &Tile,
    snake_style: &SnakeStyle,
    frame: u64,
) -> Result<(), io::Error> {
    let tile_ch = match t {
        Tile::Snake(v) => ' '.on(snake_color(*v, snake_style, frame)),
        Tile::Apple => ' '.on_red(),
        _ => ' '.blue(),
    };

    window.inner().pixel_styled(x * 2, y, tile_ch)?;
    window.inner().pixel_styled(x * 2 + 1, y, tile_ch)?;

    Ok(())
}

fn draw_game(
    window: &mut Window,
    game: &Game,
    snake_style: &SnakeStyle,
    frame: u64,
) -> Result<(), io::Error> {
    let title = format!("Apples: {}", game.points());
    window.set_title(&title);
    window.draw_borders()?;

    for y in 0..game.field().len() {
        for x in 0..game.field()[0].len() {
            draw_tile(
                window,
                x as u16,
                y as u16,
                &game.field()[y][x],
                snake_style,
                frame,
            )?;
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
    let args = Args::parse();
    let snake_style = args.color.to_style();

    let mut renderer = Renderer::new();

    renderer.init()?;

    let renderer = Rc::new(RefCell::new(renderer));

    let mut game = Game::new();
    let mut win = Window::centered(
        renderer.clone(),
        (FIELD_COLS * 2 + 2) as u16,
        (FIELD_LINES + 1) as u16,
    );

    let tick_rate = Duration::from_secs_f64(1.0 / args.frequency);
    let mut last_tick = Instant::now();
    let mut pending_moves: VecDeque<Direction> = VecDeque::new();
    let mut frame: u64 = 0;

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
        frame = frame.wrapping_add(1);

        if let Some(dir) = pending_moves.pop_front() {
            game.move_to(dir);
        }

        game.step();

        match game.state() {
            GameState::Starting => draw_main_menu(&mut win)?,
            GameState::Started => draw_game(&mut win, &game, &snake_style, frame)?,
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
