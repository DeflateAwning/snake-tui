use chrono::Local;
use clap::Parser;
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
mod highscore;
mod tui;
mod util;

use crate::game::{Direction, Game, GameState, Tile, FIELD_COLS, FIELD_LINES};
use crate::highscore::{add_score, load_scores, qualifies, HighScoreEntry};
use crate::tui::{Renderer, Window};

const MAX_NAME_LEN: usize = 8;
const GRADIENT_LOOPS: f32 = 3.0;

enum PostGamePhase {
    Cooldown,
    NameEntry(String),
    Scoreboard(Vec<HighScoreEntry>),
}

#[derive(Parser, Debug)]
#[command(author, version, about = "A cross platform snake game running in the terminal")]
struct Args {
    /// Game refresh rate in Hz (ticks per second, higher means faster)
    #[arg(short = 'r', long, default_value_t = 10.0)]
    frequency: f64,

    /// Snake color: one of green, red, blue, yellow, orange, magenta, cyan,
    /// white, rainbow (static gradient across the whole body), party
    /// (animated rainbow), or a custom gradient made of hyphen-joined color names,
    /// e.g. "red-magenta-blue".
    #[arg(short, long, value_parser = parse_color_arg, default_value = "green")]
    color: ColorArg,
}

#[derive(Clone, Debug)]
struct ColorArg {
    raw: String,
    style: SnakeStyle,
}

fn named_color_rgb(name: &str) -> Option<(u8, u8, u8)> {
    match name.trim().to_lowercase().as_str() {
        "green" => Some((66, 168, 50)),
        "red" => Some((200, 60, 60)),
        "blue" => Some((60, 110, 200)),
        "yellow" => Some((200, 180, 40)),
        "orange" => Some((220, 120, 30)),
        "magenta" => Some((170, 60, 170)),
        "cyan" => Some((50, 170, 170)),
        "white" => Some((210, 210, 210)),
        _ => None,
    }
}

fn parse_style(s: &str) -> Result<SnakeStyle, String> {
    let s = s.trim();

    if s.is_empty() {
        return Err("color must not be empty".to_string());
    }

    match s.to_lowercase().as_str() {
        "rainbow" => return Ok(SnakeStyle::Rainbow),
        "party" => return Ok(SnakeStyle::Party),
        _ => {}
    }

    let stops = s
        .split('-')
        .map(|part| {
            named_color_rgb(part)
                .map(|(r, g, b)| ColorStruct::new(r, g, b))
                .ok_or_else(|| format!("unknown color '{part}'"))
        })
        .collect::<Result<Vec<ColorStruct>, String>>()?;

    match stops.len() {
        1 => Ok(SnakeStyle::Solid(stops[0])),
        _ => Ok(SnakeStyle::Gradient(stops)),
    }
}

fn parse_color_arg(s: &str) -> Result<ColorArg, String> {
    let style = parse_style(s)?;

    Ok(ColorArg {
        raw: s.trim().to_lowercase(),
        style,
    })
}

fn apple_color_for(raw: &str) -> Color {
    if raw.contains('-') || raw == "red" || raw == "rainbow" || raw == "party" {
        Color::White
    } else {
        Color::Red
    }
}

#[derive(Clone, Debug, PartialEq)]
enum SnakeStyle {
    Solid(ColorStruct),
    Gradient(Vec<ColorStruct>),
    Rainbow,
    Party,
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

#[derive(Copy, Clone, Debug, PartialEq)]
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
        SnakeStyle::Gradient(stops) => {
            let total_cells = (FIELD_LINES * FIELD_COLS) as f32;
            let segments = stops.len() - 1;
            let total_virtual_segments = segments as f32 * GRADIENT_LOOPS;
            let pos = (v as f32 / total_cells).clamp(0.0, 1.0) * total_virtual_segments;
            let idx_global = (pos.floor() as usize).min(total_virtual_segments as usize - 1);
            let local_t = pos - idx_global as f32;
            let idx = idx_global % segments;

            stops[idx + 1].interpolate(stops[idx], local_t).to_crossterm()
        }
        SnakeStyle::Rainbow => {
            let total_cells = (FIELD_LINES * FIELD_COLS) as f32;
            let hue = (v as f32 / total_cells) * 360.0;
            let (r, g, b) = hsv_to_rgb(hue, 0.85, 0.95);
            ColorStruct::new(r, g, b).to_crossterm()
        }
        SnakeStyle::Party => {
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
    apple_color: Color,
    frame: u64,
) -> Result<(), io::Error> {
    let tile_ch = match t {
        Tile::Snake(v) => ' '.on(snake_color(*v, snake_style, frame)),
        Tile::Apple => ' '.on(apple_color),
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
    apple_color: Color,
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
                apple_color,
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

fn draw_end_menu(window: &mut Window, points: u16, duration: Duration) -> Result<(), io::Error> {
    window.set_title("Game Over");
    window.draw_borders()?;

    let p = format!("You ate {} apples", points);
    let d = format!("You survived {:.1}s", duration.as_secs_f32());
    window.print_centered_str(2, &p)?;
    window.print_centered_str(3, &d)?;
    window.print_centered_str(5, "Use arrow keys ← → ↑ ↓, WASD or HJKL to restart")?;

    Ok(())
}

fn draw_name_entry(window: &mut Window, points: u16, name: &str) -> Result<(), io::Error> {
    window.set_title("Game Over");
    window.draw_borders()?;

    let p = format!("You ate {} apples", points);
    window.print_centered_str(2, &p)?;
    window.print_centered_str(4, "Enter your name for the high score board:")?;
    window.print_centered_str(6, &format!("{}_", name))?;
    window.print_centered_str(8, "Press Enter to confirm")?;

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max).collect()
    } else {
        s.to_string()
    }
}

fn draw_scoreboard(window: &mut Window, scores: &[HighScoreEntry]) -> Result<(), io::Error> {
    window.set_title("High Scores");
    window.draw_borders()?;

    if scores.is_empty() {
        window.print_centered_str(2, "No scores yet")?;
    } else {
        for (i, entry) in scores.iter().enumerate() {
            let row = format!(
                "{:>2}. {:<8} {:>3}pts {:<7} {:>4.1}Hz {}",
                i + 1,
                truncate(&entry.name, MAX_NAME_LEN),
                entry.score,
                entry.color,
                entry.speed,
                entry.date
            );
            window.print_str(2, (i + 2) as u16, &row)?;
        }
    }

    let footer_row = scores.len().max(1) as u16 + 3;
    window.print_centered_str(footer_row, "Press an arrow key, WASD or HJKL to play again")?;

    Ok(())
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let snake_style = args.color.style.clone();
    let apple_color = apple_color_for(&args.color.raw);

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
    let end_screen_lock = Duration::from_secs(4);
    let mut last_tick = Instant::now();
    let mut pending_moves: VecDeque<Direction> = VecDeque::new();
    let mut frame: u64 = 0;
    let mut game_start: Option<Instant> = None;
    let mut ended_at: Option<Instant> = None;
    let mut post_game: Option<PostGamePhase> = None;

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
                    match &mut post_game {
                        Some(PostGamePhase::NameEntry(name)) => match key.code {
                            KeyCode::Esc => break 'main,
                            KeyCode::Enter => {
                                let final_name = if name.trim().is_empty() {
                                    "Anonymous".to_string()
                                } else {
                                    name.trim().to_string()
                                };
                                let entry = HighScoreEntry {
                                    name: final_name,
                                    score: game.points(),
                                    color: args.color.raw.clone(),
                                    speed: args.frequency,
                                    date: Local::now().format("%Y-%m-%d").to_string(),
                                };
                                let scores = add_score(entry);
                                post_game = Some(PostGamePhase::Scoreboard(scores));
                            }
                            KeyCode::Backspace => {
                                name.pop();
                            }
                            KeyCode::Char(c)
                                if !c.is_control() && name.chars().count() < MAX_NAME_LEN =>
                            {
                                name.push(c);
                            }
                            _ => {}
                        },
                        Some(PostGamePhase::Scoreboard(_)) => {
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
                                pending_moves.push_back(dir);
                                post_game = None;
                            }
                        }
                        Some(PostGamePhase::Cooldown) | None => {
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
            }
        }

        if last_tick.elapsed() < tick_rate {
            continue;
        }

        last_tick = Instant::now();
        frame = frame.wrapping_add(1);

        let prev_state = game.state();
        let can_restart = prev_state != GameState::Ended || post_game.is_none();

        if can_restart {
            if let Some(dir) = pending_moves.pop_front() {
                game.move_to(dir);
            }
        }

        game.step();

        let state = game.state();

        if prev_state != GameState::Started && state == GameState::Started {
            game_start = Some(Instant::now());
            ended_at = None;
            post_game = None;
        }

        if prev_state == GameState::Started && state == GameState::Ended {
            ended_at = Some(Instant::now());
            post_game = Some(PostGamePhase::Cooldown);
        }

        if let Some(PostGamePhase::Cooldown) = post_game {
            if ended_at.is_some_and(|t| t.elapsed() >= end_screen_lock) {
                post_game = Some(if qualifies(game.points()) {
                    PostGamePhase::NameEntry(String::new())
                } else {
                    PostGamePhase::Scoreboard(load_scores())
                });
            }
        }

        match state {
            GameState::Starting => draw_main_menu(&mut win)?,
            GameState::Started => draw_game(&mut win, &game, &snake_style, apple_color, frame)?,
            GameState::Ended => {
                renderer.borrow_mut().clear()?;

                match &post_game {
                    Some(PostGamePhase::NameEntry(name)) => {
                        draw_name_entry(&mut win, game.points(), name)?;
                    }
                    Some(PostGamePhase::Scoreboard(scores)) => {
                        draw_scoreboard(&mut win, scores)?;
                    }
                    Some(PostGamePhase::Cooldown) | None => {
                        let duration = match (game_start, ended_at) {
                            (Some(start), Some(end)) => end.duration_since(start),
                            _ => Duration::ZERO,
                        };
                        draw_end_menu(&mut win, game.points(), duration)?;
                    }
                }
            }
        }

        renderer.borrow_mut().present()?;
    }

    renderer.borrow_mut().dispose()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_named_color_as_solid() {
        let style = parse_style("green").unwrap();
        assert_eq!(style, SnakeStyle::Solid(ColorStruct::new(66, 168, 50)));
    }

    #[test]
    fn parses_orange_and_yellow() {
        assert_eq!(
            parse_style("orange").unwrap(),
            SnakeStyle::Solid(ColorStruct::new(220, 120, 30))
        );
        assert_eq!(
            parse_style("yellow").unwrap(),
            SnakeStyle::Solid(ColorStruct::new(200, 180, 40))
        );
    }

    #[test]
    fn parsing_is_case_insensitive_and_trims_whitespace() {
        let style = parse_style("  ReD  ").unwrap();
        assert_eq!(style, SnakeStyle::Solid(ColorStruct::new(200, 60, 60)));
    }

    #[test]
    fn parses_rainbow_keyword() {
        assert_eq!(parse_style("rainbow").unwrap(), SnakeStyle::Rainbow);
        assert_eq!(parse_style("Rainbow").unwrap(), SnakeStyle::Rainbow);
    }

    #[test]
    fn parses_party_keyword() {
        assert_eq!(parse_style("party").unwrap(), SnakeStyle::Party);
    }

    #[test]
    fn parses_two_stop_gradient() {
        let style = parse_style("red-magenta").unwrap();
        assert_eq!(
            style,
            SnakeStyle::Gradient(vec![
                ColorStruct::new(200, 60, 60),
                ColorStruct::new(170, 60, 170),
            ])
        );
    }

    #[test]
    fn parses_multi_stop_gradient_preserving_order() {
        let style = parse_style("red-magenta-blue").unwrap();
        assert_eq!(
            style,
            SnakeStyle::Gradient(vec![
                ColorStruct::new(200, 60, 60),
                ColorStruct::new(170, 60, 170),
                ColorStruct::new(60, 110, 200),
            ])
        );
    }

    #[test]
    fn rejects_unknown_color_name() {
        assert!(parse_style("mauve").is_err());
    }

    #[test]
    fn rejects_unknown_color_within_gradient() {
        let err = parse_style("red-mauve-blue").unwrap_err();
        assert!(err.contains("mauve"));
    }

    #[test]
    fn rejects_empty_color() {
        assert!(parse_style("").is_err());
        assert!(parse_style("   ").is_err());
    }

    #[test]
    fn color_arg_stores_lowercased_raw_value() {
        let arg = parse_color_arg("Red-Magenta").unwrap();
        assert_eq!(arg.raw, "red-magenta");
        assert_eq!(
            arg.style,
            SnakeStyle::Gradient(vec![
                ColorStruct::new(200, 60, 60),
                ColorStruct::new(170, 60, 170),
            ])
        );
    }

    #[test]
    fn apple_is_white_for_gradients_red_rainbow_and_party() {
        assert_eq!(apple_color_for("red-magenta"), Color::White);
        assert_eq!(apple_color_for("red"), Color::White);
        assert_eq!(apple_color_for("rainbow"), Color::White);
        assert_eq!(apple_color_for("party"), Color::White);
    }

    #[test]
    fn apple_is_red_for_other_solid_colors() {
        assert_eq!(apple_color_for("green"), Color::Red);
        assert_eq!(apple_color_for("blue"), Color::Red);
    }

    #[test]
    fn gradient_final_stop_shows_when_snake_fills_the_board() {
        let stops = vec![
            ColorStruct::new(200, 60, 60),
            ColorStruct::new(170, 60, 170),
            ColorStruct::new(60, 110, 200),
        ];
        let style = SnakeStyle::Gradient(stops.clone());
        let total_cells = (FIELD_LINES * FIELD_COLS) as u16;

        let color = snake_color(total_cells, &style, 0);
        assert_eq!(color, stops.last().unwrap().to_crossterm());
    }

    #[test]
    fn gradient_starts_at_first_stop() {
        let stops = vec![
            ColorStruct::new(200, 60, 60),
            ColorStruct::new(170, 60, 170),
            ColorStruct::new(60, 110, 200),
        ];
        let style = SnakeStyle::Gradient(stops.clone());

        let color = snake_color(0, &style, 0);
        assert_eq!(color, stops[0].to_crossterm());
    }

    #[test]
    fn gradient_loops_three_times_across_the_board() {
        let stops = vec![
            ColorStruct::new(200, 60, 60),
            ColorStruct::new(170, 60, 170),
            ColorStruct::new(60, 110, 200),
        ];
        let style = SnakeStyle::Gradient(stops.clone());
        let total_cells = (FIELD_LINES * FIELD_COLS) as u16;

        // Halfway through the board is 1.5 gradient loops in, landing back
        // exactly on the middle stop instead of merely halfway to the end.
        let halfway = total_cells / 2;
        let color = snake_color(halfway, &style, 0);
        assert_eq!(color, stops[1].to_crossterm());
    }
}
