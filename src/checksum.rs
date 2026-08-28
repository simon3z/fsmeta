//! SHA1 checksum computation.

use anyhow::Result;
use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::Read;

/// Compute SHA1 of a file's content.
pub fn sha1_checksum(path: &std::path::Path) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hasher.finalize().to_vec())
}
