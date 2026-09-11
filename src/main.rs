mod checksum;
mod cli;
mod compare;
mod dump;
mod format;
mod record;
mod show;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};

fn main() {
    // Suppress panic output and exit silently on broken pipe (Unix convention)
    std::panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        if msg.contains("Broken pipe") {
            std::process::exit(0);
        }
        eprintln!("fsmeta: {}", msg);
        std::process::exit(1);
    }));

    if let Err(e) = run() {
        eprintln!("fsmeta: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Dump(args) => cmd_dump(&args)?,
        Commands::Compare(args) => cmd_compare(&args)?,
        Commands::Diff(args) => cmd_diff(&args)?,
        Commands::Show(args) => cmd_show(&args)?,
        Commands::List(args) => cmd_list(&args)?,
    }

    Ok(())
}

fn cmd_dump(args: &cli::DumpArgs) -> Result<()> {
    require_piped_stdout()?;
    let config = dump::WalkConfig {
        paths: args.paths.clone(),
        excludes: args.exclude.clone(),
        checksum_size_limit: args.checksum_size_limit,
        from_stdin: args.stdin,
        verbose: args.verbose,
    };

    // Stream records directly to stdout, checking DFS order as we go
    let rx = dump::walk_stream(&config)?;
    stream_dump_records(&rx)
}

fn require_piped_stdout() -> Result<()> {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        anyhow::bail!("stdout is a terminal; pipe output to a file or pipe (e.g. > snapshot.fs or | gzip > snapshot.fs.gz)");
    }
    Ok(())
}

fn stream_dump_records(rx: &std::sync::mpsc::Receiver<crate::record::FileRecord>) -> Result<()> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    // Write header before the walk starts so the file is non-zero immediately
    format::write_header(&mut lock)?;

    let mut last_path: Option<String> = None;
    let mut count = 0u64;
    while let Ok(record) = rx.recv() {
        // Check DFS order (O(1): only needs previous path)
        if let Some(ref prev) = last_path {
            if !crate::format::dfs_after(prev, &record.path) {
                eprintln!(
                    "warning: DFS order violated in dump at \"{}\" (previous: \"{}\")",
                    record.path, prev
                );
            }
        }
        last_path = Some(record.path.clone());
        format::write_record(&record, &mut lock)?;
        count += 1;
    }
    format::write_trailer(count, &mut lock)?;

    Ok(())
}

fn cmd_compare(args: &cli::CompareArgs) -> Result<()> {
    let full_baseline = format::read_snapshot(&args.snapshot_a)?;

    // Filter baseline to only paths under the live_path (preserves sort order)
    let baseline = filter_baseline(&full_baseline, &args.live_path);

    let mut mc = compare::MergeCompare::new(baseline);
    let config = compare_walk_config(&args.live_path, &args.exclude);

    // Stream live records (same DFS order as baseline), merge and emit
    let rx = dump::walk_stream(&config)?;
    while let Ok(record) = rx.recv() {
        for line in mc.process(&record, args.verbose) {
            println!("{}", line);
        }
    }
    for line in mc.finish() {
        println!("{}", line);
    }

    Ok(())
}

fn filter_baseline(
    full_baseline: &[crate::record::FileRecord],
    live_path: &str,
) -> Vec<crate::record::FileRecord> {
    let live_prefix = live_path.trim_end_matches('/');
    let live_prefix: &str = if live_prefix.is_empty() {
        "/"
    } else {
        live_prefix
    };
    if live_prefix == "/" {
        return full_baseline.to_vec();
    }
    let prefix_with_slash = format!("{}/", live_prefix);
    full_baseline
        .iter()
        .filter(|r| r.path == live_prefix || r.path.starts_with(&prefix_with_slash))
        .cloned()
        .collect()
}

fn compare_walk_config(live_path: &str, exclude: &[String]) -> dump::WalkConfig {
    let mut config = dump::WalkConfig {
        paths: vec![live_path.to_string()],
        excludes: vec![],
        checksum_size_limit: dump::COMPARE_CHECKSUM_LIMIT,
        from_stdin: false,
        verbose: false,
    };
    let mut excludes = vec![
        "/proc".to_string(),
        "/sys".to_string(),
        "/dev/shm".to_string(),
        "/run".to_string(),
    ];
    excludes.extend(exclude.iter().cloned());
    config.excludes = excludes;
    config
}

fn cmd_diff(args: &cli::DiffArgs) -> Result<()> {
    let lines = compare::diff_snapshots(&args.snapshot_a, &args.snapshot_b, args.verbose)?;
    for line in lines {
        println!("{}", line);
    }
    Ok(())
}

fn cmd_show(args: &cli::ShowArgs) -> Result<()> {
    show::show(&args.snapshot, &args.path)?;
    Ok(())
}

fn cmd_list(args: &cli::ListArgs) -> Result<()> {
    let lines = show::list(&args.snapshot, &args.grep, args.count)?;
    for line in lines {
        println!("{}", line);
    }
    Ok(())
}
