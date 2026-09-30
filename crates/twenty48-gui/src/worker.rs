//! The search, on a thread of its own.
//!
//! A move at the default depth is about half a millisecond, which already
//! eats a frame if it runs in `logic`, and the depth control goes high enough
//! to make it seconds. The search splits itself over the machine from here,
//! so this thread is really a foreman.
//!
//! It does not just answer one position at a time. Autoplay runs its whole
//! loop here and reports each move as it happens, because a UI that asked for
//! one move per frame would cap the agent at the refresh rate: half a
//! millisecond a move is nearly two thousand a second, and vsync would throw
//! away all but a hundred or so of them.

use std::ops::ControlFlow;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;
use std::time::{Duration, Instant};

use twenty48::{Board, Direction, Game, Spawn, Status};
use twenty48_ai::{Config, Search};

/// What a one-off search should be used for. The thread does not act on it;
/// it comes back with the answer so the UI can.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Play the move.
    Step,
    /// Show it as an arrow and leave the board alone.
    Hint,
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub config: Config,
    /// Least time between moves. Zero lets the search set the pace.
    pub interval: Duration,
}

enum Command {
    Once {
        board: Board,
        config: Config,
        kind: Kind,
        generation: u64,
    },
    Run {
        game: Box<Game>,
        settings: Settings,
        generation: u64,
    },
    /// Change settings without interrupting a run.
    Tune(Settings),
    Stop,
}

/// Something the search finished doing.
pub enum Event {
    /// A move was played during a run. The game is the state after it.
    Moved {
        game: Box<Game>,
        spawn: Spawn,
        nodes: u64,
        elapsed: Duration,
        generation: u64,
    },
    /// The answer to a one-off search.
    Suggested {
        dir: Option<Direction>,
        kind: Kind,
        nodes: u64,
        elapsed: Duration,
        generation: u64,
    },
    /// A run stopped because the game ended.
    Finished { generation: u64 },
}

pub struct Worker {
    commands: Sender<Command>,
    events: Receiver<Event>,
}

impl Worker {
    /// Spawns the search thread. It wakes the UI through `repaint` when there
    /// is something new, so the app can sit idle rather than polling.
    pub fn new(repaint: impl Fn() + Send + 'static) -> Worker {
        let (commands, inbox) = channel::<Command>();
        let (outbox, events) = channel::<Event>();

        thread::Builder::new()
            .name("search".to_owned())
            .spawn(move || run(&inbox, &outbox, &repaint))
            .expect("the search thread could not start");

        Worker { commands, events }
    }

    pub fn once(&self, board: Board, config: Config, kind: Kind, generation: u64) {
        let _ = self.commands.send(Command::Once {
            board,
            config,
            kind,
            generation,
        });
    }

    pub fn start(&self, game: Game, settings: Settings, generation: u64) {
        let _ = self.commands.send(Command::Run {
            game: Box::new(game),
            settings,
            generation,
        });
    }

    pub fn tune(&self, settings: Settings) {
        let _ = self.commands.send(Command::Tune(settings));
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }

    pub fn drain(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }
}

fn run(inbox: &Receiver<Command>, outbox: &Sender<Event>, repaint: &(impl Fn() + ?Sized)) {
    // Several MiB of tables, built once and reused for the life of the
    // process. The whole machine goes to one move, since there is only ever
    // one game on screen to spend it on.
    let threads = thread::available_parallelism().map_or(1, |n| n.get());
    let mut search = Search::with_threads(Config::default(), threads);

    while let Ok(command) = inbox.recv() {
        let flow = match command {
            Command::Once {
                board,
                config,
                kind,
                generation,
            } => {
                search.set_config(config);
                let (dir, nodes, elapsed) = think(&mut search, board);
                let event = Event::Suggested {
                    dir,
                    kind,
                    nodes,
                    elapsed,
                    generation,
                };
                send(outbox, event, repaint)
            }
            Command::Run {
                game,
                settings,
                generation,
            } => autoplay(
                &mut search,
                *game,
                settings,
                generation,
                inbox,
                outbox,
                repaint,
            ),
            Command::Tune(_) | Command::Stop => ControlFlow::Continue(()),
        };

        if flow.is_break() {
            return;
        }
    }
}

fn think(search: &mut Search, board: Board) -> (Option<Direction>, u64, Duration) {
    let before = search.nodes();
    let start = Instant::now();
    let dir = search.best_move(board);
    (dir, search.nodes() - before, start.elapsed())
}

fn send(outbox: &Sender<Event>, event: Event, repaint: &(impl Fn() + ?Sized)) -> ControlFlow<()> {
    if outbox.send(event).is_err() {
        return ControlFlow::Break(());
    }
    repaint();
    ControlFlow::Continue(())
}

/// Plays until the game ends, the UI says stop, or the channel closes.
///
/// Settings are re-read between moves, so dragging the depth slider during a
/// run takes effect on the next move rather than being ignored until the run
/// restarts.
fn autoplay(
    search: &mut Search,
    mut game: Game,
    mut settings: Settings,
    generation: u64,
    inbox: &Receiver<Command>,
    outbox: &Sender<Event>,
    repaint: &(impl Fn() + ?Sized),
) -> ControlFlow<()> {
    loop {
        match inbox.try_recv() {
            Ok(Command::Tune(next)) => settings = next,
            // Stop, or anything that supersedes this run. The UI bumps the
            // generation when it sends one, so nothing already in flight gets
            // applied to the wrong game.
            Ok(_) => return ControlFlow::Continue(()),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return ControlFlow::Break(()),
        }

        if game.status() == Status::Over {
            return send(outbox, Event::Finished { generation }, repaint);
        }

        let started = Instant::now();
        search.set_config(settings.config);
        let (dir, nodes, elapsed) = think(search, game.board());

        let Some(dir) = dir else {
            return send(outbox, Event::Finished { generation }, repaint);
        };
        let round = game.play(dir).expect("the search only returns legal moves");

        let event = Event::Moved {
            game: Box::new(game.clone()),
            spawn: round.spawn,
            nodes,
            elapsed,
            generation,
        };
        send(outbox, event, repaint)?;

        if let Some(left) = settings.interval.checked_sub(started.elapsed()) {
            thread::sleep(left);
        }
    }
}
