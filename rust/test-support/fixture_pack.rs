//! Seekable zstd fixture reads for native Linux tests.
use std::{
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::{Mutex, OnceLock},
};

use serde::Deserialize;

const FRAME_BYTES: usize = 4 * 1024 * 1024;
const CACHE_FRAMES: usize = 32;
const MAX_PAYLOAD_BYTES: u64 = 256 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FRAMES: u32 = 100_000;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn read_u32(file: &mut File) -> io::Result<u32> {
    let mut bytes = [0; 4];
    file.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

#[derive(Deserialize)]
struct Index {
    schema: u32,
    fixtures: BTreeMap<String, (u64, u64, String)>,
    #[serde(default)]
    aliases: BTreeMap<String, (String, String, String)>,
}

struct Frame {
    offset: u64,
    packed: usize,
    expanded: usize,
}

static PACK: OnceLock<Mutex<Option<(std::path::PathBuf, Pack)>>> = OnceLock::new();

#[derive(Default)]
struct DecodeStats {
    frame_hits: u64,
    frame_misses: u64,
    compressed_bytes: u64,
    decoded_bytes: u64,
}

struct Pack {
    file: File,
    index: Index,
    frames: Vec<Frame>,
    cache: VecDeque<(usize, Vec<u8>)>,
    stats: DecodeStats,
}

impl Pack {
    fn open(directory: &Path) -> io::Result<Self> {
        let mut file = File::open(directory.join("fixtures-4MiB.tar.zst"))?;
        let size = file.metadata()?.len();
        if size < 17 {
            return Err(invalid("truncated fixture pack"));
        }
        file.seek(SeekFrom::End(-9))?;
        let count = read_u32(&mut file)?;
        let mut flags = [0];
        file.read_exact(&mut flags)?;
        if read_u32(&mut file)? != 0x8F92EAB1 || flags[0] != 0 || !(1..=MAX_FRAMES).contains(&count)
        {
            return Err(invalid("unsupported fixture pack seek table"));
        }
        let table = size
            .checked_sub(17 + u64::from(count) * 8)
            .filter(|&offset| offset > 0)
            .ok_or_else(|| invalid("invalid fixture pack seek table offset"))?;
        file.seek(SeekFrom::Start(table))?;
        if read_u32(&mut file)? != 0x184D2A5E || read_u32(&mut file)? != count * 8 + 9 {
            return Err(invalid("invalid fixture pack seek table header"));
        }
        let mut frames = Vec::with_capacity(count as usize);
        let mut offset = 0;
        let mut total = 0;
        for i in 0..count {
            let packed = read_u32(&mut file)? as usize;
            let expanded = read_u32(&mut file)? as usize;
            if !(1..=FRAME_BYTES + FRAME_BYTES / 128).contains(&packed)
                || !(1..=FRAME_BYTES).contains(&expanded)
                || (i + 1 < count && expanded != FRAME_BYTES)
            {
                return Err(invalid("invalid fixture pack frame size"));
            }
            frames.push(Frame {
                offset,
                packed,
                expanded,
            });
            offset += packed as u64;
            total += expanded as u64;
        }
        if offset != table {
            return Err(invalid(
                "fixture pack frame sizes do not match the seek table",
            ));
        }

        let mut encoded = Vec::new();
        File::open(directory.join("fixtures.index.json"))?
            .take(MAX_INDEX_BYTES + 1)
            .read_to_end(&mut encoded)?;
        if encoded.len() as u64 > MAX_INDEX_BYTES {
            return Err(invalid("fixture pack index exceeds its size limit"));
        }
        let index: Index = serde_json::from_slice(&encoded)?;
        if !matches!(index.schema, 1 | 2) || (index.schema == 1 && !index.aliases.is_empty()) {
            return Err(invalid("unsupported fixture pack index"));
        }
        for (name, (start, length, digest)) in &index.fixtures {
            if !name.ends_with(".json")
                || name.contains(char::from(92))
                || name.split('/').any(|part| matches!(part, "" | "." | ".."))
                || *start > total
                || *length > MAX_PAYLOAD_BYTES.min(total - start)
                || digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(invalid(format!("invalid fixture pack index entry: {name}")));
            }
        }
        for (alias, (representative, old, new)) in &index.aliases {
            let Some(&(start, length, _)) = index.fixtures.get(alias) else {
                return Err(invalid(format!(
                    "missing fixture pack alias entry: {alias}"
                )));
            };
            if index.aliases.contains_key(representative)
                || index
                    .fixtures
                    .get(representative)
                    .is_none_or(|row| (row.0, row.1) != (start, length))
                || ![old, new]
                    .iter()
                    .all(|prefix| prefix.starts_with(r#"{"name":"#) && prefix.ends_with(','))
                || old.len() as u64 > length
                || length - old.len() as u64 + new.len() as u64 > MAX_PAYLOAD_BYTES
            {
                return Err(invalid(format!("invalid fixture pack alias: {alias}")));
            }
        }
        Ok(Self {
            file,
            index,
            frames,
            cache: VecDeque::new(),
            stats: DecodeStats::default(),
        })
    }

    fn frame(&mut self, number: usize) -> io::Result<&[u8]> {
        if let Some(hit) = self.cache.iter().position(|(cached, _)| *cached == number) {
            self.stats.frame_hits += 1;
            let frame = self.cache.remove(hit).unwrap();
            self.cache.push_back(frame);
        } else {
            if self.cache.len() == CACHE_FRAMES {
                self.cache.pop_front();
            }
            let frame = &self.frames[number];
            self.file.seek(SeekFrom::Start(frame.offset))?;
            let mut compressed = vec![0; frame.packed];
            self.file.read_exact(&mut compressed)?;
            let decoded = zstd::bulk::decompress(&compressed, frame.expanded).map_err(|error| {
                invalid(format!(
                    "cannot decode fixture pack frame {number}: {error}"
                ))
            })?;
            if decoded.len() != frame.expanded {
                return Err(invalid(format!(
                    "invalid decoded length for fixture pack frame {number}"
                )));
            }
            self.stats.frame_misses += 1;
            self.stats.compressed_bytes += frame.packed as u64;
            self.stats.decoded_bytes += frame.expanded as u64;
            self.cache.push_back((number, decoded));
        }
        Ok(&self.cache.back().unwrap().1)
    }

    fn read(&mut self, name: &str) -> io::Result<Vec<u8>> {
        let &(mut offset, length, _) = self.index.fixtures.get(name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("fixture {name} is missing from the pack"),
            )
        })?;
        let mut data = Vec::with_capacity(length as usize);
        while data.len() < length as usize {
            let number = (offset / FRAME_BYTES as u64) as usize;
            let within = (offset % FRAME_BYTES as u64) as usize;
            let frame = self.frame(number)?;
            let take = (length as usize - data.len()).min(frame.len() - within);
            data.extend_from_slice(&frame[within..within + take]);
            offset += take as u64;
        }
        if let Some((_, old, new)) = self.index.aliases.get(name) {
            use sha2::{Digest, Sha256};

            if !data.starts_with(old.as_bytes()) {
                return Err(invalid(format!(
                    "fixture pack alias prefix mismatch: {name}"
                )));
            }
            drop(data.splice(..old.len(), new.bytes()));
            if format!("{:x}", Sha256::digest(&data)) != self.index.fixtures[name].2 {
                return Err(invalid(format!(
                    "fixture pack alias checksum mismatch: {name}"
                )));
            }
        }
        Ok(data)
    }
}

pub(super) fn read(directory: &Path, relative: &Path) -> io::Result<Vec<u8>> {
    let mut pack = PACK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| io::Error::other("fixture pack reader lock poisoned"))?;
    if pack
        .as_ref()
        .is_none_or(|(current, _)| current != directory)
    {
        *pack = Some((directory.to_owned(), Pack::open(directory)?));
    }
    let name = relative
        .to_str()
        .ok_or_else(|| invalid("invalid UTF-8 fixture path"))?;
    pack.as_mut().unwrap().1.read(name)
}

#[allow(dead_code)]
pub(super) fn report_decode_stats() {
    if let Some(reader) = PACK.get()
        && let Some((_, pack)) = &*reader.lock().unwrap()
    {
        let stats = &pack.stats;
        eprintln!(
            "fixture-pack-stats {}",
            serde_json::json!({
                "frame_hits": stats.frame_hits,
                "frame_misses": stats.frame_misses,
                "compressed_bytes": stats.compressed_bytes,
                "decoded_bytes": stats.decoded_bytes,
            })
        );
    }
}
