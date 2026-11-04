use std::{env, io};

use dasru::probe::run_probe;

fn main() {
    let arguments: Vec<_> = env::args().skip(1).collect();
    let json = match arguments.as_slice() {
        [command] if command == "probe" => false,
        [command, option] if command == "probe" && option == "--json" => true,
        _ => {
            eprintln!("Usage: dasru probe [--json]");
            std::process::exit(2);
        }
    };

    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let report = run_probe();
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(output) => println!("{output}"),
            Err(error) => {
                eprintln!("Could not serialize probe report: {error}");
                std::process::exit(1);
            }
        }
    } else {
        print!("{}", report.to_human_text());
    }
}
