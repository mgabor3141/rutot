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
  - `yard.rs`: sidings, headshunt, `Pull`/`Push` moves, goal = consist prefix.
  - `solver.rs`: A\* over a packed `Copy` state with an admissible heuristic.
    Optimal plans; ~5–30 ms per Inglenook task.
  - `sim.rs`: fixed-tick execution of a plan (trapezoidal speed profile,
    coupling dwell), engine-agnostic.
  - `layout.rs`: polyline geometry for a ladder yard.
  - `bin/yardbench.rs`: compare topologies by mean optimal plan length.
- `app/` — Bevy 0.19 visualiser. 30 Hz `FixedUpdate` simulation, rendering
  interpolated with `Time<Fixed>::overstep_fraction`.

## Run

```sh
cargo run -p rutot --features dev            # visualiser (dynamic linking for fast rebuilds)
cargo run -p rutot-core --release --bin yardbench 200
cargo test -p rutot-core --release
RUTOT_SHOT_AFTER=6 cargo run -p rutot --features dev   # screenshot to rutot-001.png and exit
```

Controls: `space` pause · `1/2/3` speed · `R` new task · `Y` next yard ·
`A` auto-advance · `P` screenshot.

## First numbers

200 random Inglenook tasks (8 cars, build a specific 5-car consist on siding 0):

| yard                      | mean moves | max | note                        |
|---------------------------|-----------:|----:|-----------------------------|
| 5-3-3, lead 3 (classic)   |      11.29 |  16 |                             |
| 5-3-3, lead 4             |       9.13 |  13 | +1 car of lead ≈ −19% moves |
| 5-3-3-2, lead 3           |       9.16 |  12 | a 2-car spur ≈ same gain    |
| 5-3-3, lead 2             |      11.82 |  16 | **178/200 unsolvable**      |
| 5-4-4, lead 3             |      10.46 |  14 |                             |
| 5-3-3-3-3, lead 3         |       8.46 |  11 |                             |

Topology has non-linear, non-obvious consequences (a one-car-shorter lead
doesn't slow the yard, it breaks it). That's the design space a player would
be exploring.

## Open questions this prototype exists to answer

1. Does watching it feel like ownership or like autoplay?
2. Is mean plan length a legible enough "recipe time" to design against?
3. Where does this sit in a factory — what forces mixed consists to exist?
   (Working answer: bounded buffers make time matter, small flows share
   trains, shared trains need re-sorting.)

## Not in scope yet

Run-around loops, through tracks, multiple shunters, cost by distance rather
than move count, any production chain.
