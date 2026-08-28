use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "fsmeta",
    version,
    about = "Filesystem metadata snapshot and diff tool",
    long_about = r#"Capture filesystem metadata into portable binary snapshots.

Compare snapshots against each other or against a live filesystem to detect
changes (modified, missing, or leftover files).

Designed for post-OS-upgrade review: snapshot a fresh install, compare
against the live filesystem after an upgrade, and identify leftover,
modified, or missing files.

Snapshots are pipeable (gzip them) and portable (self-contained, versioned
binary format)."#
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Walk filesystem and emit metadata snapshot to stdout
    Dump(DumpArgs),
    /// Compare baseline snapshot (stdin) against live filesystem
    Compare(CompareArgs),
    /// Diff two snapshot files
    Diff(DiffArgs),
    /// Show metadata record for a single file from a snapshot
    Show(ShowArgs),
    /// List all file paths in a snapshot
    List(ListArgs),
}

#[derive(Parser)]
pub struct DumpArgs {
    /// Paths to walk (space-separated, repeatable)
    #[arg(required = true)]
    pub paths: Vec<String>,

    /// Skip subtree (repeatable)
    #[arg(long)]
    pub exclude: Vec<String>,

    /// Read paths from stdin (one per line)
    #[arg(long)]
    pub stdin: bool,

    /// Skip checksum for files larger than N bytes (default: 1GB)
    #[arg(long, default_value = "1073741824")]
    pub checksum_size_limit: u64,

    /// List files as they are captured (to stderr)
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Parser)]
pub struct CompareArgs {
    /// Baseline snapshot file (or "-" for stdin)
    pub snapshot_a: String,

    /// Live filesystem path to walk
    pub live_path: String,

    /// Skip subtree in live walk (repeatable)
    #[arg(long)]
    pub exclude: Vec<String>,

    /// Inline detail for flagged fields
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Parser)]
pub struct DiffArgs {
    /// First snapshot file
    pub snapshot_a: String,

    /// Second snapshot file
    pub snapshot_b: String,

    /// Inline detail for flagged fields
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Parser)]
pub struct ShowArgs {
    /// Snapshot file
    pub snapshot: String,

    /// File path to show
    pub path: String,
}

#[derive(Parser)]
pub struct ListArgs {
    /// Snapshot file
    pub snapshot: String,

    /// Filter by path prefix (like grep, repeatable)
    #[arg(long)]
    pub grep: Vec<String>,

    /// Show file count only
    #[arg(long)]
    pub count: bool,
}
