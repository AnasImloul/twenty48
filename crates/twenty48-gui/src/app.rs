//! Application state and the frame loop.

use std::time::{Duration, Instant};

use eframe::egui::{Button, CentralPanel, Context, Id, Key, Panel, RichText, Slider, TextEdit, Ui};

use twenty48::{Direction, Game, Spawn, Status};
use twenty48_ai::Config;

use crate::render::{self, Highlight};
use crate::theme;
use crate::worker::{Event, Kind, Settings, Worker};

/// How long a newly spawned tile takes to grow to full size.
const SPAWN_ANIMATION: Duration = Duration::from_millis(90);

/// Top of the speed slider, in moves per second. Past this the delay is
/// removed entirely and the search sets the pace, which at shallow depths is
/// well over a thousand moves a second.
const MAX_SPEED: f32 = 500.0;

pub struct App {
    game: Game,
    seed: u64,
    seed_input: String,
    /// Positions before each move. `Game` is `Clone` and about fifty bytes, so
    /// the whole history of a twenty thousand move game is around a megabyte,
    /// which is much simpler than asking the engine to keep an undo log it has
    /// no other use for.
    history: Vec<Game>,
    config: Config,
    autoplay: bool,
    speed: f32,
    worker: Worker,
    /// Bumped whenever the UI takes control back: new game, undo, pause, a key
    /// press. Events tagged with an older generation belong to a game state
    /// the player has already left, so they are dropped rather than applied.
    generation: u64,
    hint: Option<Direction>,
    spawn: Option<Spawn>,
    spawn_at: Instant,
    last: Option<Measured>,
    rate: Rate,
}

struct Measured {
    nodes: u64,
    elapsed: Duration,
}

/// Moves actually reaching the board per second, over a short window.
///
/// Worth showing because it is the number the frame rate used to cap, and
/// because it makes the cost of a depth setting visible as you drag it.
struct Rate {
    since: Instant,
    moves: u32,
    per_second: f32,
}

impl Rate {
    const WINDOW: Duration = Duration::from_millis(500);

    fn new() -> Rate {
        Rate {
            since: Instant::now(),
            moves: 0,
            per_second: 0.0,
        }
    }

    fn tick(&mut self) {
        self.moves += 1;
    }

    fn refresh(&mut self) {
        let elapsed = self.since.elapsed();
        if elapsed >= Self::WINDOW {
            self.per_second = self.moves as f32 / elapsed.as_secs_f32();
            self.since = Instant::now();
            self.moves = 0;
        }
    }
}

impl App {
    pub fn new(ctx: &Context) -> App {
        theme::apply(ctx);
        let ctx = ctx.clone();
        let worker = Worker::new(move || ctx.request_repaint());

        App {
            game: Game::new(0),
            seed: 0,
            seed_input: "0".to_owned(),
            history: Vec::new(),
            config: Config::default(),
            autoplay: false,
            speed: MAX_SPEED,
            worker,
            generation: 0,
            hint: None,
            spawn: None,
            spawn_at: Instant::now(),
            last: None,
            rate: Rate::new(),
        }
    }

    fn settings(&self) -> Settings {
        let interval = if self.speed >= MAX_SPEED {
            Duration::ZERO
        } else {
            Duration::from_secs_f32(1.0 / self.speed)
        };
        Settings {
            config: self.config,
            interval,
        }
    }

    /// Hands control to the agent or takes it back.
    ///
    /// Autoplay runs entirely on the search thread, so this is a handover of
    /// the game rather than the start of a request loop: the UI sends its
    /// current position and then only receives.
    fn set_autoplay(&mut self, on: bool) {
        if self.autoplay == on {
            return;
        }
        self.autoplay = on;
        self.generation += 1;
        if on {
            self.worker
                .start(self.game.clone(), self.settings(), self.generation);
        } else {
            self.worker.stop();
        }
    }

    fn restart(&mut self, seed: u64) {
        self.set_autoplay(false);
        self.seed = seed;
        self.game = Game::new(seed);
        self.history.clear();
        self.generation += 1;
        self.hint = None;
        self.spawn = None;
        self.last = None;
    }

    fn undo(&mut self) {
        self.set_autoplay(false);
        if let Some(previous) = self.history.pop() {
            self.game = previous;
            self.generation += 1;
            self.hint = None;
            self.spawn = None;
        }
    }

    fn play(&mut self, dir: Direction) {
        let before = self.game.clone();
        let Ok(round) = self.game.play(dir) else {
            return;
        };
        self.history.push(before);
        self.hint = None;
        self.spawn = Some(round.spawn);
        self.spawn_at = Instant::now();
    }

    fn apply_events(&mut self) {
        let events: Vec<Event> = self.worker.drain().collect();
        for event in events {
            match event {
                Event::Moved {
                    game,
                    spawn,
                    nodes,
                    elapsed,
                    generation,
                } => {
                    if generation != self.generation {
                        continue;
                    }
                    self.history.push(std::mem::replace(&mut self.game, *game));
                    self.spawn = Some(spawn);
                    self.spawn_at = Instant::now();
                    self.hint = None;
                    self.last = Some(Measured { nodes, elapsed });
                    self.rate.tick();
                }
                Event::Suggested {
                    dir,
                    kind,
                    nodes,
                    elapsed,
                    generation,
                } => {
                    if generation != self.generation {
                        continue;
                    }
                    self.last = Some(Measured { nodes, elapsed });
                    match (kind, dir) {
                        (Kind::Step, Some(dir)) => {
                            self.play(dir);
                            self.rate.tick();
                        }
                        (Kind::Hint, dir) => self.hint = dir,
                        (Kind::Step, None) => {}
                    }
                }
                Event::Finished { generation } => {
                    if generation == self.generation {
                        self.autoplay = false;
                    }
                }
            }
        }
    }

    fn handle_keys(&mut self, ctx: &Context) {
        if self.game.status() == Status::Over {
            return;
        }
        let pressed = ctx.input(|input| {
            [
                (Direction::Left, [Key::ArrowLeft, Key::A]),
                (Direction::Right, [Key::ArrowRight, Key::D]),
                (Direction::Up, [Key::ArrowUp, Key::W]),
                (Direction::Down, [Key::ArrowDown, Key::S]),
            ]
            .into_iter()
            .find(|(_, keys)| keys.iter().any(|&key| input.key_pressed(key)))
            .map(|(dir, _)| dir)
        });

        if let Some(dir) = pressed {
            // A key press is the player taking over.
            self.set_autoplay(false);
            self.play(dir);
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.apply_events();
        self.handle_keys(ctx);
        self.rate.refresh();

        // The search thread asks for a repaint on every move, so an idle app
        // costs nothing. These two are the cases with no event behind them.
        if self.autoplay {
            ctx.request_repaint_after(Rate::WINDOW);
        }
        if self.spawn.is_some() && self.spawn_at.elapsed() < SPAWN_ANIMATION {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        Panel::top(Id::new("header")).show(ui, |ui| self.header(ui));
        Panel::right(Id::new("controls"))
            .default_size(250.0)
            .size_range(230.0..=340.0)
            .show(ui, |ui| self.controls(ui));
        CentralPanel::default().show(ui, |ui| {
            // Held back at both ends so the board stays centred and the
            // game-over line has somewhere to go without moving it.
            const RESERVE: f32 = 30.0;
            let room = ui.available_height();
            let size = (room - 2.0 * RESERVE)
                .min(ui.available_width())
                .clamp(200.0, 620.0);
            ui.add_space(((room - size) / 2.0).max(0.0));
            self.board(ui, size);
        });
    }
}

impl App {
    fn header(&mut self, ui: &mut Ui) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("2048").size(30.0).strong().color(theme::INK));
            // Pushed right with a spacer rather than a right-to-left layout,
            // which cross-aligns each chip against a taller region than the
            // last and leaves them stepped down the row.
            let chips = 3.0 * theme::CHIP_WIDTH + 2.0 * ui.spacing().item_spacing.x;
            ui.add_space((ui.available_width() - chips).max(0.0));
            theme::chip(ui, "MOVES", &self.game.moves().to_string());
            theme::chip(ui, "BEST TILE", &self.game.board().max_tile().to_string());
            theme::chip(ui, "SCORE", &self.game.score().to_string());
        });
        ui.add_space(10.0);
    }

    fn board(&mut self, ui: &mut Ui, size: f32) {
        let age = self.spawn_at.elapsed().as_secs_f32() / SPAWN_ANIMATION.as_secs_f32();
        let highlight = Highlight {
            spawn: self.spawn,
            age,
            hint: self.hint,
        };

        ui.vertical_centered(|ui| {
            render::board(ui, size, self.game.board(), &highlight);
            if self.game.status() == Status::Over {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "game over after {} moves, best tile {}",
                        self.game.moves(),
                        self.game.board().max_tile()
                    ))
                    .strong()
                    .color(theme::DANGER),
                );
            }
        });
    }

    fn controls(&mut self, ui: &mut Ui) {
        let before = (self.config, self.speed);
        let over = self.game.status() == Status::Over;

        theme::section(ui, "GAME");
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.seed_input)
                    .desired_width(96.0)
                    .text_color(theme::INK)
                    .hint_text("seed"),
            );
            if ui.button("new").clicked() {
                let seed = self.seed_input.trim().parse().unwrap_or(self.seed);
                self.seed_input = seed.to_string();
                self.restart(seed);
            }
            if ui.button("random").clicked() {
                // Good enough to get a different game, and the seed is shown
                // so it can be typed back in to replay this one.
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64);
                self.seed_input = seed.to_string();
                self.restart(seed);
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.history.is_empty(), Button::new("undo"))
                .clicked()
            {
                self.undo();
            }
            theme::hint(ui, &format!("playing seed {}", self.seed));
        });

        theme::rule(ui);
        theme::section(ui, "AGENT");

        ui.horizontal(|ui| {
            let label = if self.autoplay { "pause" } else { "play" };
            if ui.add_enabled(!over, Button::new(label)).clicked() {
                self.set_autoplay(!self.autoplay);
            }
            if ui
                .add_enabled(!over && !self.autoplay, Button::new("step"))
                .clicked()
            {
                self.worker
                    .once(self.game.board(), self.config, Kind::Step, self.generation);
            }
            if ui
                .add_enabled(!over && !self.autoplay, Button::new("hint"))
                .clicked()
            {
                self.worker
                    .once(self.game.board(), self.config, Kind::Hint, self.generation);
            }
        });

        ui.add_space(6.0);
        ui.add(
            Slider::new(&mut self.speed, 1.0..=MAX_SPEED)
                .logarithmic(true)
                .text("speed")
                .custom_formatter(|value, _| {
                    if value >= f64::from(MAX_SPEED) {
                        "max".to_owned()
                    } else {
                        format!("{value:.0}/s")
                    }
                }),
        );

        theme::rule(ui);
        theme::section(ui, "SEARCH");

        ui.add(Slider::new(&mut self.config.depth, 1..=8).text("depth"));
        ui.add(
            Slider::new(&mut self.config.floor, 1e-5..=1e-1)
                .logarithmic(true)
                .text("floor")
                .custom_formatter(|value, _| format!("{value:.0e}")),
        );
        ui.add_space(2.0);
        theme::hint(ui, "depth bounds the late game, the floor the early one");

        theme::rule(ui);
        theme::section(ui, "THROUGHPUT");
        match &self.last {
            Some(last) => {
                theme::readout(
                    ui,
                    "per move",
                    &format!("{:.1} ms", last.elapsed.as_secs_f64() * 1e3),
                );
                theme::readout(ui, "nodes", &format!("{}", last.nodes));
                theme::readout(
                    ui,
                    "search",
                    &format!(
                        "{:.1} M nodes/s",
                        last.nodes as f64 / last.elapsed.as_secs_f64() / 1e6
                    ),
                );
            }
            None => theme::hint(ui, "nothing searched yet"),
        }
        theme::readout(
            ui,
            "on the board",
            &if self.autoplay {
                format!("{:.0} moves/s", self.rate.per_second)
            } else {
                "paused".to_owned()
            },
        );

        theme::rule(ui);
        theme::hint(ui, "arrow keys or WASD to play yourself");

        // Sliders take effect on the agent's next move rather than when it
        // next stops.
        if self.autoplay && (self.config, self.speed) != before {
            self.worker.tune(self.settings());
        }
    }
}
