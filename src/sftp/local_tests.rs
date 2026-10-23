//! 本地目录读取与路径拼接的单元测试。

use std::path::{Path, PathBuf};

use super::{
    FileEntry, FileKind, FileSelection, SortDir, SortKey, SortState, format_bytes, format_eta,
    join_remote_path, local_parent, read_local_dir, remote_parent, sort_file_entries,
};

#[test]
fn reads_a_directory_with_directories_first() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(directory.path().join("a-folder")).expect("folder");
    std::fs::write(directory.path().join("b.txt"), b"hello").expect("file");

    let entries = read_local_dir(directory.path()).expect("listing");
    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["a-folder", "b.txt"]);
    assert!(entries[0].is_dir);
    assert_eq!(entries[0].size_kb, None);
    assert_eq!(entries[1].size_kb, Some(1));
}

#[test]
fn the_configured_folder_is_where_the_local_pane_starts() {
    let directory = tempfile::tempdir().expect("temp dir");
    let configured = directory.path().to_string_lossy().to_string();

    assert_eq!(
        super::starting_local_path(&configured),
        directory.path(),
        "设置里的路径能读就用它"
    );
    // 首尾空白来自手输，不当作路径的一部分。
    assert_eq!(
        super::starting_local_path(&format!("  {configured}  ")),
        directory.path()
    );
}

#[test]
fn a_blank_or_missing_folder_falls_back_to_the_home_dir() {
    let home = dirs::home_dir().expect("home dir");
    assert_eq!(super::starting_local_path(""), home, "留空用主目录");
    assert_eq!(
        super::starting_local_path(r"Z:\definitely\not\here"),
        home,
        "路径失效时不留在空目录里"
    );
}

#[test]
fn parents_are_relative_to_the_root() {
    assert_eq!(
        local_parent(Path::new(r"C:\Users\Qi")),
        Some(PathBuf::from(r"C:\Users"))
    );
    assert_eq!(local_parent(Path::new(r"C:\")), None);
    assert_eq!(remote_parent("/var/www"), Some("/var".into()));
    assert_eq!(remote_parent("/var"), Some("/".into()));
    assert_eq!(remote_parent("/"), None);
}

#[test]
fn joins_remote_paths_from_the_root() {
    assert_eq!(join_remote_path("/", "home"), "/home");
    assert_eq!(join_remote_path("/var/www/", "atlas"), "/var/www/atlas");
}

fn entry(name: &str, is_dir: bool, size_kb: Option<u64>, modified: &str) -> FileEntry {
    FileEntry {
        name: name.into(),
        kind: if is_dir {
            FileKind::Folder
        } else {
            FileKind::File
        },
        is_dir,
        size_kb,
        modified: modified.into(),
        permissions: String::new(),
        owner: String::new(),
        target: None,
    }
}

#[test]
fn sorting_keeps_directories_first_and_honours_direction() {
    let mut entries = vec![
        entry("small.txt", false, Some(1), "2026-01-01 10:00"),
        entry("big.txt", false, Some(9), "2026-01-03 10:00"),
        entry("folder-a", true, None, "2026-01-02 10:00"),
        entry("folder-b", true, None, "2026-01-01 10:00"),
    ];
    sort_file_entries(
        &mut entries,
        SortState {
            key: SortKey::Size,
            dir: SortDir::Desc,
            touched: true,
        },
    );
    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["folder-a", "folder-b", "big.txt", "small.txt"]);

    sort_file_entries(
        &mut entries,
        SortState {
            key: SortKey::Modified,
            dir: SortDir::Asc,
            touched: true,
        },
    );
    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["folder-b", "folder-a", "small.txt", "big.txt"]);
}

#[test]
fn file_selection_supports_single_toggle_and_range() {
    let mut selection = FileSelection::default();
    selection.set_single(2);
    assert_eq!(selection.count(), 1);
    assert!(selection.contains(2));

    selection.toggle(4);
    assert_eq!(selection.count(), 2);
    assert!(selection.contains(4));

    selection.toggle(2);
    assert_eq!(selection.count(), 1);
    assert!(!selection.contains(2));

    selection.set_single(1);
    selection.toggle(3);
    selection.select_range(5);
    assert_eq!(selection.count(), 3);
    for index in 3..=5 {
        assert!(selection.contains(index));
    }

    selection.clear();
    assert_eq!(selection.count(), 0);
}

#[test]
fn formats_transfer_sizes_and_eta() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(1024), "1.0 KB");
    assert_eq!(format_bytes(1536), "1.5 KB");
    assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
    assert_eq!(format_eta(Some(27)), "00:27");
    assert_eq!(format_eta(Some(3672)), "01:01:12");
    assert_eq!(format_eta(None), "—");
}

#[test]
fn file_sizes_step_up_to_megabytes_and_gigabytes() {
    let entry = |size_kb| FileEntry {
        name: "file".into(),
        kind: FileKind::File,
        is_dir: false,
        size_kb,
        modified: "-".into(),
        permissions: "-".into(),
        owner: "-".into(),
        target: None,
    };
    assert_eq!(entry(Some(512)).size_label(), "512 KB");
    assert_eq!(entry(Some(1024)).size_label(), "1.0 MB");
    assert_eq!(entry(Some(1536)).size_label(), "1.5 MB");
    assert_eq!(entry(Some(1024 * 1024)).size_label(), "1.0 GB");
    assert_eq!(entry(None).size_label(), "-");
}
