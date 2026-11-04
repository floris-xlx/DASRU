use std::{env, ffi::OsStr, io, path::PathBuf, process::ExitCode};

use dasru::probe::{inspect_executable, run_probe};

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let (report, json) = match arguments.as_slice() {
        [command] if command == OsStr::new("probe") => (run_probe(), false),
        [command, option] if command == OsStr::new("probe") && option == OsStr::new("--json") => {
            (run_probe(), true)
        }
        [command, option, path]
            if command == OsStr::new("inspect") && option == OsStr::new("--exe") =>
        {
            (inspect_executable(&PathBuf::from(path)), false)
        }
        [command, option, path, json_option]
            if command == OsStr::new("inspect")
                && option == OsStr::new("--exe")
                && json_option == OsStr::new("--json") =>
        {
            (inspect_executable(&PathBuf::from(path)), true)
        }
        _ => {
            eprintln!("Usage: dasru probe [--json]\n       dasru inspect --exe <path> [--json]");
            return ExitCode::from(2);
        }
    };

    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_max_level(tracing::Level::DEBUG)
        .init();

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(output) => println!("{output}"),
            Err(error) => {
                eprintln!("Could not serialize probe report: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print!("{}", report.to_human_text());
    }
    ExitCode::from(report.exit_code() as u8)
}
