use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs::{self, File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// One cached object on disk. Written once, never modified (OBM segments, assets and show
/// JSON are fixed for a given key). Dropping it deletes the file: readers that already opened
/// it keep reading, since deleting only removes the name, not the open inode.
struct CacheFile {
    path: PathBuf,
    size: usize,
}

impl CacheFile {
    /// [Dishonest / File I/O]
    /// Writes `buffer` to a new file at `path`. `create_new` fails with AlreadyExists if the name
    /// is taken, so only one CacheFile per name can exist (two would delete each other's file).
    fn create(path: PathBuf, buffer: &[u8]) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        // From here the file is ours: owning it right away lets Drop clean up a failed write
        let entry = Self {
            path,
            size: buffer.len(),
        };
        file.write_all(buffer)?;
        Ok(entry)
    }
}

impl Drop for CacheFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Key -> file table. Only stores and finds files: eviction policy, byte budget, in-flight
/// tracking and stats live outside and call `insert` / `get` / `remove`.
pub struct CacheTable {
    table: Mutex<HashMap<String, CacheFile>>,
    base_path: PathBuf,
}

impl CacheTable {
    /// [Dishonest / File I/O]
    /// Uses `base_path` as the cache folder, creating it and deleting any files left from an
    /// earlier run, so every run starts cold. Only plain files directly inside it are removed.
    pub fn new(base_path: impl AsRef<Path>) -> io::Result<Self> {
        let base_path = base_path.as_ref().to_path_buf();
        fs::create_dir_all(&base_path)?;
        for item in fs::read_dir(&base_path)? {
            let item = item?;
            if item.file_type()?.is_file() {
                fs::remove_file(item.path())?;
            }
        }
        Ok(Self {
            table: Mutex::new(HashMap::new()),
            base_path,
        })
    }

    /// [Dishonest / File I/O]
    /// Stores `buffer` under `key`. Errors with AlreadyExists if the key is cached or being
    /// written by someone else; any other error (e.g. disk full) leaves nothing behind.
    /// The file is written outside the lock and only becomes visible to `get` once complete.
    pub fn insert(&self, key: &str, buffer: &[u8]) -> io::Result<()> {
        if self.lock().contains_key(key) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let entry = CacheFile::create(self.file_path(key), buffer)?;

        let rejected = match self.lock().entry(key.to_string()) {
            Entry::Vacant(slot) => {
                slot.insert(entry);
                None
            }
            Entry::Occupied(_) => Some(entry),
        };
        // Dropped after the lock is released (deletes our file)
        match rejected {
            None => Ok(()),
            Some(_) => Err(io::ErrorKind::AlreadyExists.into()),
        }
    }

    /// [Dishonest / File I/O]
    /// Opens the cached file for `key` with its own read handle (own file offset), plus its size.
    /// The handle stays valid even if the entry is removed while it is being sent.
    pub fn get(&self, key: &str) -> Option<(File, usize)> {
        let table = self.lock();
        let entry = table.get(key)?;
        // Under the lock the file still exists: `remove` deletes it only after taking it out
        let file = File::open(&entry.path).ok()?;
        Some((file, entry.size))
    }

    /// [Dishonest / File I/O]
    /// Removes `key` and deletes its file, returning the freed size (for a byte budget).
    pub fn remove(&self, key: &str) -> Option<usize> {
        let entry = self.lock().remove(key)?;
        // The lock is released at the end of the statement above; the file is deleted here
        Some(entry.size)
    }

    /// [Honest / Pure Domain Logic]
    /// File name for a key: its 64-bit hash in hex, so request paths ('/', '|', '..') never
    /// reach the file system. A collision only makes the second key uncacheable (AlreadyExists).
    fn file_path(&self, key: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        self.base_path.join(format!("{:016x}", hasher.finish()))
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, CacheFile>> {
        // A panic while holding the lock leaves the map itself consistent, so keep using it
        self.table
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Read;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("obm_file_cache_{name}_{}", std::process::id()))
    }

    fn read_all(mut file: File) -> Vec<u8> {
        let mut out = Vec::new();
        file.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn test_insert_get_remove() {
        let dir = test_dir("basic");
        let cache = CacheTable::new(&dir).unwrap();
        let key = "/offload/show/f1.json/0/10/1280/720/race|Sam";

        assert!(cache.get(key).is_none());
        cache.insert(key, b"segment bytes").unwrap();

        let (file, size) = cache.get(key).unwrap();
        assert_eq!(size, 13);
        assert_eq!(read_all(file), b"segment bytes");

        assert_eq!(cache.remove(key), Some(13));
        assert!(cache.get(key).is_none());
        assert_eq!(cache.remove(key), None);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_insert_existing_key_keeps_original() {
        let dir = test_dir("dup");
        let cache = CacheTable::new(&dir).unwrap();

        cache.insert("k", b"first").unwrap();
        let err = cache.insert("k", b"second").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read_all(cache.get("k").unwrap().0), b"first");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_handle_survives_remove() {
        let dir = test_dir("survive");
        let cache = CacheTable::new(&dir).unwrap();

        cache.insert("k", b"still readable").unwrap();
        let (file, _) = cache.get("k").unwrap();
        cache.remove("k");
        // the name is gone but the open handle still reads the whole file
        assert_eq!(read_all(file), b"still readable");
        // and the key can be cached again (new file, same name)
        cache.insert("k", b"again").unwrap();
        assert_eq!(read_all(cache.get("k").unwrap().0), b"again");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_new_clears_previous_run() {
        let dir = test_dir("clear");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("leftover"), b"old").unwrap();

        let cache = CacheTable::new(&dir).unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        cache.insert("k", b"x").unwrap();
        drop(cache);
        fs::remove_dir_all(&dir).unwrap();
    }
}
