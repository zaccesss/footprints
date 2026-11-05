//! content-addressed blob store.
//!
//! each blob is named by the BLAKE3 hash of its uncompressed bytes and stored zstd compressed
//! under `blobs/<first two hex chars>/<rest>`. Identical content is stored once.

use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// zstd level 3 is the library default and keeps hook latency low; higher levels cost more
/// time than they save on typical source files.
const ZSTD_LEVEL: i32 = 3;

pub struct BlobStore {
    root: PathBuf,
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

impl BlobStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::store::private_dir(&root)
            .with_context(|| format!("creating blob directory {}", root.display()))?;
        Ok(Self { root })
    }

    fn path_for(&self, hash: &str) -> Result<PathBuf> {
        if hash.len() < 3 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("invalid blob hash {hash:?}");
        }
        Ok(self.root.join(&hash[..2]).join(&hash[2..]))
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.path_for(hash).map(|p| p.exists()).unwrap_or(false)
    }

    /// storing the same bytes twice writes nothing new, which is what keeps repeated
    /// snapshots of an unchanged tree cheap.
    pub fn put(&self, bytes: &[u8]) -> Result<String> {
        let hash = hash_bytes(bytes);
        let path = self.path_for(&hash)?;
        if path.exists() {
            return Ok(hash);
        }
        let dir = path.parent().expect("blob path has a parent");
        if !dir.exists() {
            crate::store::private_dir(dir)?;
        }
        let compressed = zstd::encode_all(bytes, ZSTD_LEVEL)?;
        // write to a unique temporary name then rename, so two hooks racing on the same
        // content never leave a half-written blob behind
        let tmp = dir.join(format!(
            ".tmp-{}-{}-{}",
            std::process::id(),
            &hash[2..10],
            nanos_now()
        ));
        {
            let mut f = private_file(&tmp)?;
            f.write_all(&compressed)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        Ok(hash)
    }

    pub fn put_file(&self, path: &Path) -> Result<String> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        self.put(&bytes)
    }

    pub fn get(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.path_for(hash)?;
        let compressed =
            fs::read(&path).with_context(|| format!("blob {hash} is missing from the store"))?;
        let bytes = zstd::decode_all(&compressed[..])?;
        if hash_bytes(&bytes) != hash {
            bail!("blob {hash} is corrupt: its contents no longer match its hash");
        }
        Ok(bytes)
    }

    /// used by tests to prove deduplication.
    pub fn count(&self) -> Result<usize> {
        let mut n = 0;
        for shard in fs::read_dir(&self.root)? {
            let shard = shard?;
            if shard.file_type()?.is_dir() {
                for entry in fs::read_dir(shard.path())? {
                    let name = entry?.file_name();
                    if !name.to_string_lossy().starts_with(".tmp-") {
                        n += 1;
                    }
                }
            }
        }
        Ok(n)
    }
}

/// blobs are copies of files that may be secret, so they are readable by the owner only
/// whatever the umask.
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

fn nanos_now() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path().join("blobs")).unwrap();
        let a = store.put(b"hello world\n").unwrap();
        let b = store.put(b"hello world\n").unwrap();
        let c = store.put(b"something else\n").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(store.count().unwrap(), 2);
        assert_eq!(store.get(&a).unwrap(), b"hello world\n");
        assert_eq!(store.get(&c).unwrap(), b"something else\n");
    }

    #[test]
    fn blobs_are_compressed() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path().join("blobs")).unwrap();
        let big = vec![b'a'; 100_000];
        let h = store.put(&big).unwrap();
        let on_disk = fs::metadata(store.path_for(&h).unwrap()).unwrap().len();
        assert!(on_disk < 1_000, "expected compression, got {on_disk} bytes");
        assert_eq!(store.get(&h).unwrap(), big);
    }

    #[test]
    fn corrupt_blob_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path().join("blobs")).unwrap();
        let h = store.put(b"original").unwrap();
        let p = store.path_for(&h).unwrap();
        fs::write(&p, zstd::encode_all(&b"tampered"[..], 3).unwrap()).unwrap();
        assert!(store.get(&h).is_err());
    }

    #[test]
    fn rejects_bad_hash() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path().join("blobs")).unwrap();
        assert!(store.get("../../etc/passwd").is_err());
    }
}
