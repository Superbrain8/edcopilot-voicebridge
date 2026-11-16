//! Build the command map from one or more `.vap` files and print facts and
//! timings. It reads the files and writes nothing.
//!
//! cargo run -p edcopilot-voicebridge-core --example profile_build -- FILE.vap [FILE.vap ...]

use std::collections::BTreeMap;
use std::time::Instant;

use edcopilot_voicebridge_core::profile::{build_map, record};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: profile_build FILE.vap [FILE.vap ...]");
        std::process::exit(2);
    }
    for path in paths {
        println!("== {path}");
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("   cannot read: {e}");
                continue;
            }
        };
        // Time the full build several times; report the best and the first run.
        let mut times = Vec::new();
        let mut result = None;
        for _ in 0..5 {
            let start = Instant::now();
            let r = build_map(&bytes);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            result = Some(r);
        }
        let start = Instant::now();
        let inflated = record::inflate(&bytes);
        let inflate_ms = start.elapsed().as_secs_f64() * 1000.0;
        match result.unwrap() {
            Ok(built) => {
                let best = times.iter().cloned().fold(f64::INFINITY, f64::min);
                println!(
                    "   build: first {:.1} ms, best of 5 {:.1} ms (inflate alone {:.1} ms)",
                    times[0], best, inflate_ms
                );
                let report = &built.report;
                println!("   source: {:?}", report.source);
                println!("   format: {}", report.format);
                let mut by_type = BTreeMap::new();
                for c in &built.commands {
                    *by_type.entry(format!("{:?}", c.command_type)).or_insert(0) += 1;
                }
                let mut by_class = BTreeMap::new();
                for l in &report.left_out {
                    *by_class.entry(l.class.as_str()).or_insert(0) += 1;
                }
                println!(
                    "   records {} {:?}, left out {} {:?}, profile variables {}",
                    built.commands.len(),
                    by_type,
                    report.left_out.len(),
                    by_class,
                    built.profile_vars.len()
                );
            }
            Err(e) => {
                println!("   build failed: {e}");
                // Show what the walk reads, without the rules.
                if let Ok(data) = inflated {
                    match record::walk(&data) {
                        Ok(p) => println!(
                            "   walk: {} commands, {} actions, field counts {:?}",
                            p.commands.len(),
                            p.commands.iter().map(|c| c.actions.len()).sum::<usize>(),
                            p.counts
                        ),
                        Err(e) => println!("   walk failed: {e}"),
                    }
                }
            }
        }
    }
}
