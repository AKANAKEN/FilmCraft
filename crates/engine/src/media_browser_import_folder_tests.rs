//! Tests of `file.importFolder` (#1805): drag / File ▸ Import Folder should accept a parent
//! folder, and keep files and folders working together.

use std::sync::Arc;

use serde_json::json;

use crate::media_browser::DirEntry;
use crate::{Services, Session};

#[derive(Default)]
struct FakeFs {
    dirs: Vec<String>,
    files: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl Clone for FakeFs {
    fn clone(&self) -> Self {
        Self { dirs: self.dirs.clone(), files: std::sync::Mutex::new(self.files.lock().unwrap().clone()) }
    }
}

impl FakeFs {
    fn with_shoot() -> Self {
        let wav = filmcraft_media::wav::write_wav16(&[0.0; 48], 1, 48_000);
        let fs =
            FakeFs { dirs: ["/", "/Shoot", "/Shoot/Day 1", "/Shoot/Day 1/Cam A", "/Shoot/Empty"].iter().map(|d| d.to_string()).collect(), ..FakeFs::default() };
        {
            let mut files = fs.files.lock().unwrap();
            for f in ["/Shoot/slate.wav", "/Shoot/Day 1/room.wav", "/Shoot/Day 1/Cam A/take1.wav", "/Shoot/Day 1/Cam A/take2.wav"] {
                files.insert(f.into(), wav.clone());
            }
        }
        fs
    }
}

impl Services for FakeFs {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        let files = self.files.lock().unwrap();
        files.get(path).cloned().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, path.to_string()))
    }
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        let mut files = self.files.lock().unwrap();
        files.insert(path.into(), data.to_vec());
        Ok(())
    }
    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        self.read_file(path).map(|b| b.len() as u64)
    }
    fn list_entries(&self, dir: &str) -> Option<std::io::Result<Vec<DirEntry>>> {
        if !self.dirs.iter().any(|d| *d == dir) {
            return Some(Err(std::io::Error::new(std::io::ErrorKind::NotFound, dir.to_string())));
        }
        let prefix = if dir == "/" { "/" } else { &format!("{dir}/") };
        let direct = |p: &str| p.strip_prefix(prefix).filter(|r| !r.is_empty() && !r.contains('/')).map(str::to_string);
        let mut v: Vec<DirEntry> =
            self.dirs.iter().filter_map(|d| direct(d)).map(|name| DirEntry { name, is_dir: true, size: None, modified: Some(0) }).collect();
        let files = self.files.lock().unwrap();
        for (p, b) in files.iter() {
            if let Some(name) = direct(p) {
                v.push(DirEntry { name, is_dir: false, size: Some(b.len() as u64), modified: Some(0) });
            }
        }
        Some(Ok(v))
    }
    fn list_dir(&self, dir: &str) -> Option<std::io::Result<Vec<String>>> {
        self.list_entries(dir).map(|r| r.map(|v| v.into_iter().map(|e| e.name).collect()))
    }
    fn volumes(&self) -> Vec<crate::media_browser::Volume> {
        Vec::new()
    }
    fn home_dir(&self) -> Option<String> {
        Some("/home".into())
    }
}

fn session_with_shoot() -> Session {
    Session::new(Arc::new(FakeFs::with_shoot()))
}

fn folder_names(s: &Session, parent: Option<u64>) -> Vec<String> {
    let mut names: Vec<String> = match parent {
        Some(id) => {
            let b = s.project.root.find_bin(filmcraft_project::BinId(id)).unwrap();
            b.children.iter().filter_map(|c| if let filmcraft_project::BinEntry::Bin(b) = c { Some(b.name.clone()) } else { None }).collect()
        }
        None => s.project.root.children.iter().filter_map(|c| if let filmcraft_project::BinEntry::Bin(b) = c { Some(b.name.clone()) } else { None }).collect(),
    };
    names.sort();
    names
}

#[test]
fn file_import_folder_mirrors_sub_folders_as_bins() {
    let mut s = session_with_shoot();
    let r = s.execute("file.importFolder", json!({"paths": ["/Shoot"]})).unwrap();
    assert_eq!(r["bins"].as_array().unwrap().len(), 3, "{r}");
    let shoot = s.project.root.children.iter().find_map(|c| if let filmcraft_project::BinEntry::Bin(b) = c { Some(b.clone()) } else { None }).unwrap();
    let shoot_sub_names: Vec<String> = shoot
        .children
        .iter()
        .map(|c| match c {
            filmcraft_project::BinEntry::Bin(b) => format!("bin {}", s.project.root.find_bin(b.id).unwrap().name),
            filmcraft_project::BinEntry::Item(i) => s.project.item(*i).unwrap().name.clone(),
        })
        .collect();
    assert_eq!(shoot_sub_names.join(","), "slate.wav,bin Day 1");
    let day = shoot.children.iter().find_map(|c| if let filmcraft_project::BinEntry::Bin(b) = c { Some(b.clone()) } else { None }).unwrap();
    let day_names: Vec<String> = day
        .children
        .iter()
        .map(|c| match c {
            filmcraft_project::BinEntry::Bin(b) => format!("bin {}", b.name),
            filmcraft_project::BinEntry::Item(i) => s.project.item(*i).unwrap().name.clone(),
        })
        .collect();
    assert_eq!(day_names.join(","), "room.wav,bin Cam A");
}

#[test]
fn file_import_folder_accepts_mixed_paths() {
    let mut fs = FakeFs::with_shoot();
    {
        let files = fs.files.lock().unwrap();
        let wav = files.get("/Shoot/slate.wav").unwrap().clone();
        drop(files);
        fs.files.lock().unwrap().insert("/standalone.wav".into(), wav);
    }
    fs.dirs.push("/standalone".into());
    let mut s = Session::new(Arc::new(fs.clone()));
    let r = s.execute("file.importFolder", json!({"paths": ["/Shoot", "/standalone.wav"]})).unwrap();
    assert_eq!(r["items"].as_array().unwrap().len(), 5, "{r}");
    assert_eq!(folder_names(&s, None), ["Shoot"]);
}

#[test]
fn file_import_folder_passes_files_through() {
    let mut s = session_with_shoot();
    let r = s.execute("file.importFolder", json!({"paths": ["/Shoot/slate.wav"]})).unwrap();
    assert_eq!(r["items"].as_array().unwrap().len(), 1, "{r}");
    assert!(r["bins"].is_null(), "{r}");
    assert_eq!(s.project.items.len(), 1);
}

#[test]
fn file_import_folder_requires_path() {
    let mut s = session_with_shoot();
    assert!(s.execute("file.importFolder", json!({})).is_err());
    assert!(s.project.items.is_empty());
}

#[test]
fn file_import_folder_reports_a_folder_without_media() {
    let mut fs = FakeFs::with_shoot();
    fs.dirs.push("/NoMedia".into());
    let mut s = Session::new(Arc::new(fs));
    let e = s.execute("file.importFolder", json!({"paths": ["/NoMedia"]}));
    let msg = e.err().map(|x| x.to_string()).unwrap_or_default();
    assert!(msg.contains("no media files in /NoMedia"), "error: {msg}");
    assert!(s.project.items.is_empty());
    assert!(s.project.root.children.is_empty(), "{:?}", s.project.root.children);
}

#[test]
fn file_import_folder_reports_a_missing_path() {
    let mut s = session_with_shoot();
    assert!(s.execute("file.importFolder", json!({"paths": ["/Nope"]})).is_err());
    assert!(s.project.items.is_empty());
}

#[test]
fn file_import_folder_puts_imports_in_a_target_bin() {
    let mut s = session_with_shoot();
    let bin = s.execute("file.newBin", json!({"name": "Target"})).unwrap()["bin"].as_u64().unwrap();
    let r = s.execute("file.importFolder", json!({"paths": ["/Shoot/slate.wav"], "bin": bin})).unwrap();
    assert_eq!(r["items"].as_array().unwrap().len(), 1, "{r}");
    let target = s.project.root.find_bin(filmcraft_project::BinId(bin)).unwrap();
    let items: Vec<_> = target.children.iter().filter_map(|c| if let filmcraft_project::BinEntry::Item(i) = c { Some(*i) } else { None }).collect();
    assert_eq!(items.len(), 1);
}
