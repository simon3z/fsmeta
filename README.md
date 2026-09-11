# File Metadata Comparison Tool

[![GitHub repository](https://img.shields.io/badge/github-simon3z/fsmeta-6f57b0.svg?logo=github)](https://github.com/simon3z/fsmeta)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux-blue.svg)](https://www.kernel.org/)
[![Rust edition: 2021](https://img.shields.io/badge/rust-2021-orange.svg)](https://doc.rust-lang.org/edition-guide)
[![Dependencies: 4](https://img.shields.io/badge/dependencies-4-green.svg)](Cargo.toml)
[![Quality](https://github.com/simon3z/fsmeta/actions/workflows/quality.yml/badge.svg)](https://github.com/simon3z/fsmeta/actions/workflows/quality.yml)
[![Tests: 87](https://img.shields.io/badge/tests-87-brightgreen.svg)](tests/)

Filesystem metadata dump and diff tool for Linux.

## Overview

`fsmeta` captures full filesystem metadata (file type, permissions, ownership,
hard links, size, mtime, SHA1 checksum, symlink target, device numbers, xattrs,
file attributes) as a portable binary snapshot, and diffs two snapshots in a
scannable diff format.

### Use Cases

1. **Post-OS-upgrade review:** Dump a fresh VM (baseline) and your production
   machine, then diff to find leftover files, modified configs, and drift.
2. **Drift monitoring:** Dump a machine today and again next month to see what
   changed on a running system.

### Comparison with mtree

[mtree(8)](https://www.freebsd.org/cgi/man.cgi?query=mtree&sektion=8) is a BSD
utility that creates and verifies file hierarchy manifests. Linux ports exist:
[nmtree](https://github.com/archiecobbs/nmtree) (NetBSD port, C) and
[go-mtree](https://github.com/vbatts/go-mtree) (Go, also supports tar archives
and non-upstream xattr keywords).

| | mtree | fsmeta |
|---|---|---|
| Paradigm | Verify: does this filesystem match this manifest? | Compare: what differs between two states, field by field? |
| Per-field diff | No (binary: MISMATCH / MISSING / EXTRA) | Yes (13 flag columns: owner, mode, xattrs, type, etc.) |
| xattrs | No (upstream); go-mtree adds non-upstream support | Yes (all namespaces) |
| lsattr (a, A, c, d, i, u) | No | Yes |
| Hard link count | Yes | Yes |
| Diff two snapshots | No (manifest vs filesystem only) | Yes (`fsmeta diff a.fs b.fs`) |
| Format | Text, fixed schema | Binary, versioned, extensible |
| Output | Human-readable verify report | Scannable one-line-per-file flags |

**Use mtree** when you need a quick integrity check ("is my system intact?")
or want to integrate with existing BSD/FreeBSD manifest tooling.

**Use fsmeta** when you need to know *what* changed (owner? mode? content?
SELinux label?), diff two arbitrary snapshots, or capture xattrs and file
attributes.

## Building

```sh
cargo build --release
```

Requires:
- Rust (stable)
- Linux (kernel ≥ 4.19)

## Usage

```
fsmeta dump [OPTIONS] <PATHS...>
fsmeta compare <SNAPSHOT_A> <LIVE_PATH> [OPTIONS]
fsmeta diff <SNAPSHOT_A> <SNAPSHOT_B> [OPTIONS]
fsmeta show <SNAPSHOT> <PATH>
fsmeta list <SNAPSHOT> [OPTIONS]
```

### Examples

```sh
# Dump a filesystem (excludes /proc, /sys, /dev/shm, /run by convention)
fsmeta dump --exclude=/proc --exclude=/sys / > baseline.fs

# Dump from stdin (pipe find output)
find /etc -type f | fsmeta dump --stdin > etc-only.fs

# Compare baseline against live filesystem
fsmeta compare baseline.fs /

# Diff two snapshots
fsmeta diff baseline.fs current.fs

# Show a single file's record
fsmeta show baseline.fs /etc/hostname

# List all paths (optionally filter by prefix)
fsmeta list baseline.fs --grep /etc
fsmeta list baseline.fs --count
```

### dump Options

| Flag | Default | Description |
|------|---------|-------------|
| `--exclude=PATH` | — | Skip subtree (repeatable) |
| `--stdin` | off | Read paths from stdin (one per line) |
| `--checksum-size-limit=N` | 1 GB | Skip checksum for files > N bytes |

### compare Options

| Flag | Default | Description |
|------|---------|-------------|
| `--exclude=PATH` | — | Skip subtree in live walk (repeatable) |
| `--verbose` | off | Inline detail for flagged fields |

### diff Options

| Flag | Default | Description |
|------|---------|-------------|
| `--verbose` | off | Inline detail for flagged fields |

### list Options

| Flag | Default | Description |
|------|---------|-------------|
| `--grep=PATTERN` | — | Filter by path prefix (repeatable) |
| `--count` | off | Show file count only |

## Diff Output Format

The diff output is a scannable one-line-per-file format, inspired by the rpm
verification output:

```
PREFIX FLAGS  PATH
```

- **PREFIX:** `!` (leftover), `-` (missing), ` ` (in both)
- **FLAGS:** 13 characters, one per flag position (`.` = no change)

| Pos | Flag | Meaning |
|-----|------|---------|
| 0   | s    | size differs |
| 1   | M    | mode differs |
| 2   | D    | device numbers differ |
| 3   | G    | group differs |
| 4   | U    | user/owner differs |
| 5   | P    | permissions differ (setuid/setgid/sticky) |
| 6   | L    | hard link count differs |
| 7   | T    | mtime differs |
| 8   | S    | symlink target differs |
| 9   | C/?  | content differs (C=mismatch, ?=skipped) |
| 10  | X    | xattrs differ |
| 11  | A    | file attributes differ (lsattr) |
| 12  | F    | file type changed |

## Working Example

A typical workflow: snapshot a fresh install as the baseline, snapshot the
machine after an OS upgrade, and diff the two to see what changed.

```sh
# 1. Snapshot a fresh install (baseline), then the upgraded system
fsmeta dump / > baseline.fs
fsmeta dump / > current.fs

# 2. Diff the two snapshots — flag columns, then the path
$ fsmeta diff baseline.fs current.fs
 s........C...  /etc/nginx/nginx.conf
!.............  /tmp/core
```

Reading the diff output:
- ` s........C...  /etc/nginx/nginx.conf` — modified: size `s` and content
  `C` (checksum) changed, everything else identical.
- `!.............  /tmp/core` — leftover: present now, absent in the baseline.

For a single flagged file, `--verbose` prints the before → after values:

```sh
$ fsmeta diff --verbose baseline.fs current.fs
 s........C...  /etc/nginx/nginx.conf (s:2479→2641 C:a1b2c3d→9f8e7d6)
```

The same comparison can be run against the live filesystem instead of a
second snapshot:

```sh
$ fsmeta compare baseline.fs /
 s........C...  /etc/nginx/nginx.conf
!.............  /tmp/core
```

Inspect a single file, or browse the snapshot:

```sh
# Full metadata record for one file
$ fsmeta show baseline.fs /etc/nginx/nginx.conf
Path:    /etc/nginx/nginx.conf
Type:    Regular
Mode:    100644
User:    root
Group:   root
Perms:   (none)
Attrs:   (none)
Hardlinks: 1
Size:      2479
Mtime:     2025-08-28 14:27:58 (1787927278)
SHA1:      a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0
Xattrs:
  security.selinux: unconfined_u:object_r:etc_t:s0

# List paths under a directory, or count everything
$ fsmeta list baseline.fs --grep /etc/nginx
/etc/nginx
/etc/nginx/conf.d
/etc/nginx/nginx.conf
$ fsmeta list baseline.fs --count
14286
```

## Snapshot Format

Binary, versioned, extensible. See [FORMAT.md](FORMAT.md) for the full spec.

## License

[Apache-2.0](LICENSE)
