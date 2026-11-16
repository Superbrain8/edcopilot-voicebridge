//! Sends one command token to an EDCoPilot request file and reports the result.
//!
//! Usage: cargo run -p edcopilot-voicebridge-core --example send_token -- <request-file> <token>

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use edcopilot_voicebridge_core::{NotConsumed, Writer};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(token)) = (args.next(), args.next()) else {
        eprintln!("usage: send_token <request-file> <token>");
        return ExitCode::from(2);
    };
    let start = Instant::now();
    match Writer::new(PathBuf::from(path)).send(&token) {
        Ok(()) => {
            println!("consumed after {:?}", start.elapsed());
            ExitCode::SUCCESS
        }
        Err(e) => {
            match e.downcast_ref::<NotConsumed>() {
                Some(n) => println!("not consumed (withdrawn: {}): {e:#}", n.withdrawn),
                None => println!("send failed: {e:#}"),
            }
            ExitCode::FAILURE
        }
    }
}
