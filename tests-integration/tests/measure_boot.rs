use std::time::{Duration, Instant};
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const DEFAULT_SAMPLES: usize = 5;
const INIT_MARKER: &str = "[KERNEL] Initializing";
const KERNEL_MARKER: &str = "[KERNEL] Starting";
const PROMPT_MARKER: &str = "rose>";

fn sample_count() -> usize {
    std::env::var("ROSX_BOOT_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SAMPLES)
}

fn median(times: &[Duration]) -> Duration {
    let mut sorted = times.to_vec();
    sorted.sort();
    sorted[sorted.len() / 2]
}

fn report(label: &str, times: &[Duration]) {
    println!(
        "{label} over {} samples: min={:?} median={:?} max={:?}",
        times.len(),
        times.iter().min().expect("at least one sample"),
        median(times),
        times.iter().max().expect("at least one sample"),
    );
}

#[test]
#[ignore = "boot-to-prompt distribution, not a gate; run with -- --ignored --nocapture, ROSX_BOOT_SAMPLES sets the count"]
fn boot_to_prompt_distribution() {
    let samples = sample_count();
    let mut spawn_times = Vec::with_capacity(samples);
    let mut init_times = Vec::with_capacity(samples);
    let mut kernel_times = Vec::with_capacity(samples);
    let mut prompt_times = Vec::with_capacity(samples);

    for index in 0..samples {
        let started = Instant::now();
        let session = QemuSession::spawn();
        let spawned = started.elapsed();
        session.expect_output(INIT_MARKER, BOOT_BUDGET);
        let init = started.elapsed();
        session.expect_output(KERNEL_MARKER, BOOT_BUDGET);
        let kernel = started.elapsed();
        session.expect_output(PROMPT_MARKER, BOOT_BUDGET);
        let prompt = started.elapsed();

        println!(
            "boot sample {index}: spawn={spawned:?} init={init:?} kernel={kernel:?} prompt={prompt:?}"
        );
        spawn_times.push(spawned);
        init_times.push(init);
        kernel_times.push(kernel);
        prompt_times.push(prompt);
    }

    report("session spawn", &spawn_times);
    report("boot to init banner", &init_times);
    report("boot to kernel banner", &kernel_times);
    report("boot to prompt", &prompt_times);
    assert_eq!(prompt_times.len(), samples);
}
