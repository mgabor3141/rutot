# rutot

A prototype testing one idea: **the yard is a machine.**

Cars go in as ingredients, a consist comes out as the product, and a
deterministic search is the crafting animation. The player designs the yard
topology (sidings, capacities, headshunt length); the game finds the shunting
moves. Nobody programs a loco driver.

The claim being tested is that watching a solver shunt your yard feels like
*your* factory working, the way watching Factorio inserters does — rather
than like watching the game play itself.

## What's here

- `core/` — dependency-free model and solver.
  - `yard.rs`: a *main* with stub sidings off either end, a loco that stands
    on one side of its string, and three moves: `Pull`, `Push`, `RunAround`.
    A siding on the right is worked from the left of the string and vice
    versa; the run-round loop is the only way to change sides. Goal = a
    consist as the ladder-end prefix of one siding.
  - `solver.rs`: A\* over a packed `Copy` state with an admissible heuristic
    (pulls per siding, pushes per lead-length, run-rounds per side needed).
    Optimal plans. Cost is in *legs*: pull/push = 2, run-round = 3.
  - `sim.rs`: fixed-tick execution of a plan (trapezoidal speed profile,
    coupling dwell; run-rounds animate as loop / back on / draw up).
  - `layout.rs`: polyline geometry for a two-ended ladder yard with a loop.
  - `bin/yardbench.rs`: compare topologies by mean optimal plan cost.
- `app/` — Bevy 0.19 visualiser. 30 Hz `FixedUpdate` simulation, rendering
  interpolated with `Time<Fixed>::overstep_fraction`. Planning runs on a
  background thread.

## Run

```sh
cargo run -p rutot --features dev            # visualiser (dynamic linking for fast rebuilds)
cargo run -p rutot-core --release --bin yardbench 200
cargo test -p rutot-core --release

RUTOT_YARD=4 RUTOT_SEED=7 cargo run -p rutot --features dev            # pick yard / seed
RUTOT_SHOT_AFTER=6 cargo run -p rutot --features dev                   # screenshot + exit
RUTOT_SHOT_PHASE="round the loop" RUTOT_SHOT_PHASE_TICKS=30 cargo run -p rutot --features dev
```

Controls: `space` pause · `1/2/3` speed · `R` new task · `Y` next yard ·
`A` auto-advance · `P` screenshot.

## First numbers

200 random tasks per yard (8 cars, build a specific 5-car consist on siding 0).
Legs: pull/push = 2, run-round = 3.

| yard                                  | legs | moves | run-rounds | unsolved | note |
|---------------------------------------|-----:|------:|-----------:|---------:|------|
| Inglenook 5-3-3, lead 3 (classic)     | 22.6 | 11.3  | 0.00 |   0 | baseline |
| Inglenook 5-3-3, lead 3 **+ loop**    | 22.6 | 11.3  | 0.00 |   0 | loop never used: one-sided yard |
| Inglenook 5-3-3, lead 4               | 18.3 |  9.1  | 0.00 |   0 | +1 car of lead ≈ −19% |
| Inglenook 5-3-3-2, lead 3             | 18.3 |  9.2  | 0.00 |   0 | a 2-car spur ≈ same gain |
| Inglenook 5-3-3-3-3, lead 3           | 16.9 |  8.5  | 0.00 |   0 | |
| Inglenook 5-3-3, **lead 2**           |    — |    —  |    — | 178 | proven unsolvable, not budget |
| **Split** 5-3 \| 3-2, lead 3 + loop   | 32.5 | 14.8  | 2.88 |   0 | same capacities as 5-3-3-2, sidings on both ends: **+78%** |
| Timesaver 5-2 \| 3-2, main 3 + loop   | 39.8 | 17.8  | 4.28 |   1 | the famously awkward one |
| Timesaver, **no loop**                |    — |    —  |    — | 200 | far-side sidings are dead storage |

Three things fall out that a player would have to discover:

1. Topology consequences are non-linear. One car less of lead doesn't slow
   the yard, it breaks it.
2. A run-round loop is worthless unless sidings face both ways — and sidings
   facing both ways are what make a yard slow. Keep your ladder on one side.
   (This is real yard-design wisdom; the solver rediscovered it.)
3. Without the loop, far-side sidings aren't slow, they're unreachable.

## Open questions this prototype exists to answer

1. Does watching it feel like ownership or like autoplay?
2. Is mean plan length a legible enough "recipe time" to design against?
3. Where does this sit in a factory — what forces mixed consists to exist?
   (Working answer: bounded buffers make time matter, small flows share
   trains, shared trains need re-sorting.)

## Not in scope yet

Through (double-ended) sidings, multiple shunters, cost by distance rather
than legs, loop capacity limits, any production chain.
