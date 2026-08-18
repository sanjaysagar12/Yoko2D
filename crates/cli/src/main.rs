mod action_script; // ActionScript, Action, ActionScriptError, load_action_script, execute_action_script
mod output; // PatternOutput, build_output, write_output_to_file

use std::path::PathBuf; // the path type every file-path argument below is parsed into

/// Entry point: dispatches on the first CLI argument to pick a subcommand.
///
/// Kept as a thin `match` over `std::env::args()` rather than pulling in a
/// dependency like `clap`: this binary only has two subcommands and a
/// handful of flags, well within what the standard library alone can
/// parse clearly.
fn main() {
    let args: Vec<String> = std::env::args().collect(); // the whole argv, including argv[0] (this binary's own path)
    let subcommand = args.get(1).map(String::as_str); // the first real argument (if any); selects which subcommand runs

    match subcommand {
        // dispatch on which subcommand was requested
        Some("formula-check") => formula_check(), // the original Phase-0 formula smoke test, moved here from the old default (no-argument) behavior
        Some("run") => run_command(&args[2..]), // everything after "run" itself is that subcommand's own arguments
        _ => {
            // no subcommand, or an unrecognized one: print usage and fail loudly rather than guessing what was meant
            eprintln!("Usage:");
            eprintln!("  yoko2d-cli formula-check");
            eprintln!("  yoko2d-cli run <action-script.json> [--output <path.json>] [--open]");
            std::process::exit(1); // a missing/unknown subcommand is a usage error: exit non-zero, never panic
        }
    }
}

/// Exercises the real tokenize -> parse -> evaluate formula pipeline on a
/// fixed expression, printing its result.
///
/// This is the exact behavior `main()` used to run unconditionally before
/// this phase's real `run` subcommand existed; moved here, under its own
/// subcommand name, so `crates/cli/tests/smoke.rs`'s existing coverage
/// keeps working (now invoking `formula-check` explicitly) rather than
/// being silently dropped.
fn formula_check() {
    let vars = std::collections::HashMap::new(); // no variables needed for this trivial smoke-test formula
    let result = core_lib::formula::eval_formula("1+2*3", &vars) // exercise the real tokenize -> parse -> evaluate pipeline
        .map(|value| value.to_string()) // format the numeric result for printing
        .unwrap_or_else(|err| err.to_string()); // formatting rather than unwrapping keeps this binary panic-free
    println!("yoko2d-cli placeholder — core: {result}"); // unchanged text: crates/cli/tests/smoke.rs asserts on this exact substring
}

/// Parses `run`'s own arguments: a positional action-script path, plus the
/// optional `--output <path>` and `--open` flags.
///
/// A small hand-rolled loop rather than a parsing library, matching this
/// binary's overall "standard library only" argument-handling approach.
fn run_command(args: &[String]) {
    let mut script_path: Option<PathBuf> = None; // the positional action-script path, once found
    let mut output_path: Option<PathBuf> = None; // set if --output <path> was given
    let mut open_gui = false; // set if --open was given

    let mut i = 0; // manual index, since --output consumes two consecutive arguments (the flag and its value)
    while i < args.len() {
        // walk every argument, recognizing flags and treating anything else as the positional script path
        match args[i].as_str() {
            "--output" => {
                let Some(value) = args.get(i + 1) else {
                    eprintln!("yoko2d-cli: --output requires a path argument"); // a bare trailing --output with nothing after it is a usage error
                    std::process::exit(1);
                };
                output_path = Some(PathBuf::from(value)); // record the path that follows --output
                i += 2; // consumed both --output and its value
            }
            "--open" => {
                open_gui = true; // a boolean flag: no following value to consume
                i += 1; // consumed just this one argument
            }
            other => {
                if script_path.is_some() {
                    // a second positional argument: this binary only accepts one script path
                    eprintln!("yoko2d-cli: unexpected argument {other:?}");
                    std::process::exit(1);
                }
                script_path = Some(PathBuf::from(other)); // the first non-flag argument is the action-script path
                i += 1; // consumed this one argument
            }
        }
    }

    let Some(script_path) = script_path else {
        eprintln!("yoko2d-cli: run requires a path to an action script"); // no positional argument was ever found
        std::process::exit(1);
    };

    let script = action_script::load_action_script(&script_path).unwrap_or_else(|err| {
        eprintln!("yoko2d-cli: failed to load action script: {err}"); // e.g. file not found, or malformed JSON
        std::process::exit(1);
    });

    let document = action_script::execute_action_script(&script).unwrap_or_else(|err| {
        eprintln!("yoko2d-cli: failed to execute action script: {err}"); // e.g. an unknown/duplicate point name, or a rejected Document constructor call
        std::process::exit(1);
    });

    let data = core_lib::recompute_all(&document).unwrap_or_else(|err| {
        eprintln!("yoko2d-cli: failed to recompute pattern: {err}"); // e.g. a formula that fails to evaluate, or degenerate geometry
        std::process::exit(1);
    });

    let pattern_output = output::build_output(&data).unwrap_or_else(|err| {
        eprintln!("yoko2d-cli: failed to build output: {err}"); // e.g. a dangling line reference (shouldn't happen after a successful recompute, but handled rather than assumed)
        std::process::exit(1);
    });

    match &output_path {
        Some(path) => {
            output::write_output_to_file(&pattern_output, path).unwrap_or_else(|err| {
                eprintln!("yoko2d-cli: failed to write output: {err}"); // e.g. the output path's parent directory doesn't exist
                std::process::exit(1);
            });
            println!("yoko2d-cli: wrote output to {}", path.display()); // success message, per this subcommand's own contract
        }
        None => {
            // No --output flag: print the JSON to stdout instead of writing a
            // file, so the CLI is pipeable, e.g. `yoko2d-cli run script.json | some-other-tool`.
            let json = serde_json::to_string_pretty(&pattern_output).unwrap_or_else(|err| {
                eprintln!("yoko2d-cli: failed to serialize output: {err}"); // practically unreachable (PatternOutput is plain data), but handled rather than assumed
                std::process::exit(1);
            });
            println!("{json}"); // the JSON itself IS the output here; no extra text, so piping stays clean
        }
    }

    if open_gui {
        // Reuse the SAME measurements path the action script itself used
        // (if any), so the opened window keeps watching/reflecting that
        // same file, exactly as if it had been passed to app::run() directly.
        let measurements_path = script.measurements_path.as_ref().map(PathBuf::from);
        // Only `app`'s public API is touched here — never egui/eframe
        // types by name — so this crate never needs egui/eframe as a
        // direct dependency; see Cargo.toml's comment on the `app` entry.
        if let Err(err) = app::run_with_document(document, measurements_path) {
            eprintln!("yoko2d-cli: failed to open GUI: {err}"); // e.g. the windowing backend failed to initialize
            std::process::exit(1);
        }
    }
}
