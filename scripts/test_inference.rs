#!/usr/bin/env rust-script
//! Integration tests for the tree inference algorithm.
//!
//! Run: `rust-script --base-path /path/to/mdvs scripts/test_inference.rs`
//!
//! ```cargo
//! [dependencies.mdvs-schema]
//! path = "../crates/mdvs-schema"
//! ```

use std::collections::HashSet;
use std::path::PathBuf;

use mdvs_schema::{FieldPaths, infer_field_paths};

fn obs(path: &str, fields: &[&str]) -> (PathBuf, HashSet<String>) {
    (
        PathBuf::from(path),
        fields.iter().map(|s| s.to_string()).collect(),
    )
}

fn fp(allowed: &[&str], required: &[&str]) -> FieldPaths {
    FieldPaths {
        allowed: allowed.iter().map(|s| s.to_string()).collect(),
        required: required.iter().map(|s| s.to_string()).collect(),
    }
}

fn main() {
    test_empty();
    test_single_file();
    test_root_only_partial();
    test_single_dir();
    test_two_dirs();
    test_deep_nesting();
    test_worked_example();
    test_mixed_root_and_subdir();
    test_many_dirs_shared_field();
    test_single_file_deep();
    test_large_tree();
    test_leaf_next_to_subdirectory();
    test_root_files_with_deep_subdirectory();

    eprintln!("All 13 tests passed.");
}

fn test_empty() {
    let r = infer_field_paths(&[]);
    assert!(r.is_empty(), "empty input -> empty map");
    eprintln!("  ok: empty");
}

fn test_single_file() {
    let r = infer_field_paths(&[obs("a.md", &["title", "tags"])]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    assert_eq!(r["tags"], fp(&["**"], &["**"]));
    eprintln!("  ok: single_file");
}

fn test_root_only_partial() {
    let r = infer_field_paths(&[
        obs("a.md", &["title", "tags"]),
        obs("b.md", &["title"]),
        obs("c.md", &["title", "date"]),
    ]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    assert_eq!(r["tags"], fp(&["**"], &[]));
    assert_eq!(r["date"], fp(&["**"], &[]));
    eprintln!("  ok: root_only_partial");
}

fn test_single_dir() {
    let r = infer_field_paths(&[
        obs("blog/a.md", &["title", "tags"]),
        obs("blog/b.md", &["title"]),
    ]);
    // Single child of root -> collapses to "**"
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    assert_eq!(r["tags"], fp(&["**"], &[]));
    eprintln!("  ok: single_dir");
}

fn test_two_dirs() {
    let r = infer_field_paths(&[
        obs("blog/a.md", &["title", "tags", "date"]),
        obs("blog/b.md", &["title", "date"]),
        obs("papers/x.md", &["title", "doi"]),
        obs("papers/y.md", &["title", "doi", "date"]),
    ]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    assert_eq!(r["date"], fp(&["**"], &["blog/**"]));
    assert_eq!(r["tags"], fp(&["blog/**"], &[]));
    assert_eq!(r["doi"], fp(&["papers/**"], &["papers/**"]));
    eprintln!("  ok: two_dirs");
}

fn test_deep_nesting() {
    let r = infer_field_paths(&[
        obs("blog/posts/a.md", &["title", "tags"]),
        obs("blog/posts/b.md", &["title", "tags"]),
        obs("blog/drafts/c.md", &["title", "draft"]),
        obs("papers/x.md", &["title", "doi"]),
    ]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    assert_eq!(r["tags"], fp(&["blog/posts/**"], &["blog/posts/**"]));
    assert_eq!(r["draft"], fp(&["blog/drafts/**"], &["blog/drafts/**"]));
    assert_eq!(r["doi"], fp(&["papers/**"], &["papers/**"]));
    eprintln!("  ok: deep_nesting");
}

fn test_worked_example() {
    // From inference.md design doc
    let r = infer_field_paths(&[
        obs("blog/post1.md", &["title", "tags"]),
        obs("blog/post2.md", &["title"]),
        obs("blog/drafts/d1.md", &["title", "tags"]),
        obs("blog/drafts/d2.md", &["title", "tags"]),
        obs("notes/idea1.md", &["title", "tags"]),
        obs("notes/idea2.md", &["title", "tags"]),
        obs("papers/paper1.md", &["title"]),
    ]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    assert_eq!(
        r["tags"],
        fp(&["blog/**", "notes/**"], &["blog/drafts/**", "notes/**"])
    );
    eprintln!("  ok: worked_example");
}

fn test_mixed_root_and_subdir() {
    let r = infer_field_paths(&[
        obs("a.md", &["title", "draft"]),
        obs("blog/b.md", &["title", "tags"]),
    ]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    // draft: only at root leaf, not in root.any → stays as * (shallow)
    assert_eq!(r["draft"], fp(&["*"], &[]));
    // tags: only in blog, collapses there
    assert_eq!(r["tags"], fp(&["blog/**"], &["blog/**"]));
    eprintln!("  ok: mixed_root_and_subdir");
}

fn test_many_dirs_shared_field() {
    let r = infer_field_paths(&[
        obs("a/x.md", &["title", "extra_a"]),
        obs("b/y.md", &["title", "extra_b"]),
        obs("c/z.md", &["title", "extra_c"]),
    ]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    assert_eq!(r["extra_a"], fp(&["a/**"], &["a/**"]));
    assert_eq!(r["extra_b"], fp(&["b/**"], &["b/**"]));
    assert_eq!(r["extra_c"], fp(&["c/**"], &["c/**"]));
    eprintln!("  ok: many_dirs_shared_field");
}

fn test_single_file_deep() {
    let r = infer_field_paths(&[obs("a/b/c/d.md", &["title"])]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    eprintln!("  ok: single_file_deep");
}

fn test_large_tree() {
    // Simulate a realistic vault: 5 top-level dirs, mixed field coverage
    let observations: Vec<(PathBuf, HashSet<String>)> = vec![
        // blog/ - all have title, all have tags
        obs("blog/post1.md", &["title", "tags", "date"]),
        obs("blog/post2.md", &["title", "tags"]),
        obs("blog/post3.md", &["title", "tags", "date"]),
        // blog/drafts/ - all have title and draft
        obs("blog/drafts/d1.md", &["title", "draft"]),
        obs("blog/drafts/d2.md", &["title", "draft", "tags"]),
        // notes/ - all have title
        obs("notes/idea1.md", &["title"]),
        obs("notes/idea2.md", &["title", "tags"]),
        // research/ - all have title and doi
        obs("research/paper1.md", &["title", "doi", "date"]),
        obs("research/paper2.md", &["title", "doi"]),
        // recipes/ - all have title, none share other fields with above
        obs("recipes/cake.md", &["title", "servings"]),
        obs("recipes/bread.md", &["title", "servings"]),
    ];
    let r = infer_field_paths(&observations);

    assert_eq!(r["title"], fp(&["**"], &["**"]));
    assert_eq!(r["servings"], fp(&["recipes/**"], &["recipes/**"]));
    assert_eq!(r["doi"], fp(&["research/**"], &["research/**"]));
    assert_eq!(r["draft"], fp(&["blog/drafts/**"], &["blog/drafts/**"]));

    // tags: blog leaf has it in all, blog/drafts has it in any\all.
    // blog.any has tags → collapse allowed at blog → Recursive.
    // notes.any has tags → collapse allowed at notes → Recursive.
    // root.any = {title} → no collapse.
    // required: no directory has tags in all (blog.all={title}, notes.all={title}).
    assert_eq!(
        r["tags"],
        fp(&["blog/**", "notes/**"], &[])
    );

    // date: blog leaf has it in any\all. research leaf has it in any\all.
    // blog has two children (leaf + drafts), blog.any = {title,tags} — date NOT in blog.any.
    // So blog leaf keeps Shallow → "blog/*".
    // research has one child (leaf), research.any = {title,doi,date} — date in any\all.
    // Collapse allowed at research → Recursive → "research/**".
    assert_eq!(
        r["date"],
        fp(&["blog/*", "research/**"], &[])
    );

    eprintln!("  ok: large_tree");
}

fn test_leaf_next_to_subdirectory() {
    // file4.md sits directly in a/b/ alongside subdirectory c/.
    // deep is in the leaf but c/ doesn't have it everywhere → no collapse at b/.
    let r = infer_field_paths(&[
        obs("a/b/file4.md", &["title", "deep"]),
        obs("a/b/c/file3.md", &["title"]),
        obs("a/b/c/d/file1.md", &["title", "deep"]),
        obs("a/b/c/d/file2.md", &["title", "deep"]),
        obs("x/file5.md", &["title"]),
    ]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    // a/b/* (leaf, shallow) + a/b/c/d/** (collapsed)
    assert_eq!(
        r["deep"],
        fp(&["a/b/*", "a/b/c/d/**"], &["a/b/c/d/**"])
    );
    eprintln!("  ok: leaf_next_to_subdirectory");
}

fn test_root_files_with_deep_subdirectory() {
    // file6.md at root + a/b/c/d/ has deep, but intermediate dirs don't.
    let r = infer_field_paths(&[
        obs("file6.md", &["title", "deep"]),
        obs("a/b/c/d/file1.md", &["title", "deep"]),
        obs("a/b/c/d/file2.md", &["title", "deep"]),
        obs("a/b/c/file3.md", &["title"]),
        obs("a/b/file4.md", &["title"]),
        obs("x/file5.md", &["title"]),
    ]);
    assert_eq!(r["title"], fp(&["**"], &["**"]));
    // * (root leaf, shallow) + a/b/c/d/** (collapsed)
    assert_eq!(
        r["deep"],
        fp(&["*", "a/b/c/d/**"], &["a/b/c/d/**"])
    );
    eprintln!("  ok: root_files_with_deep_subdirectory");
}
