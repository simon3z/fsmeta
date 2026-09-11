//! Format v1 structural tests.
//!
//! Verifies the binary layout of the v1 snapshot format:
//! version header, tagged records, trailer with file count.

use std::io::Read;

/// Create a minimal v1 snapshot in memory with a known record.
fn make_v1_snapshot() -> Vec<u8> {
    let mut buf = Vec::new();

    // Header: version=1 (just 2 bytes, no count)
    buf.extend_from_slice(&1u16.to_le_bytes());

    // Record: path="/test", type=Regular(0), mode=0o644
    // tag=0x0001 (path), len=5, value="/test"
    buf.extend_from_slice(&0x0001u16.to_le_bytes());
    buf.extend_from_slice(&5u32.to_le_bytes());
    buf.extend_from_slice(b"/test");

    // tag=0x0002 (type), len=1, value=0 (Regular)
    buf.extend_from_slice(&0x0002u16.to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&0u8.to_le_bytes());

    // tag=0x0003 (mode), len=4, value=0o644 (420)
    buf.extend_from_slice(&0x0003u16.to_le_bytes());
    buf.extend_from_slice(&4u32.to_le_bytes());
    buf.extend_from_slice(&(420u32).to_le_bytes());

    // End-of-record sentinel
    buf.extend_from_slice(&0xFFFFu16.to_le_bytes());

    // Trailer: magic=0xFE00, count=1
    buf.extend_from_slice(&0xFE00u16.to_le_bytes());
    buf.extend_from_slice(&1u64.to_le_bytes());

    buf
}

#[test]
fn test_v1_header() {
    let snap = make_v1_snapshot();
    let mut reader = &snap[..];

    let mut version_buf = [0u8; 2];
    reader.read_exact(&mut version_buf).unwrap();
    let version = u16::from_le_bytes(version_buf);
    assert_eq!(version, 1, "version should be 1");
}

// Read one (tag, length-prefixed value) pair from the stream.
fn read_tag_value(reader: &mut &[u8]) -> (u16, Vec<u8>) {
    let mut tag_buf = [0u8; 2];
    reader.read_exact(&mut tag_buf).unwrap();
    let tag = u16::from_le_bytes(tag_buf);
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).unwrap();
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut value = vec![0u8; len];
    reader.read_exact(&mut value).unwrap();
    (tag, value)
}

// Read one (tag, length-prefixed value) pair and assert both.
fn assert_field(reader: &mut &[u8], expected_tag: u16, expected_value: &[u8], label: &str) {
    let (tag, value) = read_tag_value(reader);
    assert_eq!(tag, expected_tag, "{label} tag");
    assert_eq!(&value, expected_value, "{label} value");
}

#[test]
fn test_v1_record_fields() {
    let snap = make_v1_snapshot();
    let mut reader = &snap[2..]; // skip 2-byte header

    // First tag: path
    assert_field(&mut reader, 0x0001, b"/test", "first tag (path)");

    // Second tag: type
    assert_field(&mut reader, 0x0002, &[0], "second tag (type)");

    // Third tag: mode
    let (tag, value) = read_tag_value(&mut reader);
    assert_eq!(tag, 0x0003, "third tag should be mode");
    let mode = u32::from_le_bytes(value.try_into().unwrap());
    assert_eq!(mode, 420, "mode should be 0o644 (420)");

    // Read end-of-record sentinel
    let mut tag_buf = [0u8; 2];
    reader.read_exact(&mut tag_buf).unwrap();
    let sentinel = u16::from_le_bytes(tag_buf);
    assert_eq!(sentinel, 0xFFFF, "should have end-of-record sentinel");

    // Read trailer
    reader.read_exact(&mut tag_buf).unwrap();
    let trailer_magic = u16::from_le_bytes(tag_buf);
    assert_eq!(trailer_magic, 0xFE00, "should have trailer magic");

    let mut count_buf = [0u8; 8];
    reader.read_exact(&mut count_buf).unwrap();
    let count = u64::from_le_bytes(count_buf);
    assert_eq!(count, 1, "trailer count should be 1");
}

/// Byte-for-byte stability test.
#[test]
fn test_v1_byte_for_byte() {
    let expected = vec![
        // version = 1
        0x01, 0x00, // record: path
        0x01, 0x00, // tag = path
        0x05, 0x00, 0x00, 0x00, // len = 5
        b'/', b't', b'e', b's', b't', // "/test"
        // record: type
        0x02, 0x00, // tag = type
        0x01, 0x00, 0x00, 0x00, // len = 1
        0x00, // Regular
        // record: mode
        0x03, 0x00, // tag = mode
        0x04, 0x00, 0x00, 0x00, // len = 4
        0xA4, 0x01, 0x00, 0x00, // 420 = 0x1A4
        // end-of-record
        0xFF, 0xFF, // trailer
        0x00, 0xFE, // magic = 0xFE00
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // count = 1
    ];

    let actual = make_v1_snapshot();
    assert_eq!(actual, expected, "v1 snapshot must be byte-for-byte stable");
}
