use std::{env, ffi::OsStr, io, path::PathBuf, process::ExitCode};

use dasru::{
    memory::{inspect_metrics_layout, scan_metrics},
    probe::{ProbeReport, inspect_executable, run_probe},
};
use serde::Serialize;

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    match arguments.as_slice() {
        [command] if command == OsStr::new("probe") => print_probe(run_probe(), false),
        [command, option] if command == OsStr::new("probe") && option == OsStr::new("--json") => {
            print_probe(run_probe(), true)
        }
        [command, option, path]
            if command == OsStr::new("inspect") && option == OsStr::new("--exe") =>
        {
            print_probe(inspect_executable(&PathBuf::from(path)), false)
        }
        [command, option, path, json_option]
            if command == OsStr::new("inspect")
                && option == OsStr::new("--exe")
                && json_option == OsStr::new("--json") =>
        {
            print_probe(inspect_executable(&PathBuf::from(path)), true)
        }
        [command, subcommand, option]
            if command == OsStr::new("memory")
                && subcommand == OsStr::new("scan")
                && option == OsStr::new("--metrics") =>
        {
            print_metrics(scan_metrics(), false)
        }
        [command, subcommand, option]
            if command == OsStr::new("memory")
                && subcommand == OsStr::new("inspect")
                && option == OsStr::new("--metrics") =>
        {
            print_metrics_layout(inspect_metrics_layout(), false)
        }
        [command, subcommand, option, json_option]
            if command == OsStr::new("memory")
                && subcommand == OsStr::new("scan")
                && option == OsStr::new("--metrics")
                && json_option == OsStr::new("--json") =>
        {
            print_metrics(scan_metrics(), true)
        }
        [command, subcommand, option, json_option]
            if command == OsStr::new("memory")
                && subcommand == OsStr::new("inspect")
                && option == OsStr::new("--metrics")
                && json_option == OsStr::new("--json") =>
        {
            print_metrics_layout(inspect_metrics_layout(), true)
        }
        _ => {
            eprintln!(
                "Usage: dasru probe [--json]\n       dasru inspect --exe <path> [--json]\n       dasru memory scan --metrics [--json]\n       dasru memory inspect --metrics [--json]"
            );
            ExitCode::from(2)
        }
    }
}

fn print_probe(report: ProbeReport, json: bool) -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_max_level(tracing::Level::DEBUG)
        .init();
    print_report(&report, &report.to_human_text(), json, report.exit_code())
}

fn print_metrics(report: dasru::memory::MetricsScanReport, json: bool) -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_max_level(tracing::Level::DEBUG)
        .init();
    print_report(&report, &report.to_human_text(), json, report.exit_code())
}

fn print_metrics_layout(report: dasru::memory::MetricsLayoutReport, json: bool) -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_max_level(tracing::Level::DEBUG)
        .init();
    print_report(&report, &report.to_human_text(), json, report.exit_code())
}

fn print_report<T: Serialize>(report: &T, human: &str, json: bool, exit_code: i32) -> ExitCode {
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(output) => println!("{output}"),
            Err(error) => {
                eprintln!("Could not serialize probe report: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print!("{human}");
    }
    ExitCode::from(exit_code as u8)
}
