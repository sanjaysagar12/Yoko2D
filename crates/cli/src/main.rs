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
        Some("view") => view_command(&args[2..]), // everything after "view" itself is that subcommand's own arguments
        _ => {
            // no subcommand, or an unrecognized one: print usage and fail loudly rather than guessing what was meant
            eprintln!("Usage:");
            eprintln!("  yoko2d-cli formula-check");
            eprintln!("  yoko2d-cli run <action-script.json> [--output <path.json>] [--save-pattern <path.xml>] [--open]");
            eprintln!("  yoko2d-cli view <pattern.xml> [--measurements <path.json>] [--raw]");
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
    let mut save_pattern_path: Option<PathBuf> = None; // set if --save-pattern <path> was given
    let mut open_gui = false; // set if --open was given

    let mut i = 0; // manual index, since --output/--save-pattern each consume two consecutive arguments (the flag and its value)
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
            "--save-pattern" => {
                let Some(value) = args.get(i + 1) else {
                    eprintln!("yoko2d-cli: --save-pattern requires a path argument"); // a bare trailing --save-pattern with nothing after it is a usage error
                    std::process::exit(1);
                };
                save_pattern_path = Some(PathBuf::from(value)); // record the path that follows --save-pattern
                i += 2; // consumed both --save-pattern and its value
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

    if let Some(path) = &save_pattern_path {
        // Reuses the EXACT `document` value execute_action_script returned
        // above: every add/remove/modify action this run performed is
        // already applied, and every unmodified original ToolRecord from
        // any loaded pattern (script.load_pattern_path) is still carried
        // through untouched.
        let xml = io::serialize_document(&document, script.measurements_path.as_deref())
            .unwrap_or_else(|err| {
                eprintln!("yoko2d-cli: failed to serialize pattern: {err}"); // practically unreachable (Document is plain data), but handled rather than assumed
                std::process::exit(1);
            });
        std::fs::write(path, xml).unwrap_or_else(|err| {
            eprintln!("yoko2d-cli: failed to write pattern: {err}"); // e.g. the save-pattern path's parent directory doesn't exist
            std::process::exit(1);
        });
        println!("yoko2d-cli: saved pattern to {}", path.display()); // success message, matching --output's own success-message style
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

/// Describes one `ToolKind`'s own reference/formula/literal fields (never
/// its `name` or id — those are printed separately by `view_command`'s own
/// caller), for the plain-text tool listing `view_command` prints.
///
/// A plain function, not a method on `ToolKind` (which lives in the `core`
/// crate, not this one): keeps this CLI-output-specific formatting
/// entirely local to this binary instead of growing `core_lib`'s own
/// surface area for a single subcommand's display needs.
fn describe_tool(kind: &core_lib::ToolKind) -> String {
    match kind {
        // dispatch on which kind of tool this is, formatting only the fields relevant to that kind
        core_lib::ToolKind::BasePoint { x, y, .. } => format!("x={x}, y={y}"), // a literal coordinate: nothing else to show
        core_lib::ToolKind::EndLine {
            base_point,
            angle_formula,
            length_formula,
            ..
        } => format!(
            "base_point={}, angle={angle_formula:?}, length={length_formula:?}", // {:?} on a &String quotes it, matching this task's spec example
            base_point.raw() // the raw integer id, for a plain-text listing rather than ObjectId's own Debug format
        ),
        core_lib::ToolKind::Line { p1, p2 } => format!("p1={}, p2={}", p1.raw(), p2.raw()), // just the two endpoint ids: no formulas on a Line
        core_lib::ToolKind::AlongLine {
            p1,
            p2,
            length_formula,
            ..
        } => format!(
            "p1={}, p2={}, length={length_formula:?}",
            p1.raw(), // the point measured from
            p2.raw()  // the point defining the direction to continue in
        ),
        core_lib::ToolKind::Normal {
            p1,
            p2,
            length_formula,
            angle_formula,
            ..
        } => format!(
            "p1={}, p2={}, length={length_formula:?}, angle={angle_formula:?}",
            p1.raw(), // the point measured from
            p2.raw()  // the point defining the perpendicular line
        ),
        core_lib::ToolKind::Bisector {
            p1,
            p2,
            p3,
            length_formula,
            ..
        } => format!(
            "p1={}, p2={}, p3={}, length={length_formula:?}",
            p1.raw(), // one angle ray
            p2.raw(), // the angle's vertex
            p3.raw()  // the other angle ray
        ),
        core_lib::ToolKind::Height {
            point,
            line_p1,
            line_p2,
            ..
        } => format!(
            "point={}, line_p1={}, line_p2={}", // no formula fields: Height is pure geometry
            point.raw(),                        // the point being projected
            line_p1.raw(),                      // one point defining the line projected onto
            line_p2.raw()                       // the other point defining the line projected onto
        ),
        core_lib::ToolKind::Midpoint { p1, p2, .. } => {
            format!("p1={}, p2={}", p1.raw(), p2.raw()) // no formula fields: Midpoint is pure geometry
        }
        core_lib::ToolKind::Piece {
            nodes,
            seam_allowance_formula,
            ..
        } => format!(
            "nodes={}, seam_allowance={seam_allowance_formula:?}", // just the node count here: individual node points aren't this summary's focus
            nodes.len() // how many boundary vertices this piece has
        ),
    }
}

/// Parses `view`'s own arguments: a positional pattern-file path, plus the
/// optional `--measurements <path>` and `--raw` flags.
///
/// Same hand-rolled-loop approach as `run_command`'s own argument parsing,
/// for the same "standard library only" reason.
fn view_command(args: &[String]) {
    let mut pattern_path: Option<PathBuf> = None; // the positional pattern-file path, once found
    let mut measurements_path: Option<PathBuf> = None; // set if --measurements <path> was given
    let mut raw = false; // set if --raw was given

    let mut i = 0; // manual index, since --measurements consumes two consecutive arguments (the flag and its value)
    while i < args.len() {
        // walk every argument, recognizing flags and treating anything else as the positional pattern path
        match args[i].as_str() {
            "--measurements" => {
                let Some(value) = args.get(i + 1) else {
                    eprintln!("yoko2d-cli: --measurements requires a path argument"); // a bare trailing --measurements with nothing after it is a usage error
                    std::process::exit(1);
                };
                measurements_path = Some(PathBuf::from(value)); // record the path that follows --measurements
                i += 2; // consumed both --measurements and its value
            }
            "--raw" => {
                raw = true; // a boolean flag: no following value to consume
                i += 1; // consumed just this one argument
            }
            other => {
                if pattern_path.is_some() {
                    // a second positional argument: this subcommand only accepts one pattern path
                    eprintln!("yoko2d-cli: unexpected argument {other:?}");
                    std::process::exit(1);
                }
                pattern_path = Some(PathBuf::from(other)); // the first non-flag argument is the pattern-file path
                i += 1; // consumed this one argument
            }
        }
    }

    let Some(pattern_path) = pattern_path else {
        eprintln!("yoko2d-cli: view requires a path to a pattern file"); // no positional argument was ever found, matching run_command's own "missing positional argument" convention
        std::process::exit(1);
    };

    let contents = std::fs::read_to_string(&pattern_path).unwrap_or_else(|err| {
        eprintln!("yoko2d-cli: failed to read pattern file: {err}"); // e.g. file not found
        std::process::exit(1);
    });

    if raw {
        // Bypasses this command's own summary formatting entirely: this
        // mode exists for piping/diffing/scripting against the literal XML
        // text, not for a human-readable summary.
        print!("{contents}"); // print the exact raw file contents verbatim, including whatever trailing newline (or lack of one) the file itself has
        return; // done: nothing below this point runs in --raw mode
    }

    let (document, embedded_measurements_path) = io::deserialize_document(&contents)
        .unwrap_or_else(|err| {
            eprintln!("yoko2d-cli: failed to load pattern: {err}"); // e.g. malformed XML, or an unknown tool type
            std::process::exit(1);
        });

    println!("Tools:"); // header for the always-shown tool-definition listing
    for record in document.history() {
        // print every ToolRecord, in history order, regardless of whether resolved geometry ends up available below
        let variant = action_script::tool_kind_variant_name(&record.kind); // e.g. "BasePoint", "EndLine", "Line"
        let name_part = action_script::tool_name(&record.kind) // this tool's user-facing name, if it has one
            .map(|name| format!(" name={name:?}")) // quoted, with a leading space, ready to splice into the line below
            .unwrap_or_default(); // Line has no name: print nothing extra for it
        let details = describe_tool(&record.kind); // this tool's own reference/formula/literal fields
        println!(
            "  id={} {variant}{name_part} {details}", // one line per tool: id, kind, optional name, then its own reference/formula fields
            record.id.raw()                           // the raw integer id, for a plain-text listing
        );
    }

    // Script-level `--measurements` takes precedence; otherwise fall back
    // to the pattern file's own embedded `<measurementsPath>`, the same
    // fallback order Part 1's `load_pattern_path` handling uses. If
    // neither is present, no measurements are applied at all, and any
    // formula referencing an undefined variable simply fails to recompute
    // below — handled gracefully, not treated as fatal for this command.
    let effective_measurements_path = measurements_path
        .clone()
        .or_else(|| embedded_measurements_path.clone().map(PathBuf::from));

    let mut doc_for_recompute = document.clone(); // clone so this command's own measurement application never mutates the Document the tool listing above already printed from
    if let Some(path) = &effective_measurements_path {
        // Best-effort: a measurements file that fails to load is treated
        // exactly like "no measurements were available" — the recompute
        // attempt below still runs (and may still succeed, e.g. if every
        // formula in this pattern is a literal number needing no
        // variables), and any failure it hits is reported by the same
        // graceful "could not resolve geometry" message either way, rather
        // than adding a second, separate fatal-error path here.
        if let Ok(measurements) = io::load_measurements_from_file(path) {
            doc_for_recompute.apply_measurements(measurements); // seed the variables this pattern's formulas may reference
        }
    }

    match core_lib::recompute_all(&doc_for_recompute) {
        Ok(data) => {
            // Viewing a pattern's STRUCTURE (above) should work even
            // without its measurement file on hand; only this
            // resolved-coordinates section needs a successful recompute.
            match output::build_output(&data) {
                Ok(resolved) => {
                    println!(); // blank line separating the two sections
                    println!("Resolved geometry:"); // header for the resolved-coordinates section
                    for point in &resolved.points {
                        // one line per resolved point, in ascending-id order (build_output's own ordering guarantee)
                        println!("  point id={} x={} y={}", point.id, point.x, point.y);
                    }
                    for line in &resolved.lines {
                        // one line per resolved line, showing both endpoint ids and their already-resolved coordinates
                        println!(
                            "  line id={} p1={} ({}, {}) -> p2={} ({}, {})",
                            line.id, line.p1, line.x1, line.y1, line.p2, line.x2, line.y2
                        );
                    }
                }
                Err(err) => {
                    println!("yoko2d-cli: could not resolve geometry ({err}) — showing tool definitions only");
                    // e.g. a dangling line reference (shouldn't normally happen after a successful recompute, but handled rather than assumed)
                }
            }
        }
        Err(err) => {
            println!(
                "yoko2d-cli: could not resolve geometry ({err}) — showing tool definitions only"
            ); // e.g. a formula referencing a measurement that was never loaded
        }
    }
}
