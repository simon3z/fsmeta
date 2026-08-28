# fsmeta

Filesystem metadata dump and diff tool.

## Format Spec (v1)

### Ordering

Records MUST be written in DFS (depth-first, pre-order) traversal order with
sorted siblings: for each directory, emit the directory record, then
recursively emit its children in byte-sorted filename order. This means a
directory and all its descendants form a contiguous block.

This is the natural order produced by the walk, and `compare`/`diff` rely on
it for streaming merge.

### Header

```
u16  version       // 1
```

No file count in the header — records are streamed, so the writer
does not need to know the total upfront.

### Record

Each record is a sequence of tagged fields, terminated by a sentinel.

```
Repeating:
  u16  tag          // field identifier
  u32  length       // byte length of value
  bytes value       // field data

Terminated by:
  u16  0xFFFF       // end-of-record sentinel
  (no length, no value)
```

### Trailer

After all records, a trailer provides post-hoc verification:

```
u16  0xFE00         // trailer magic (unmistakable: never a valid tag)
u64  file_count     // total records written
```

### Tag Table

| Tag     | Name             | Type           | Applies To       |
|---------|------------------|----------------|------------------|
| 0x0001  | path             | string (UTF-8) | all              |
| 0x0002  | type             | u8             | all              |
| 0x0003  | mode             | u32            | all              |
| 0x0004  | user             | string         | all              |
| 0x0005  | group            | string         | all              |
| 0x0006  | perms            | u8             | all              |
| 0x0007  | hardlinks        | u32            | all              |
| 0x0008  | size             | u64            | regular files    |
| 0x0009  | mtime            | i64            | regular files    |
| 0x000A  | checksum         | bytes (20)     | regular files    |
| 0x000B  | (reserved)       | —              | —                |
| 0x000C  | symlink_target   | string         | symlinks         |
| 0x000D  | dev_major        | u32            | char/block       |
| 0x000E  | dev_minor        | u32            | char/block       |
| 0x000F  | xattr            | key-value      | all              |
| 0x0010  | file_attrs       | string         | all              |
| 0x0011  | checksum_skipped | (empty)        | regular files    |
| 0xFFFF  | end-of-record    | (sentinel)     |                  |
| 0xFE00  | trailer          | (see above)    |                  |

### Type Values (u8)

| Value | Type        |
|-------|-------------|
| 0     | regular     |
| 1     | symlink     |
| 2     | directory   |
| 3     | char dev    |
| 4     | block dev   |
| 5     | fifo        |
| 6     | socket      |

### xattr Encoding

The xattr tag (0x000F) appears once per xattr. Value layout:

```
u16  key_len
bytes key        // UTF-8, e.g. "security.selinux"
bytes value      // raw bytes
```

### Forward Compatibility

- Readers MUST skip unknown tags (0x0012–0xFFFE).
- 0xFFFF (end-of-record) and 0xFE00 (trailer) are reserved.
- The version field in the header indicates the format version.
- v1 removes the file count from the header; it is now a trailer.

### Checksum

- Tag 0x000A: checksum present. Value = 20-byte SHA1.
- Tag 0x0011: checksum skipped (size threshold). Value = empty (length 0).
- A regular file has EITHER 0x000A OR 0x0011, never both.

### File Attributes (lsattr)

String of characters, one per flag:
- `a` — append-only
- `A` — atime not updated
- `c` — compressed (btrfs)
- `d` — no dump
- `i` — immutable
- `S` — nocow
- `u` — undirty
- (future flags appended as characters)

## CLI

```
fsmeta dump [OPTIONS] <PATHS...>
fsmeta compare <SNAPSHOT_A> <LIVE_PATH> [OPTIONS]
fsmeta diff <SNAPSHOT_A> <SNAPSHOT_B> [OPTIONS]
fsmeta show <SNAPSHOT> <PATH>
fsmeta list <SNAPSHOT> [OPTIONS]
```

### dump Options

| Flag | Default | Description |
|------|---------|-------------|
| `--exclude=PATH` | — | Skip subtree (repeatable) |
| `--stdin` | off | Read paths from stdin (one per line) |
| `--checksum-size-limit=N` | 1 GB | Skip checksum for files > N bytes |
| `--verbose` | off | List files as they are captured (to stderr) |

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

## Output Format (Diff)

```
PREFIX FLAGS PATH
```

- PREFIX: `!` (leftover), `-` (missing), ` ` (in both)
- FLAGS: 13 characters, one per flag position (`.` = no change)
- PATH: the file path

Example:
```
!s.......F  /etc/stray-config
-.......F   /etc/old-config
...C.....   /etc/some.conf
..UG.....   /usr/bin/some-tool
```

Flag positions (0-indexed):

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

## Dependencies

Target: ≤5 direct dependencies.

| Crate | Purpose |
|-------|---------|
| `clap` | CLI parsing |
| `sha1` | SHA1 checksum |
| `anyhow` | Error handling |
| `libc` | Linux syscalls (llistxattr, lgetxattr, ioctl) |
