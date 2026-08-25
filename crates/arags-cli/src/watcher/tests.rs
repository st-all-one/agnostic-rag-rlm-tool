#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn test_change_buffer_quiet_window() {
    let mut buf = ChangeBuffer::default();
    assert!(!buf.due());
    assert!(buf.is_empty());

    buf.extend(vec!["a.rs".into()]);
    assert!(!buf.due(), "freshly changed files wait the full window");
    assert!(!buf.is_empty());

    buf.take();
    assert!(buf.is_empty());
    assert!(!buf.due(), "deadline cleared after take");
}

#[test]
fn test_marker_paths_are_dotfiles_at_root() {
    let root = Path::new("/tmp/proj");
    assert_eq!(pid_path(root), root.join(".arags-watch.pid"));
    assert_eq!(stop_path(root), root.join(".arags-watch.stop"));
    for p in [pid_path(root), stop_path(root)] {
        assert!(
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.')),
            "{p:?} must be ignored by the dot-path rule"
        );
    }
}
