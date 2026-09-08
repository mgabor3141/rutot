//! Compare yard topologies by mean optimal plan cost over random tasks.
//!
//!     cargo run -p rutot-core --release --bin yardbench [tasks] [seed]

use rutot_core::{benchmark, Side, Siding, Yard};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tasks: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1234);

    let mut yards = vec![
        Yard::inglenook(),
        Yard::inglenook_with_loop(),
        Yard::inglenook_long_lead(),
        Yard::inglenook_four(),
        Yard::timesaver(),
        Yard::timesaver_no_loop(),
    ];
    let mut y = Yard::inglenook();
    y.name = "Inglenook 5-3-3 / lead 2".into();
    y.headshunt = 2;
    yards.push(y);
    let mut y = Yard::inglenook();
    y.name = "Inglenook 5-4-4 / lead 3".into();
    y.sidings[1].capacity = 4;
    y.sidings[2].capacity = 4;
    yards.push(y);
    let mut y = Yard::inglenook();
    y.name = "Inglenook 5-3-3-3-3 / lead 3".into();
    y.sidings.push(Siding { name: "S3".into(), capacity: 3, side: Side::Right });
    y.sidings.push(Siding { name: "S4".into(), capacity: 3, side: Side::Right });
    yards.push(y);
    // Same total siding capacity as Inglenook-four, but split across both ends.
    let mut y = Yard::inglenook_with_loop();
    y.name = "Split 5-3 | 3-2 / lead 3 + loop".into();
    y.sidings[2].side = Side::Left;
    y.sidings.push(Siding { name: "Spur".into(), capacity: 2, side: Side::Left });
    yards.push(y);

    println!("{tasks} random tasks each, 8 cars, 5-car consist on siding 0, seed {seed}");
    println!("cost is in legs: pull/push = 2, run-round = 3\n");
    println!("{:<38} {:>6} {:>6} {:>7} {:>5} {:>9} {:>7} {:>8}", "yard", "moves", "legs", "rounds", "max", "unsolved", "budget", "ms/task");
    for y in &yards {
        let st = benchmark(y, 8, 5, 0, tasks, seed);
        println!(
            "{:<38} {:>6.2} {:>6.2} {:>7.2} {:>5} {:>9} {:>7} {:>8.1}",
            y.name,
            st.mean_moves(),
            st.mean_cost(),
            st.mean_run_arounds(),
            st.max_moves,
            st.tasks - st.solved,
            st.budget_hits,
            st.elapsed.as_secs_f64() * 1000.0 / tasks as f64,
        );
    }
}
