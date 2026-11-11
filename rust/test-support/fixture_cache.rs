//! Test-only, case-at-a-time reader. Plain files remain useful for authoring.
use std::{
    fs::File,
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
};

fn local(path: &Path) -> io::Result<Box<dyn Read>> {
    match File::open(path) {
        Ok(file) => Ok(Box::new(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut compressed = path.as_os_str().to_owned();
            compressed.push(".gz");
            Ok(Box::new(flate2::read::GzDecoder::new(File::open(
                compressed,
            )?)))
        }
        Err(error) => Err(error),
    }
}

pub fn open(path: impl AsRef<Path>) -> io::Result<Box<dyn Read>> {
    let path = path.as_ref();
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut candidates = vec![path.to_path_buf()];
    if let Ok(relative) = path.strip_prefix(&base) {
        if let Some(cache) = std::env::var_os("GOA2_FIXTURE_CACHE") {
            candidates.push(PathBuf::from(cache).join(relative));
        }
        candidates.push(base.join("smoke").join(relative));
    }
    for candidate in candidates {
        match local(&candidate) {
            Ok(reader) => return Ok(Box::new(BufReader::new(reader))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "Missing fixture {} (plain or .gz). Quick inputs ship in fixtures/smoke; for full parity set GOA2_FIXTURE_CACHE and run tools/fixtures.py verify --full. See docs/testing.md. Regenerate only with the pinned oracle.",
            path.display()
        ),
    ))
}

#[allow(dead_code)] // Shared by integration targets needing only open().
pub fn text(path: impl AsRef<Path>) -> String {
    let mut text = String::new();
    open(path)
        .expect("fixture cache unavailable")
        .read_to_string(&mut text)
        .expect("invalid fixture cache");
    text
}
