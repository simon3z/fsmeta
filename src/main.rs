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
    // Canonicalize roots: relative→absolute, fold `.`/`..`, sort, dedupe,
    // drop any root that lies inside another kept root. Sorted, disjoint
    // roots preserve global DFS order.
    let mut roots: Vec<String> = args.paths.iter().map(|p| canon_root(p)).collect();
    roots.sort();
    roots.dedup();
    let mut kept: Vec<String> = Vec::new();
    for r in roots {
        if kept
            .iter()
            .any(|k| dump::is_excluded(&r, std::slice::from_ref(k)))
        {
            continue;
        }
        kept.push(r);
    }
    let config = dump::WalkConfig {
        paths: kept,
        excludes: args.exclude.clone(),
        checksum_size_limit: args.checksum_size_limit,
        from_stdin: args.stdin,
        verbose: args.verbose,
    };

    // Stream records directly to stdout, checking DFS order as we go
    let rx = dump::walk_stream(&config)?;
    stream_dump_records(&rx)
}

/// Canonicalize a root path: absolutize (cwd-joined if relative), fold
/// `.`/`..` components, and normalize slashes. "a/../b" → "/cwd/b".
fn canon_root(p: &str) -> String {
    let path = std::path::Path::new(p);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("/"))
            .join(path)
    };
    let mut parts: Vec<String> = Vec::new();
    for comp in abs.components() {
        match comp {
            std::path::Component::CurDir | std::path::Component::RootDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(c) => {
                parts.push(c.to_string_lossy().into_owned());
            }
            std::path::Component::Prefix(_) => {}
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
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
    let live = canon_root(&args.live_path);

    // User excludes apply to BOTH the live walk and the baseline, so
    // excluded subtrees can never appear as "missing" leftovers.
    let excludes = args.exclude.clone();

    // Filter baseline to live root and excluded subtrees (preserves sort order)
    let baseline = filter_baseline(&full_baseline, &live, &excludes);

    let mut mc = compare::MergeCompare::new(baseline);
    let config = compare_walk_config(&live, &excludes);

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
    excludes: &[String],
) -> Vec<crate::record::FileRecord> {
    let live_prefix = live_path.trim_end_matches('/');
    let live_prefix: &str = if live_prefix.is_empty() {
        "/"
    } else {
        live_prefix
    };
    if live_prefix == "/" {
        return full_baseline
            .iter()
            .filter(|r| !dump::is_excluded(&r.path, excludes))
            .cloned()
            .collect();
    }
    let prefix_with_slash = format!("{}/", live_prefix);
    full_baseline
        .iter()
        .filter(|r| {
            (r.path == live_prefix || r.path.starts_with(&prefix_with_slash))
                && !dump::is_excluded(&r.path, excludes)
        })
        .cloned()
        .collect()
}

fn compare_walk_config(live_path: &str, excludes: &[String]) -> dump::WalkConfig {
    dump::WalkConfig {
        paths: vec![live_path.to_string()],
        excludes: excludes.to_vec(),
        checksum_size_limit: dump::COMPARE_CHECKSUM_LIMIT,
        from_stdin: false,
        verbose: false,
    }
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

#[cfg(test)]
mod tests {
    use super::canon_root;

    #[test]
    fn canon_root_absolute_unchanged() {
        assert_eq!(canon_root("/a/b"), "/a/b");
        assert_eq!(canon_root("//a//b/"), "/a/b");
        assert_eq!(canon_root("/"), "/");
    }

    #[test]
    fn canon_root_collapses_dots() {
        assert_eq!(canon_root("/a/../b"), "/b");
        assert_eq!(canon_root("/a/./b"), "/a/b");
        assert_eq!(canon_root("/a/../../"), "/");
    }

    #[test]
    fn canon_root_relative_becomes_absolute() {
        let cwd = std::env::current_dir().unwrap();
        let cwd_str = cwd.to_string_lossy().to_string();
        let expected = format!("{cwd_str}/x");
        assert_eq!(canon_root("x"), expected);
    }
}
