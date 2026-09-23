//! Атомарная запись с правами 0600.
//!
//! Стейт, конфиг и credentials пишутся через tmp + fsync + rename + fsync каталога:
//! падение посреди записи оставляет либо старый файл, либо новый, но не половину.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(path);
    // Остаток от упавшего процесса с тем же pid мешал бы create_new.
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(parent_dir(path))?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

pub fn is_group_or_world_accessible(path: &Path) -> io::Result<bool> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o077 != 0)
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    parent_dir(path).join(format!(".{name}.tmp-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aplaut-fsutil-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_file_with_0600_and_leaves_no_tmp() {
        let dir = scratch("perm");
        let path = dir.join("state.json");
        write_atomic(&path, b"{}").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{}");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let leftovers: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(leftovers.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_write_keeps_old_file() {
        let dir = scratch("keep");
        let path = dir.join("state.json");
        write_atomic(&path, b"old").unwrap();
        // Каталог на месте tmp-файла: создать tmp нельзя, запись должна упасть до rename.
        fs::create_dir(tmp_path(&path)).unwrap();
        assert!(write_atomic(&path, b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn private_dir_is_0700_and_open_file_detected() {
        let dir = scratch("dir");
        let conf = dir.join("aplaut");
        ensure_private_dir(&conf).unwrap();
        assert_eq!(
            fs::metadata(&conf).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let file = conf.join("credentials");
        fs::write(&file, "x").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(is_group_or_world_accessible(&file).unwrap());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!is_group_or_world_accessible(&file).unwrap());
        fs::remove_dir_all(&dir).unwrap();
    }
}
