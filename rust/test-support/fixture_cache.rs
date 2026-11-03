//! Test-only lossless fixture reader; no full-corpus extraction or retention.
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

pub fn open(path: impl AsRef<Path>) -> io::Result<Box<dyn Read>> {
    let path = path.as_ref();
    match File::open(path) {
        Ok(file) => Ok(Box::new(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut name = path.as_os_str().to_owned();
            name.push(".gz");
            let file = File::open(&name).map_err(|error| io::Error::new(error.kind(), format!(
                "Missing fixture {} (or .gz cache). Regenerate with scripts/rust_oracle/ using the pinned oracle: {error}", path.display()
            )))?;
            Ok(Box::new(flate2::read::GzDecoder::new(file)))
        }
        Err(error) => Err(error),
    }
}

#[allow(dead_code)] // Integration targets share this module but may need only open().
pub fn text(path: impl AsRef<Path>) -> String {
    let mut text = String::new();
    open(path)
        .expect("fixture cache unavailable")
        .read_to_string(&mut text)
        .expect("invalid fixture cache");
    text
}
