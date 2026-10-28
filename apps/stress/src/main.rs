#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use usrlib::println;
use usrlib::syscall::Syscall;

mod alignment_test;
mod allocation_test;
mod context_switching;
mod random;

struct Scenario {
    name: &'static str,
    run: fn(),
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "mem",
        run: allocation_test::run,
    },
    Scenario {
        name: "ctx",
        run: context_switching::run,
    },
    Scenario {
        name: "align",
        run: alignment_test::run,
    },
    Scenario {
        name: "mixed",
        run: worker_mixed_load,
    },
];

fn main() {
    println!("=== RosX Test Suite Started ===");

    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };

    if requested.first().copied() == Some("list") {
        println!("Available tests: {}", available_names());
        return;
    }

    let selected = match select(requested) {
        Ok(scenarios) => scenarios,
        Err(unknown) => {
            println!(
                "Unknown test: {} (available: {})",
                unknown,
                available_names()
            );
            return;
        }
    };

    println!("Running: {}", names_of(&selected));
    for scenario in selected {
        run_scenario(scenario);
    }

    println!("=== Main Thread Finished ===");
}

fn select<'a>(requested: &[&'a str]) -> Result<Vec<&'static Scenario>, &'a str> {
    if requested.is_empty() {
        return Ok(SCENARIOS.iter().collect());
    }
    let mut selected = Vec::new();
    for name in requested {
        let scenario = SCENARIOS
            .iter()
            .find(|scenario| scenario.name == *name)
            .ok_or(*name)?;
        selected.push(scenario);
    }
    Ok(selected)
}

fn run_scenario(scenario: &'static Scenario) {
    if let Some(fh) = Syscall::exec(scenario.run as *const () as usize) {
        Syscall::wait_future(fh);
    } else {
        println!("Cannot spawn test: {}", scenario.name);
    }
}

fn available_names() -> String {
    SCENARIOS
        .iter()
        .map(|scenario| scenario.name)
        .collect::<Vec<&str>>()
        .join(", ")
}

fn names_of(selected: &[&'static Scenario]) -> String {
    selected
        .iter()
        .map(|scenario| scenario.name)
        .collect::<Vec<&str>>()
        .join(", ")
}

fn worker_mixed_load() {
    println!("[MixWorker] Starting Mixed Load Test...");

    if let Some(mem) = Syscall::exec(allocation_test::run as *const () as usize) {
        if let Some(ctx) =
            Syscall::exec(context_switching::worker_context_switch as *const () as usize)
        {
            Syscall::wait_future(mem);
            Syscall::wait_future(ctx);
        }
    }

    println!("[MixWorker] Finished Mixed Load Test...");
}
