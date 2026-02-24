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

    eprintln!("All 11 tests passed.");
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
    // draft: only at root -> "**"/"**" (root leaf is "**")
    assert_eq!(r["draft"], fp(&["**"], &["**"]));
    // tags: only in blog -> "blog/**"/"blog/**"
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
        // blog/ - all have title, most have tags
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

    // title: present in ALL files everywhere
    assert_eq!(r["title"], fp(&["**"], &["**"]));

    // servings: only in recipes, all recipes have it
    assert_eq!(r["servings"], fp(&["recipes/**"], &["recipes/**"]));

    // doi: only in research, all research files have it
    assert_eq!(r["doi"], fp(&["research/**"], &["research/**"]));

    // draft: only in blog/drafts, all drafts have it
    assert_eq!(r["draft"], fp(&["blog/drafts/**"], &["blog/drafts/**"]));

    // tags: in blog(leaf+drafts), notes. Not in research/recipes -> doesn't reach root.
    // blog leaf: any has tags (2/3), all doesn't (post2 lacks date but has tags... wait let me recheck)
    // Actually: blog leaf: all = title (intersection of all 3 blog files), any = {title, tags, date}
    // blog/drafts: all = {title, draft}, any = {title, draft, tags}
    // blog merged: all = {title} ∩ {title,draft} = {title}, any = {title,tags,date} ∩ {title,draft,tags} = {title,tags}
    // notes: all = {title}, any = {title, tags}
    // research: all = {title, doi}, any = {title, doi, date}
    // recipes: all = {title, servings}, any = {title, servings}
    // root: all = {title}∩{title}∩{title,doi}∩{title,servings} = {title}
    //        any = {title,tags}∩{title,tags}∩{title,doi,date}∩{title,servings} = {title}
    // tags not in root.any -> no collapse at root
    // tags in blog.any \ blog.all -> collapse allowed at blog
    // tags in notes.any \ notes.all -> collapse allowed at notes
    // So: allowed = ["blog/**", "notes/**"], required = ? (where is tags in all?)
    // blog/drafts: tags in any but not in all (d1 lacks tags)... wait d2 has tags, d1 doesn't
    // So blog/drafts all={title,draft}, blog leaf all={title,tags}? No:
    // blog/post1={title,tags,date}, post2={title,tags}, post3={title,tags,date}
    // blog leaf: all = {title,tags,date} ∩ {title,tags} ∩ {title,tags,date} = {title,tags}
    // blog/drafts: d1={title,draft}, d2={title,draft,tags} -> all={title,draft}, any={title,draft,tags}
    // blog merged: all = {title,tags} ∩ {title,draft} = {title}, any = {title,tags,date} ∩ {title,draft,tags} = {title,tags}
    // So tags in blog.any = yes. tags in notes.any = yes. Not in research/recipes.
    // root.any = {title,tags} ∩ {title,tags} ∩ {title,doi,date} ∩ {title,servings} = {title}
    // tags not in root.any. In blog: tags in any\all -> collapse allowed at blog. In notes: tags in any\all -> collapse allowed.
    // Required: blog leaf has tags in leaf_all={title,tags}, so required initialized at blog/.
    // blog/drafts: tags NOT in leaf_all (d1 lacks tags). So required not initialized for drafts.
    // notes/idea1 lacks tags, so notes leaf_all = {title}. Required not initialized for notes.
    // At blog collapse: tags in blog.any\blog.all -> collapse only allowed. Required stays.
    // Final: allowed=["blog/**","notes/**"], required=["blog/**"] (from blog leaf_all)
    // Hmm wait, let me be more careful. The required at blog/ is from leaf_all = {title,tags}.
    // So required for tags = {blog/}. No other leaf has tags in leaf_all.
    // At blog collapse: tags in blog.any \ blog.all -> collapse ONLY allowed. Required untouched.
    // At root: tags not in root.any -> no collapse.
    // Final: allowed=["blog/**","notes/**"], required=["blog/**"]
    assert_eq!(
        r["tags"],
        fp(&["blog/**", "notes/**"], &["blog/**"])
    );

    // date: blog leaf any has it, research any has it.
    // blog leaf_all = {title,tags}, so date NOT in leaf_all. date IS in leaf_any.
    // research leaf_all = {title,doi}, date NOT in leaf_all. date IS in leaf_any.
    // blog/drafts: no date at all.
    // blog.any = {title,tags} (from merge). date not in blog.any? Let me recheck.
    // blog leaf any = {title,tags,date}. blog/drafts any = {title,draft,tags}.
    // blog.any = {title,tags,date} ∩ {title,draft,tags} = {title,tags}. So date NOT in blog.any.
    // So date stays at blog leaf level. At blog collapse, date not in blog.any -> no collapse.
    // notes: no date. research any has date. recipes: no date.
    // root.any = {title}. date not in root.any -> no collapse.
    // Final: allowed=["blog/**","research/**"], required=[]
    assert_eq!(
        r["date"],
        fp(&["blog/**", "research/**"], &[])
    );

    eprintln!("  ok: large_tree");
}
