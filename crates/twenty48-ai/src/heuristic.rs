use twenty48::Board;

/// Position evaluation, as a table over the 65536 possible rows.
///
/// Every term is a property of a single line, so a board is scored by eight
/// lookups: its four rows and, after a transpose, its four columns. The
/// weights are the ones from nneonneo's 2048-ai, which are well tuned and not
/// worth rediscovering by hand.
pub struct Heuristic {
    line: Vec<f32>,
}

impl Heuristic {
    pub fn new() -> Heuristic {
        let mut line = vec![0.0f32; 65536];
        for (index, value) in line.iter_mut().enumerate() {
            let ranks = [
                (index >> 12) as u32 & 0xF,
                (index >> 8) as u32 & 0xF,
                (index >> 4) as u32 & 0xF,
                index as u32 & 0xF,
            ];
            *value = score_line(ranks);
        }
        Heuristic { line }
    }

    pub fn eval(&self, board: Board) -> f32 {
        let rows = board.raw();
        let cols = board.transpose().raw();
        let mut total = 0.0;
        for i in 0..4 {
            let shift = 48 - 16 * i;
            total += self.line[(rows >> shift) as u16 as usize];
            total += self.line[(cols >> shift) as u16 as usize];
        }
        total
    }
}

fn score_line(ranks: [u32; 4]) -> f32 {
    /// Paid by a line in every position, so a lost game (which scores zero)
    /// sits far below any reachable evaluation.
    const LOST: f32 = 200_000.0;
    const EMPTY: f32 = 270.0;
    const MERGES: f32 = 700.0;
    const MONOTONICITY: f32 = 47.0;
    const MONOTONICITY_POWER: f32 = 4.0;
    const SUM: f32 = 11.0;
    const SUM_POWER: f32 = 3.5;

    let mut sum = 0.0;
    let mut empty = 0u8;
    let mut merges = 0u8;

    let mut previous = 0;
    let mut run = 0u8;
    for rank in ranks {
        sum += (rank as f32).powf(SUM_POWER);
        if rank == 0 {
            empty += 1;
        } else {
            if previous == rank {
                run += 1;
            } else if run > 0 {
                merges += 1 + run;
                run = 0;
            }
            previous = rank;
        }
    }
    if run > 0 {
        merges += 1 + run;
    }

    // Charge the line for however far it is from sorted, in whichever
    // direction is closer, so a descending line is as welcome as an ascending
    // one and only the zigzags are penalised.
    let (mut left, mut right) = (0.0, 0.0);
    for pair in ranks.windows(2) {
        let (a, b) = (
            (pair[0] as f32).powf(MONOTONICITY_POWER),
            (pair[1] as f32).powf(MONOTONICITY_POWER),
        );
        if pair[0] > pair[1] {
            left += a - b;
        } else {
            right += b - a;
        }
    }

    LOST + EMPTY * f32::from(empty) + MERGES * f32::from(merges)
        - MONOTONICITY * left.min(right)
        - SUM * sum
}
