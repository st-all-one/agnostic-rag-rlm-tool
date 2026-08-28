#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn root_rule(line: &str) -> IgnoreRule {
    parse_line(line, Path::new(".")).expect("rule")
}

#[test]
fn test_comments_and_blanks_skipped() {
    assert!(parse_line("", Path::new(".")).is_none());
    assert!(parse_line("   ", Path::new(".")).is_none());
    assert!(parse_line("# comment", Path::new(".")).is_none());
}

#[test]
fn test_simple_name_matches_any_depth() {
    let r = root_rule("build.log");
    assert!(r.matches("build.log", false));
    assert!(r.matches("a/b/build.log", false));
    assert!(!r.matches("src/main.rs", false));
}

#[test]
fn test_dir_only_pattern() {
    let r = root_rule("logs/");
    assert!(r.matches("logs", true));
    assert!(r.matches("a/logs", true));
    assert!(!r.matches("logs", false), "dir-only must not match files");
}

#[test]
fn test_anchored_pattern() {
    let r = root_rule("/dist");
    assert!(r.matches("dist", true));
    assert!(r.matches("dist/x.js", false) || r.matches("dist", true));
    assert!(!r.matches("pkg/dist", true), "anchored to root only");
}

#[test]
fn test_glob_star_and_question() {
    let r = root_rule("*.tmp");
    assert!(r.matches("a.tmp", false));
    assert!(r.matches("deep/dir/b.tmp", false));
    assert!(!r.matches("a.txt", false));

    let q = root_rule("file?.rs");
    assert!(q.matches("file1.rs", false));
    assert!(!q.matches("file12.rs", false));
}

#[test]
fn test_double_star() {
    let r = root_rule("**/generated/**");
    assert!(r.matches("generated/x.rs", false));
    assert!(r.matches("a/b/generated/c.rs", false));
    assert!(!r.matches("a/b/src.rs", false));
}

#[test]
fn test_negation_last_wins() {
    let keep = root_rule("!keep.log");
    assert!(keep.negated);
    // The caller applies last-match-wins ordering across the rule list.
    let rules = [root_rule("*.log"), keep];
    let mut ignored = None;
    for rule in &rules {
        if rule.decides("keep.log", false) {
            ignored = Some(!rule.negated);
        }
    }
    assert_eq!(ignored, Some(false), "negation must win as last match");

    let mut ignored_other = None;
    for rule in &rules {
        if rule.decides("drop.log", false) {
            ignored_other = Some(!rule.negated);
        }
    }
    assert_eq!(ignored_other, Some(true));
}

#[test]
fn test_nested_gitignore_scope() {
    let base = Path::new("sub/pkg");
    let r = IgnoreRule {
        pattern: "cache".to_string(),
        base: base.to_path_buf(),
        dir_only: false,
        anchored: true,
        negated: false,
    };
    assert!(r.matches("sub/pkg/cache", true));
    assert!(r.matches("sub/pkg/cache/x", false));
    assert!(!r.matches("other/cache", true));
}

#[test]
fn test_glob_match_basics() {
    assert!(glob_match("*", "abc"));
    assert!(!glob_match("*", "a/b"));
    assert!(glob_match("a/*/c", "a/b/c"));
    assert!(!glob_match("a/*/c", "a/b/d/c"));
    assert!(glob_match("a/**/c", "a/x/y/c"));
    assert!(glob_match("**", "x/y/z"));
}

#[test]
fn nested_unanchored_star_never_leaks_outside_base() {
    // Regression (agnostic-rag-rlm-tool-4d4d): Laravel's bootstrap/cache/.gitignore
    // contains a bare `*`; it must only govern paths under bootstrap/cache/,
    // never wipe the whole project index.
    let star = parse_line("*", Path::new("bootstrap/cache")).expect("rule");
    assert!(!star.matches("index.php", false));
    assert!(!star.matches("app/Models/User.php", false));
    assert!(!star.matches("readme.md", false));
    assert!(star.matches("bootstrap/cache/views/a.php", false));

    let keep = parse_line("!.gitkeep", Path::new("bootstrap/cache")).expect("rule");
    assert!(keep.matches("bootstrap/cache/.gitkeep", false));
    assert!(
        !keep.matches(".gitkeep", false),
        "negation is also base-scoped"
    );
}

#[test]
fn nested_anchored_rule_scoped_to_base() {
    let r = parse_line("/junk", Path::new("storage/app")).expect("rule");
    assert!(r.matches("storage/app/junk", true));
    assert!(r.matches("storage/app/junk/inner.txt", false));
    assert!(!r.matches("junk", true), "same name at root: outside base");
    assert!(!r.matches("other/storage/app/junk", true));
}

#[test]
fn root_gitignore_still_governs_everything() {
    let r = parse_line("*.log", Path::new(".")).expect("rule");
    assert!(r.matches("a.log", false));
    assert!(r.matches("deep/nested/b.log", false));
}
