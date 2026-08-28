//! Pretty-print snapshot records: single record and path listing.

use anyhow::Result;

use crate::record::FileRecord;

/// Show the record for a given path from a snapshot file.
pub fn show(snapshot_path: &str, file_path: &str) -> Result<()> {
    let records = crate::format::read_snapshot(snapshot_path)?;

    match records.iter().find(|r| r.path == file_path) {
        Some(record) => {
            print_record(record);
            Ok(())
        }
        None => {
            eprintln!("Path not found in snapshot: {}", file_path);
            Ok(())
        }
    }
}

fn print_record(r: &FileRecord) {
    println!("Path:    {}", r.path);
    println!("Type:    {:?}", r.file_type);
    println!("Mode:    {:o}", r.mode);
    println!("User:    {}", r.user);
    println!("Group:   {}", r.group);
    println!("Perms:   {}", format_perms(r.perms));
    println!("Attrs:   {}", if r.file_attrs.is_empty() { "(none)".to_string() } else { r.file_attrs.clone() });
    println!("Hardlinks: {}", r.hardlinks);

    if let Some(size) = r.size {
        println!("Size:      {}", format_size(size));
    }
    if let Some(mtime) = r.mtime {
        println!("Mtime:     {}", format_mtime(mtime));
    }
    if let Some(checksum) = &r.checksum {
        let hex: Vec<String> = checksum.iter().map(|b| format!("{:02x}", b)).collect();
        println!("SHA1:      {}", hex.join(""));
    }
    if r.checksum_skipped {
        println!("SHA1:      (skipped - size threshold)");
    }
    if let Some(target) = &r.symlink_target {
        println!("Symlink:   -> {}", target);
    }
    if let Some(major) = r.dev_major {
        println!("Device:    {}:{}", major, r.dev_minor.unwrap_or(0));
    }

    if !r.xattrs.is_empty() {
        println!("Xattrs:");
        for (key, value) in &r.xattrs {
            let val_str = if value.len() <= 100 {
                String::from_utf8_lossy(value).to_string()
            } else {
                format!("<{} bytes>", value.len())
            };
            println!("  {}: {}", key, val_str);
        }
    }

}

/// Human-readable size with the raw byte count in parentheses.
fn format_size(bytes: u64) -> String {
    if bytes < 1_024 {
        return bytes.to_string();
    }
    format!("{} ({} bytes)", human_size(bytes), bytes)
}

/// Human-readable size (K/M/G/T), e.g. `31 KB`, `1.4 MB`.
fn human_size(bytes: u64) -> String {
    const KB: f64 = 1_024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let (value, unit) = if bytes < 1_024 {
        (bytes as f64, "B")
    } else if bytes < 1_024 * 1_024 {
        (bytes as f64 / KB, "KB")
    } else if bytes < 1_024 * 1_024 * 1_024 {
        (bytes as f64 / MB, "MB")
    } else if bytes < 1_024u64.pow(4) {
        (bytes as f64 / GB, "GB")
    } else {
        (bytes as f64 / TB, "TB")
    };
    if value < 10.0 {
        format!("{:.1} {}", value, unit)
    } else {
        format!("{} {}", value.round() as u64, unit)
    }
}

/// Human-readable mtime in UTC with the raw epoch in parentheses.
/// UTC is used (not local time) so a snapshot reviewed on a different
/// machine shows the same date; the raw epoch is in parens if you want to
/// convert to your timezone.
fn format_mtime(mtime: i64) -> String {
    format!("{} ({})", format_utc_time(mtime), mtime)
}

/// Convert a unix timestamp to `YYYY-MM-DD HH:MM:SS` in UTC.
fn format_utc_time(mtime: i64) -> String {
    // days since 1970-01-01
    let days = mtime.div_euclid(86400);
    let rem = mtime.rem_euclid(86400);
    let (hour, min, sec) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );
    // civil-from-days algorithm (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - 365 * yoe - yoe / 4 + yoe / 100;
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y - if month <= 2 { 1 } else { 0 };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, min, sec
    )
}

/// Special-bit summary: only the set bits, or `(none)`.
fn format_perms(perms: u8) -> String {
    let mut bits = Vec::new();
    if perms & 1 != 0 {
        bits.push("setuid");
    }
    if perms & 2 != 0 {
        bits.push("setgid");
    }
    if perms & 4 != 0 {
        bits.push("sticky");
    }
    if bits.is_empty() {
        "(none)".to_string()
    } else {
        bits.join(", ")
    }
}

/// List all paths in a snapshot, optionally filtered by prefix.
pub fn list(snapshot_path: &str, prefixes: &[String], count_only: bool) -> Result<Vec<String>> {
    let records = crate::format::read_snapshot(snapshot_path)?;

    if count_only {
        return Ok(vec![format!("{}", records.len())]);
    }

    let paths: Vec<String> = records
        .into_iter()
        .filter(|r| {
            prefixes.is_empty() || prefixes.iter().any(|prefix| r.path.starts_with(prefix.as_str()))
        })
        .map(|r| r.path)
        .collect();
    Ok(paths)
}
