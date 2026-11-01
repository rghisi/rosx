#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use usrlib::println;
use usrlib::syscall::Syscall;

mod alloc_pairs;

struct Benchmark {
    name: &'static str,
    run: fn(),
}

const BENCHMARKS: &[Benchmark] = &[Benchmark {
    name: "alloc",
    run: alloc_pairs::run,
}];

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };

    if requested.first().copied() == Some("list") {
        println!("Available benchmarks: {}", available_names());
        return;
    }

    let selected = match select(requested) {
        Ok(benchmarks) => benchmarks,
        Err(unknown) => {
            println!(
                "Unknown benchmark: {} (available: {})",
                unknown,
                available_names()
            );
            return;
        }
    };

    println!("Running: {}", names_of(&selected));
    for benchmark in selected {
        run_benchmark(benchmark);
    }

    println!("=== Bench Finished ===");
}

fn select<'a>(requested: &[&'a str]) -> Result<Vec<&'static Benchmark>, &'a str> {
    if requested.is_empty() {
        return Ok(BENCHMARKS.iter().collect());
    }
    let mut selected = Vec::new();
    for name in requested {
        let benchmark = BENCHMARKS
            .iter()
            .find(|benchmark| benchmark.name == *name)
            .ok_or(*name)?;
        selected.push(benchmark);
    }
    Ok(selected)
}

fn run_benchmark(benchmark: &'static Benchmark) {
    if let Some(fh) = Syscall::exec(benchmark.run as *const () as usize) {
        Syscall::wait_future(fh);
    } else {
        println!("Cannot spawn benchmark: {}", benchmark.name);
    }
}

fn available_names() -> String {
    BENCHMARKS
        .iter()
        .map(|benchmark| benchmark.name)
        .collect::<Vec<&str>>()
        .join(", ")
}

fn names_of(selected: &[&'static Benchmark]) -> String {
    selected
        .iter()
        .map(|benchmark| benchmark.name)
        .collect::<Vec<&str>>()
        .join(", ")
}
