//! Compare yard topologies by mean optimal plan length over random tasks.
//!
//!     cargo run -p rutot-core --release --bin yardbench [tasks] [seed]

use rutot_core::{benchmark, Siding, Yard};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tasks: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1234);

    let mut yards = vec![Yard::inglenook(), Yard::inglenook_long_lead(), Yard::inglenook_four()];
    // A few more topologies to see the shape of the design space.
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
    y.sidings.push(Siding { name: "S3".into(), capacity: 3 });
    y.sidings.push(Siding { name: "S4".into(), capacity: 3 });
    yards.push(y);

    println!("{tasks} random tasks each, 8 cars, 5-car consist on siding 0, seed {seed}\n");
    println!("{:<32} {:>6} {:>5} {:>8} {:>10}", "yard", "mean", "max", "ms/solve", "capacity");
    for y in &yards {
        let st = benchmark(y, 8, 5, 0, tasks, seed);
        println!(
            "{:<32} {:>6.2} {:>5} {:>8.1} {:>10}",
            y.name,
            st.mean_moves(),
            st.max_moves,
            st.elapsed.as_secs_f64() * 1000.0 / tasks as f64,
            y.total_capacity()
        );
        if st.solved != st.tasks {
            println!("  ! {} of {} tasks unsolved", st.tasks - st.solved, st.tasks);
        }
    }
}
