mod appearance;
mod engine;
mod live;
mod model;
mod ui;

use engine::{store::Store, types::*};
use gtk::{gio, prelude::*};
use std::path::PathBuf;

fn cli(args: &[String]) -> engine::Result<()> {
    let (valued, flags): (&[&str], &[&str]) = match args[0].as_str() {
        "--scan" | "--scan-partition" => (
            &[
                "--scan",
                "--scan-partition",
                "--partition",
                "--workspace",
                "--mode",
                "--extensions",
                "--metadata",
                "--carver",
            ],
            &["--clone", "--include-existing", "--consolidate"],
        ),
        "--discover-partitions" => (
            &[
                "--discover-partitions",
                "--workspace",
                "--table",
                "--timeout",
            ],
            &["--deeper"],
        ),
        "--recover" => (
            &["--recover", "--destination", "--ids"],
            &["--all", "--flat", "--skip", "--all-paths"],
        ),
        "--list" => (&["--list"], &[]),
        "--findings" => (&["--findings", "--id"], &[]),
        "--sources" | "--backends" => (&[], &[args[0].as_str()]),
        _ => return engine::fail("Unknown command. Use --help for usage."),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut cursor = 0;
    while cursor < args.len() {
        let name = args[cursor].as_str();
        if !seen.insert(name) {
            return engine::fail(format!("Duplicate option: {name}"));
        }
        if valued.contains(&name) {
            if args.get(cursor + 1).is_none_or(|v| v.starts_with("--")) {
                return engine::fail(format!("Missing value for {name}"));
            }
            cursor += 2;
        } else if flags.contains(&name) {
            cursor += 1;
        } else {
            return engine::fail(format!("Unknown option: {name}"));
        }
    }
    if seen.contains("--all") && seen.contains("--ids") {
        return engine::fail("Choose --all or --ids, not both.");
    }
    if (args[0] == "--scan" && (seen.contains("--scan-partition") || seen.contains("--partition")))
        || (args[0] == "--scan-partition" && seen.contains("--scan"))
    {
        return engine::fail("Choose --scan SOURCE or --scan-partition REPORT --partition NUMBER.");
    }
    fn argument(args: &[String], key: &str) -> engine::Result<String> {
        let i = args
            .iter()
            .position(|a| a == key)
            .ok_or_else(|| engine::Error::Message(format!("Missing {key}")))?;
        args.get(i + 1)
            .filter(|v| !v.starts_with("--"))
            .cloned()
            .ok_or_else(|| engine::Error::Message(format!("Missing value for {key}")))
    }
    let recovery_failed = std::cell::Cell::new(false);
    let notify = |event: Event| match event {
        Event::Session(dir) => println!("SESSION={}", dir.display()),
        Event::Message(message) => eprintln!("{message}"),
        Event::Stage { text, fraction } => eprintln!(
            "{text}{}",
            fraction
                .map(|f| format!(" · {:.0}%", f * 100.0))
                .unwrap_or_default()
        ),
        Event::Candidates(count) => eprintln!("{count} candidates"),
        Event::Finished {
            directory,
            cancelled,
            error,
        } => {
            println!("SAVED={}", directory.display());
            if cancelled {
                eprintln!("Cancelled");
            }
            if let Some(error) = error {
                eprintln!("{error}");
            }
        }
        Event::Recovered {
            successful,
            failed,
            skipped,
        } => {
            recovery_failed.set(failed > 0);
            println!("Recovered {successful}; failed {failed}; skipped {skipped}");
        }
    };
    if args[0] == "--backends" {
        for (name, version) in engine::adapters::versions() {
            println!("{name}: {version}");
        }
        return Ok(());
    }
    if args[0] == "--sources" {
        println!(
            "{}",
            serde_json::to_string_pretty(&engine::sources::discover()?)?
        );
        return Ok(());
    }
    if args[0] == "--discover-partitions" {
        let source =
            engine::sources::inspect(&PathBuf::from(argument(args, "--discover-partitions")?))?;
        let table = if seen.contains("--table") {
            match argument(args, "--table")?.as_str() {
                "auto" => engine::discovery::Table::Auto,
                "intel" | "mbr" => engine::discovery::Table::Intel,
                "gpt" => engine::discovery::Table::Gpt,
                _ => return engine::fail("Partition table must be auto, intel/mbr or gpt."),
            }
        } else {
            engine::discovery::Table::Auto
        };
        let timeout_seconds = if seen.contains("--timeout") {
            argument(args, "--timeout")?.parse::<u64>().map_err(|_| {
                engine::Error::Message("Timeout must be a nonnegative number of seconds.".into())
            })?
        } else {
            0
        };
        let report = engine::discovery::discover(
            source,
            engine::discovery::Options {
                workspace: PathBuf::from(argument(args, "--workspace")?),
                table,
                deeper: seen.contains("--deeper"),
                timeout_seconds,
            },
            &engine::process::Control::default(),
        )?;
        eprintln!("Partition search report: {}", report.path().display());
        println!("{}", serde_json::to_string_pretty(&report)?);
        if !report.complete {
            return engine::fail(
                report
                    .error
                    .unwrap_or_else(|| "Partition search did not complete.".into()),
            );
        }
        return Ok(());
    }
    if args[0] == "--list" {
        let dir = PathBuf::from(argument(args, "--list")?);
        let store = Store::open(&dir)?;
        let mut query = Query {
            limit: 250,
            ..Default::default()
        };
        loop {
            let page = store.page(&query)?;
            if page.files.is_empty() {
                break;
            }
            for file in page.files {
                println!("{}", serde_json::to_string(&file)?);
            }
            query.offset += 250;
        }
        return Ok(());
    }
    if args[0] == "--findings" {
        let store = Store::open(&PathBuf::from(argument(args, "--findings")?))?;
        let id = argument(args, "--id")?
            .parse::<i64>()
            .map_err(|_| engine::Error::Message("--id must be a group ID from --list.".into()))?;
        store.get(id)?;
        let mut after = 0;
        loop {
            let files = store.findings(id, after)?;
            if files.is_empty() {
                break;
            }
            for file in files {
                after = file.id;
                println!("{}", serde_json::to_string(&file)?);
            }
            if !store.consolidated() {
                break;
            }
        }
        return Ok(());
    }
    if args[0] == "--scan" || args[0] == "--scan-partition" {
        let source = if args[0] == "--scan-partition" {
            let report =
                engine::discovery::load(&PathBuf::from(argument(args, "--scan-partition")?))?;
            let number = argument(args, "--partition")?
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .ok_or_else(|| {
                    engine::Error::Message("Partition numbers start at 1, in report order.".into())
                })?;
            report.target(number)?
        } else {
            engine::sources::inspect(&PathBuf::from(argument(args, "--scan")?))?
        };
        let mut options = ScanOptions {
            workspace: PathBuf::from(argument(args, "--workspace")?),
            ..Default::default()
        };
        if args.contains(&"--mode".into()) {
            options.mode = match argument(args, "--mode")?.as_str() {
                "quick" => Mode::Quick,
                "deep" => Mode::Deep,
                "full" => Mode::Full,
                _ => return engine::fail("Mode must be quick, deep or full."),
            };
        }
        if args.contains(&"--extensions".into()) {
            options.extensions = normalize_extensions(&argument(args, "--extensions")?)?;
        }
        if args.contains(&"--metadata".into()) {
            options.metadata_backend = argument(args, "--metadata")?;
        }
        if args.contains(&"--carver".into()) {
            options.carver = argument(args, "--carver")?;
        }
        options.clone_first = args.contains(&"--clone".into());
        options.include_existing = args.contains(&"--include-existing".into());
        options.consolidate = args.contains(&"--consolidate".into());
        let dir = engine::jobs::scan(
            source,
            options,
            &engine::process::Control::default(),
            &notify,
        )?;
        if !Store::open(&dir)?.manifest.complete {
            return engine::fail(
                "The scan is incomplete. Partial results and engine errors were saved.",
            );
        }
        return Ok(());
    }
    if args[0] == "--recover" {
        let dir = PathBuf::from(argument(args, "--recover")?);
        let store = Store::open(&dir)?;
        if args.contains(&"--all".into()) {
            let mut query = Query {
                limit: 250,
                ..Default::default()
            };
            loop {
                let page = store.page(&query)?;
                if page.files.is_empty() {
                    break;
                }
                store.select(&page.files.iter().map(|f| f.id).collect::<Vec<_>>(), true)?;
                query.offset += 250;
            }
        }
        if args.contains(&"--ids".into()) {
            let ids = argument(args, "--ids")?
                .split(',')
                .map(|v| {
                    v.parse::<i64>().map_err(|_| {
                        engine::Error::Message(
                            "--ids must be comma-separated candidate IDs.".into(),
                        )
                    })
                })
                .collect::<engine::Result<Vec<_>>>()?;
            store.select(&ids, true)?;
        }
        let options = RecoveryOptions {
            destination: PathBuf::from(argument(args, "--destination")?),
            preserve_tree: !args.contains(&"--flat".into()),
            restore_all_paths: args.contains(&"--all-paths".into()),
            conflict: if args.contains(&"--skip".into()) {
                Conflict::Skip
            } else {
                Conflict::Rename
            },
            ..Default::default()
        };
        engine::jobs::recover(
            &dir,
            &options,
            &engine::process::Control::default(),
            &notify,
        )?;
        return if recovery_failed.get() {
            engine::fail("Some files could not be recovered. Inspect the session's events.log.")
        } else {
            Ok(())
        };
    }
    engine::fail("Unknown command. Use --help for usage.")
}
fn main() -> gtk::glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Open Data Recovery\n\nUsage:\n  open-data-recovery                         Native GTK workspace\n  open-data-recovery --demo                  Safe fixture demonstration\n  open-data-recovery --sources | --backends\n  open-data-recovery --scan SOURCE --workspace DIRECTORY [--mode quick|deep|full]\n      [--extensions jpg,pdf] [--metadata ENGINE] [--carver ENGINE] [--clone] [--include-existing] [--consolidate]\n  open-data-recovery --discover-partitions SOURCE --workspace DIRECTORY [--table auto|intel|gpt] [--deeper] [--timeout SECONDS]\n  open-data-recovery --scan-partition REPORT.json --partition NUMBER --workspace DIRECTORY [scan options]\n  open-data-recovery --list SESSION\n  open-data-recovery --findings SESSION --id GROUP_ID\n  open-data-recovery --recover SESSION --destination DIRECTORY [--ids 1,2 | --all] [--flat] [--skip] [--all-paths]\n  open-data-recovery --smoke-test | --live-smoke-test\n\nSources are read-only. Workspace and recovery destination must be on a separate physical drive.\nPartition discovery only searches; it never writes a partition table. Partition numbers start at 1.\nConsolidation is off by default; --consolidate compares contents after scanning.\nRecovery uses the persisted selection unless --ids or --all is supplied.\nConsolidated groups recover one copy each; --all-paths restores every recorded path."
        );
        return gtk::glib::ExitCode::SUCCESS;
    }
    let demo = args
        .first()
        .is_some_and(|a| a == "--demo" || a == "--smoke-test");
    let smoke = args
        .first()
        .is_some_and(|a| a == "--smoke-test" || a == "--live-smoke-test");
    if !args.is_empty() && !demo && !smoke {
        return match cli(&args) {
            Ok(()) => gtk::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                gtk::glib::ExitCode::FAILURE
            }
        };
    }
    let app = gtk::Application::builder()
        .application_id("org.opendatarecovery.App")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| {
        if demo {
            ui::activate(app, smoke)
        } else {
            live::activate(app, smoke)
        }
    });
    app.run_with_args(&["open-data-recovery"])
}
