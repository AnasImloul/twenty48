# twenty48

A 2048 engine and an expectimax agent that plays it to 32768, the largest tile
the board can hold, in a few milliseconds per move. Ships as a desktop app you
can watch it play in, and as two libraries if you want to drive the rules or
the search yourself.

![The desktop app, showing a finished game scoring 623992 with a 32768 tile](https://raw.githubusercontent.com/AnasImloul/twenty48/main/docs/screenshot.png)

That game is the agent at depth 8, which it played to 32768 and on to a score
of 623992 over 21977 moves, at 3.1 ms and 90476 nodes per move. Reaching the
largest tile is routine rather than lucky: over twenty-four seeded games at the
default settings the median score is 342976, and every game ends on 8192 or
better. The [full distribution](#an-agent) is below.

## Get it

Every push to `main` publishes binaries for macOS, Linux and Windows to the
[`latest`](https://github.com/AnasImloul/twenty48/releases/tag/latest) release.
Download the archive for your platform, unpack it, and run `twenty48-gui`. They
are unsigned, so the first launch needs a nudge: on macOS
`xattr -d com.apple.quarantine twenty48-gui`, on Windows "More info" then
"Run anyway".

To build it yourself, with a Rust toolchain installed:

```sh
cargo run --release -p twenty48-gui
```

Use `--release`. A debug build searches the same tree with none of the
optimisations and takes seconds per move rather than milliseconds.

## Using the app

Arrow keys or WASD play by hand. Everything else is the panel on the right.

**Game.** The text box holds the seed, which fixes the entire sequence of
spawned tiles: `new` starts that seed, `random` picks a fresh one and shows it,
and the same seed always replays the same game. `undo` steps back a move, as
far back as you like.

**Agent.** `play` hands it the game and it plays until the board is dead or you
pause it; `step` gives it one move; `hint` shows the move it would make and
leaves the playing to you. The speed slider throttles it down to one move a
second if you want to follow along, or up to `max` to watch a game finish in a
few seconds.

**Search.** `depth` bounds how far ahead it looks and `floor` is the
probability below which a branch is evaluated rather than expanded. They bind
at opposite ends of the game, which is why both are exposed: depth bounds the
late game, where the board is crowded, and the floor bounds the early one,
where a chance node fans out thirty ways and depth alone bounds nothing.
Raising either makes the agent stronger and slower, and both take effect on the
agent's next move, so you can turn depth up mid-game and watch the cost in the
throughput panel.

**Throughput** reports what the last move actually cost: wall time, nodes
visited, and the node rate across all the threads the search ran on.

## Using the engine

The engine is split in two. `Board` is pure: it applies moves and knows nothing
about randomness or score history, which is what a search wants. `Game` wraps
it with a running score and a seeded tile source, which is what a frontend
wants. The whole board is one `u64` and sliding a row is a table lookup, which
puts a move at a handful of instructions and nothing on the heap.

```rust
use twenty48::{Direction, Game, MoveError};

let mut game = Game::new(0xC0FFEE);

match game.play(Direction::Left) {
    Ok(round) => println!("scored {}, tile at {:?}", round.gained, round.spawn),
    Err(MoveError::Illegal(dir)) => println!("nothing moves {dir}"),
    Err(MoveError::GameOver) => println!("final score {}", game.score()),
}
```

Replaying the same moves against the same seed reproduces the game exactly,
which is what makes "policy A beats policy B over ten thousand games" a claim
you can re-run.

## Driving it from a search

The deterministic slide and the random spawn are separate operations, so a
search can enumerate every tile the game might place instead of sampling one:

```rust
use twenty48::{Board, Direction};

let board = Board::from_tiles([
    [2, 2, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
]).unwrap();

let (next, gained) = board.shift_scored(Direction::Left);
assert_eq!(gained, 4);

for offset in next.empty_cells() {
    for exponent in [1, 2] {
        let _child = next.spawn_at(offset, exponent);
    }
}
```

Use `Board::shift_all` rather than four calls to `Board::shift` when expanding a
node; it shares the transpose between the vertical moves and leaves sixteen
table loads with no dependencies between them. `Board::transpose` is public for
a related reason: an evaluation function that scores one line at a time wants
the columns as rows, and that is otherwise the one piece of the engine an agent
would have to reimplement.

## How it works

Every tile is a power of two, so a cell only needs to store `log2(tile)`: four
bits, with zero meaning empty. Sixteen cells is 64 bits, so a `Board` is one
`u64`, is `Copy`, lives in a register and lets a search keep snapshots on the
stack instead of maintaining an undo log. Cell `(row, col)` sits at bit
`60 - 16*row - 4*col`, most significant nibble first, so a board written as a
hex literal reads in the same order as the grid.

Once a board is a `u64` a row is a `u16`, of which there are only 65536, so the
result of sliding one is a lookup rather than a loop. The tables hold the slid
row and the points its merges scored, for each of the two horizontal
directions; vertical moves transpose first, which is fourteen ALU ops and no
memory traffic. They are built by const evaluation, which keeps their base
address a link-time constant: no lazy-init branch and no atomic load on the hot
path.

The tables are 768 KiB, which is the usual reason to be suspicious of this
design. The objection does not survive measurement: reachable rows are
dominated by small exponents and mostly-monotone runs, and a couple of hundred
thousand lookups over 256 games touch 3390 of the 65536 rows, spanning 20 KiB
of each table. Sliding real boards runs about 20% faster than sliding boards
made of random bits, and the `cargo bench` output reports both so the gap stays
visible.

Game over is answered without materialising any moves. A board with an empty
cell is never over; a full one is over only when no two adjacent tiles can
merge, which an XOR against a shifted copy answers in bit operations alone,
with no table lookups and no moves applied.

The crate is `#![forbid(unsafe_code)]`. Table indices are written
`(bits >> 48) as u16 as usize` rather than with an explicit mask, which hands
LLVM the range as a type fact and folds the bounds check away; CI greps the
release assembly for `panic_bounds_check` to keep it that way.

## Measured throughput

```
shift, played boards         724.6 M shifts/s
shift, random boards         591.1 M shifts/s
spawn, draw and place         74.3 M spawns/s
round, random policy          22.1 M rounds/s
round, down-first policy      40.1 M rounds/s
node, expectimax depth 3     293.4 M nodes/s
```

Apple M2 Max, rustc 1.94.1, `lto = "fat"` and `codegen-units = 1`. Reproduce
with `cargo bench`, which uses a hand-rolled harness and no dev-dependencies.

The three rates are reported separately because they differ by an order of
magnitude. A *shift* is one slide and nothing else. A *round* is what a game
does per move: slide, legality, the spawn including its random draw, and the
game-over check. A *node* is a position visited by a depth-3 expectimax whose
position evaluation is one instruction, so it measures the engine under a
search rather than the search itself.

## An agent

`crates/twenty48-ai/examples/expectimax.rs` is a real player rather than a
demonstration, and it exists mostly to check that the API is adequate for the
consumer it was designed around. Twenty-four seeded games at the default
settings:

```
70127 nodes and 3.33 ms per move, 298049 moves over all games
21.1 M nodes/s per core, 221.5 M across the machine

score: mean 325501, median 342976, best 630036

largest tile reached:
  32768     2    8%
  16384    16   66%
   8192     6   25%
```

It reaches 32768, which is the largest tile four bits can hold. That is worth
more than a test: such a game exercises the rule that two 32768s do not merge,
on a board the engine played into rather than one written by hand.

The position evaluation is a table over all 65536 lines, so scoring a board is
eight lookups, four rows and four columns after a `transpose`. Two settings
bound the work, and they bind at opposite ends of the game. `--depth` bounds
the late game, where the board is crowded and a chance node has few children.
`--floor` bounds the early game, where a chance node fans out thirty ways and
depth alone bounds nothing: a branch whose probability of being reached falls
below it is evaluated rather than expanded.

Both matter, so both are tunable. Time per move here is measured on one core,
because that is the latency a single game sees:

| `--depth` | `--floor` | ms/move | nodes/move | mean score | best tile |
|---|---|---|---|---|---|
| 3 | 1e-3 | 0.15 | 5 150 | | 8192 |
| 4 | 1e-3 | 0.91 | 28 503 | 186 044 | 16384 |
| 5 | 1e-3 | 2.43 | 70 344 | 325 501 | 32768 |
| 5 | 3e-4 | 5.01 | 153 328 | | |
| 5 | 1e-4 | 10.60 | 260 124 | 278 535 | 16384 |

Depth 5 at a `1e-3` floor is the default. Spending four times as long per move
on the `1e-4` floor below it buys nothing: the two rows are within the noise of
each other, and 2048 scores are noisy enough that the twelve- and twenty-four
game samples here separate 2x differences and not much finer.

There are two ways to spend a machine here and they answer different
questions. Sharding games across cores is what an evaluation run wants, since
the result is a score distribution and games never synchronise: on an 8+4 core
M2 Max twelve games at once reach 222 M nodes/s against 37 M on one core,
though the per-move latency above degrades to 2.58 ms while all twelve are
busy.

`Search::with_threads` instead splits a single move, which is what a UI wants,
because there is only one game on screen to spend the machine on. The split is
two plies down rather than on the four root moves: a piece of work is a root
move, a tile the game could place in reply, and one answer to that tile, which
gives a mid-game position a hundred or so pieces instead of four. Twelve
threads take the default depth to a fifth of the single-threaded latency per
move.

Getting that granularity right was most of the work. Stopping a ply short, one
piece per tile the game could place, looks like plenty of parallelism at fifty
pieces, but the largest of them measured around 12% of a move's work, which is
more than a twelfth: eleven threads finished and waited on one. Utilisation sat
at 62% of twelve cores and no scheduling of that list could have passed 70%.
Splitting the reply as well triples the count and measures 75% of twelve cores
and a fifth off the latency. Running the large pieces first balances the batch
better still and is not worth it: neighbouring pieces share most of their
subtree, so visiting them apart empties the transposition table between them
and loses more node rate than the balance gains.

Shared, not one per thread. The transposition table is not only a cache: an
entry satisfies any request for its board at that depth *or shallower*, so a
hit often returns a better answer than the caller would have computed. Giving
each thread its own measures three times the nodes and one fifth of the score.
The price of sharing it is reproducibility, because which thread reaches a
position first decides what later lookups of it return, so seeded games only
replay exactly on `Search::new`. It costs a little strength too: over
twenty-four seeds the split search means 275 432 against 336 501 serial, and
reaches 16384 in eleven games against sixteen. Both find 32768 twice. That is
a real if modest loss and not merely a noisy sample, which is the trade the
split makes for answering five times sooner.

## Testing

`crates/twenty48/tests/reference/` holds a naive `[[u8; 4]; 4]` implementation
that slides by filtering into a `Vec` and walking pairs. It shares no structure
with the packed engine, so the two are wrong in the same way only by
coincidence, and everything else is checked against it:

- Every one of the 65536 rows, slid in all four directions, row and score.
- Seeded playouts driven through both in lockstep, compared after every move.
- `is_game_over` against the naive "no move changes the board", over a corpus
  of positions taken from real games rather than random bits.
- The tile total is conserved by every shift, and a move's score is the value
  of the tiles it created, derived from exponent histograms rather than from
  the engine's own accounting.

`cargo run --release --example random_agent` plays ten thousand seeded games
and reports the distribution of final tiles, which for random play should end
on a 64 or a 128 most of the time and reach 1024 never.

## Examples

```sh
cargo run --release -p twenty48-gui          # the desktop app
cargo run --release --example play           # wasd in the terminal
cargo run --release --example play -- 12345  # from a seed, to replay a game
cargo run --release --example random_agent   # playout statistics
cargo run --release --example expectimax     # an agent that reaches 32768
cargo run --release --example expectimax -- --games 4 --depth 4 --floor 1e-3
```

## License

Dual licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](https://github.com/AnasImloul/twenty48/blob/main/LICENSE-APACHE))
- MIT License
  ([LICENSE-MIT](https://github.com/AnasImloul/twenty48/blob/main/LICENSE-MIT))

at your option.
