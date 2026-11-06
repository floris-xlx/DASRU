use std::{env, ffi::OsStr, io, path::PathBuf, process::ExitCode};

use dasru::{
    doctor::DoctorReport,
    memory::{
        discover_metrics_objects, inspect_metrics_layout, inspect_metrics_layout_with_replacement,
        scan_metrics,
    },
    probe::{ProbeReport, inspect_executable, run_probe},
};
use serde::Serialize;

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == OsStr::new("hotkeys"))
    {
        return dasru::hotkeys::run_cli(&arguments[1..]);
    }
    match arguments.as_slice() {
        [command] if command == OsStr::new("doctor") => {
            print_doctor(dasru::doctor::inspect(), false)
        }
        [command, option] if command == OsStr::new("doctor") && option == OsStr::new("--json") => {
            print_doctor(dasru::doctor::inspect(), true)
        }
        [command] if command == OsStr::new("probe") => print_probe(run_probe(), false),
        [command, option] if command == OsStr::new("probe") && option == OsStr::new("--json") => {
            print_probe(run_probe(), true)
        }
        [command, subcommand, path]
            if command == OsStr::new("support") && subcommand == OsStr::new("export") =>
        {
            print_support_export(&PathBuf::from(path))
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
                && subcommand == OsStr::new("discover")
                && option == OsStr::new("--metrics-objects") =>
        {
            print_metrics_objects(discover_metrics_objects(), false)
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
                && subcommand == OsStr::new("discover")
                && option == OsStr::new("--metrics-objects")
                && json_option == OsStr::new("--json") =>
        {
            print_metrics_objects(discover_metrics_objects(), true)
        }
        [command, subcommand, option, json_option]
            if command == OsStr::new("memory")
                && subcommand == OsStr::new("inspect")
                && option == OsStr::new("--metrics")
                && json_option == OsStr::new("--json") =>
        {
            print_metrics_layout(inspect_metrics_layout(), true)
        }
        [command, subcommand, option, label_option, label]
            if command == OsStr::new("memory")
                && subcommand == OsStr::new("inspect")
                && option == OsStr::new("--metrics")
                && (label_option == OsStr::new("--replacement")
                    || label_option == OsStr::new("--fps-label")) =>
        {
            print_metrics_layout_for_label(label, false)
        }
        [
            command,
            subcommand,
            option,
            label_option,
            label,
            json_option,
        ] if command == OsStr::new("memory")
            && subcommand == OsStr::new("inspect")
            && option == OsStr::new("--metrics")
            && (label_option == OsStr::new("--replacement")
                || label_option == OsStr::new("--fps-label"))
            && json_option == OsStr::new("--json") =>
        {
            print_metrics_layout_for_label(label, true)
        }
        [command, subcommand, marker_option, marker, experimental]
            if command == OsStr::new("memory")
                && subcommand == OsStr::new("osd")
                && marker_option == OsStr::new("--fps-marker")
                && experimental == OsStr::new("--experimental-write") =>
        {
            match marker.to_str() {
                Some(marker) => {
                    print_osd_result(dasru::osd::run_experimental_session(marker, true))
                }
                None => print_usage(),
            }
        }
        [command, subcommand, restore]
            if command == OsStr::new("memory")
                && subcommand == OsStr::new("osd")
                && restore == OsStr::new("restore") =>
        {
            print_osd_result(dasru::osd::restore_fps_marker())
        }
        _ => print_usage(),
    }
}

fn print_probe(report: ProbeReport, json: bool) -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_max_level(tracing::Level::DEBUG)
        .init();
    print_report(&report, &report.to_human_text(), json, report.exit_code())
}

fn print_doctor(report: DoctorReport, json: bool) -> ExitCode {
    print_report(&report, &report.to_human_text(), json, 0)
}

fn print_support_export(path: &std::path::Path) -> ExitCode {
    match dasru::support::export_json(path) {
        Ok(()) => {
            println!("Support report exported.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Support export failed [{}].", error.code());
            ExitCode::FAILURE
        }
    }
}

fn print_metrics(report: dasru::memory::MetricsScanReport, json: bool) -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_max_level(tracing::Level::DEBUG)
        .init();
    print_report(&report, &report.to_human_text(), json, report.exit_code())
}

fn print_metrics_objects(report: dasru::memory::MetricsObjectReport, json: bool) -> ExitCode {
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

fn print_osd_result(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Memory OSD error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_metrics_layout_for_label(label: &OsStr, json: bool) -> ExitCode {
    let Some(label) = label.to_str() else {
        eprintln!("--replacement must be an ASCII label beginning with %i FPS");
        return print_usage();
    };
    if !label.is_ascii() || label.contains('\0') || !label.starts_with("%i FPS") {
        eprintln!("--replacement must be an ASCII label beginning with %i FPS");
        return print_usage();
    }
    print_metrics_layout(inspect_metrics_layout_with_replacement(label), json)
}

fn print_usage() -> ExitCode {
    eprintln!(
        "Usage: dasru doctor [--json]\n       dasru probe [--json]\n       dasru support export <path>\n       dasru inspect --exe <path> [--json]\n       dasru memory scan --metrics [--json]\n       dasru memory inspect --metrics [--replacement <label>] [--json]\n       dasru memory discover --metrics-objects [--json]\n       dasru memory osd --fps-marker + --experimental-write\n       dasru memory osd restore\n       dasru hotkeys listen --bind CTRL+ALT+F9 [--bind <shortcut>]"
    );
    ExitCode::from(2)
}

fn print_report<T: Serialize>(report: &T, human: &str, json: bool, exit_code: i32) -> ExitCode {
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(output) => println!("{output}"),
            Err(error) => {
                eprintln!("Could not serialize report: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print!("{human}");
    }
    ExitCode::from(exit_code as u8)
}
