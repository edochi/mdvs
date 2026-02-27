#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! gray_matter = "0.2"
//! serde_json = "1"
//! walkdir = "2"
//! globset = "0.4"
//! tempfile = "3"
//! ```

use globset::Glob;
use gray_matter::engine::YAML;
use gray_matter::Matter;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// --- Data structures ---

#[derive(Debug)]
struct ScannedFile {
    path: PathBuf,
    data: Option<Value>,
    content: String,
}

#[derive(Debug)]
struct ScannedFiles {
    files: Vec<ScannedFile>,
}

impl ScannedFiles {
    fn scan(root: &Path, glob: &str, include_bare_files: bool) -> Self {
        let matcher = Glob::new(glob)
            .expect("invalid glob pattern")
            .compile_matcher();
        let matter = Matter::<YAML>::new();

        let mut files = Vec::new();

        for entry in WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "md" || ext == "markdown")
            })
        {
            let abs_path = entry.path();
            let rel_path = abs_path.strip_prefix(root).unwrap().to_path_buf();

            if !matcher.is_match(&rel_path) {
                continue;
            }

            let raw = fs::read_to_string(abs_path).expect("failed to read file");
            let parsed = matter.parse(&raw);

            let data = parsed.data.and_then(|d| {
                let json: Value = d.deserialize().ok()?;
                if json.is_object() { Some(json) } else { None }
            });

            if data.is_none() && !include_bare_files {
                continue;
            }

            let content = parsed.content.trim().to_string();

            files.push(ScannedFile {
                path: rel_path,
                data,
                content,
            });
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));

        ScannedFiles { files }
    }
}

// --- Test helpers ---

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let full = root.join(rel_path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

fn main() {
    println!("=== ScannedFiles tests ===\n");

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Files with YAML frontmatter
    write_file(root, "blog/post1.md", "---\ntitle: Hello\ntags:\n  - rust\n  - arrow\n---\nThis is post 1.");
    write_file(root, "blog/post2.md", "---\ntitle: World\ndraft: true\n---\nThis is post 2.");
    write_file(root, "blog/drafts/d1.md", "---\ntitle: Draft\ncount: 42\n---\nDraft content.");
    write_file(root, "notes/idea.md", "---\ntitle: Idea\n---\nSome idea.");

    // File without frontmatter
    write_file(root, "notes/bare.md", "Just plain markdown, no frontmatter.");

    // Non-markdown file (should be ignored)
    write_file(root, "blog/image.png", "not real png data");

    // --- Test 1: Basic scan, exclude bare files ---
    let scanned = ScannedFiles::scan(root, "**", false);
    assert_eq!(scanned.files.len(), 4, "should find 4 files with frontmatter");
    println!("  Scan ** exclude bare: {} files  ✓", scanned.files.len());

    // Check paths are relative and sorted
    let paths: Vec<&str> = scanned.files.iter().map(|f| f.path.to_str().unwrap()).collect();
    assert_eq!(paths, vec![
        "blog/drafts/d1.md",
        "blog/post1.md",
        "blog/post2.md",
        "notes/idea.md",
    ]);
    println!("  Paths are relative and sorted  ✓");

    // Check frontmatter parsed correctly
    let post1 = scanned.files.iter().find(|f| f.path.to_str().unwrap() == "blog/post1.md").unwrap();
    let post1_data = post1.data.as_ref().unwrap();
    assert_eq!(post1_data["title"], "Hello");
    assert_eq!(post1_data["tags"][0], "rust");
    assert_eq!(post1_data["tags"][1], "arrow");
    println!("  Frontmatter parsed: title, tags  ✓");

    // Check content is trimmed and excludes frontmatter
    assert_eq!(post1.content, "This is post 1.");
    println!("  Content trimmed, frontmatter excluded  ✓");

    // Check boolean field
    let post2 = scanned.files.iter().find(|f| f.path.to_str().unwrap() == "blog/post2.md").unwrap();
    assert_eq!(post2.data.as_ref().unwrap()["draft"], true);
    println!("  Boolean field preserved  ✓");

    // Check integer field
    let d1 = scanned.files.iter().find(|f| f.path.to_str().unwrap() == "blog/drafts/d1.md").unwrap();
    assert_eq!(d1.data.as_ref().unwrap()["count"], 42);
    println!("  Integer field preserved  ✓");

    // --- Test 2: Include bare files ---
    let scanned_with_bare = ScannedFiles::scan(root, "**", true);
    assert_eq!(scanned_with_bare.files.len(), 5, "should find 5 files including bare");
    let bare = scanned_with_bare.files.iter().find(|f| f.path.to_str().unwrap() == "notes/bare.md").unwrap();
    assert!(bare.data.is_none());
    assert_eq!(bare.content, "Just plain markdown, no frontmatter.");
    println!("  Include bare files: {} files, bare.data=None  ✓", scanned_with_bare.files.len());

    // --- Test 3: Glob filtering ---
    let scanned_blog = ScannedFiles::scan(root, "blog/**", false);
    assert_eq!(scanned_blog.files.len(), 3, "should find 3 blog files");
    for f in &scanned_blog.files {
        assert!(f.path.starts_with("blog/"), "all paths should be under blog/");
    }
    println!("  Glob blog/**: {} files  ✓", scanned_blog.files.len());

    let scanned_notes = ScannedFiles::scan(root, "notes/**", false);
    assert_eq!(scanned_notes.files.len(), 1, "should find 1 notes file with frontmatter");
    assert_eq!(scanned_notes.files[0].path.to_str().unwrap(), "notes/idea.md");
    println!("  Glob notes/**: {} files  ✓", scanned_notes.files.len());

    // --- Test 4: Glob that matches nothing ---
    let scanned_empty = ScannedFiles::scan(root, "papers/**", false);
    assert_eq!(scanned_empty.files.len(), 0);
    println!("  Glob papers/**: 0 files  ✓");

    // --- Test 5: Non-object frontmatter rejected ---
    write_file(root, "weird/scalar.md", "---\njust a string\n---\nBody.");
    let scanned_weird = ScannedFiles::scan(root, "weird/**", true);
    assert_eq!(scanned_weird.files.len(), 1);
    assert!(scanned_weird.files[0].data.is_none(), "scalar frontmatter should be None");
    println!("  Non-object YAML frontmatter → data=None  ✓");

    // --- Test 6: Nested object in frontmatter ---
    write_file(root, "deep/nested.md", "---\ntitle: Nested\nmeta:\n  author: me\n  version: 2\n---\nNested content.");
    let scanned_deep = ScannedFiles::scan(root, "deep/**", false);
    let nested = &scanned_deep.files[0];
    let meta = &nested.data.as_ref().unwrap()["meta"];
    assert_eq!(meta["author"], "me");
    assert_eq!(meta["version"], 2);
    println!("  Nested object in frontmatter preserved  ✓");

    // --- Test 7: Empty frontmatter ---
    write_file(root, "empty/empty_fm.md", "---\n---\nBody only.");
    let scanned_empty_fm = ScannedFiles::scan(root, "empty/**", true);
    assert_eq!(scanned_empty_fm.files.len(), 1);
    // gray_matter may return None or empty object for empty frontmatter
    let ef = &scanned_empty_fm.files[0];
    if let Some(data) = &ef.data {
        assert!(data.as_object().unwrap().is_empty(), "empty frontmatter should be empty object");
        println!("  Empty frontmatter → empty object  ✓");
    } else {
        println!("  Empty frontmatter → None  ✓");
    }

    // --- Test 8: Multiple files in same dir, content check ---
    assert_eq!(
        scanned.files.iter().find(|f| f.path.to_str().unwrap() == "blog/drafts/d1.md").unwrap().content,
        "Draft content."
    );
    println!("  Content correctly extracted across dirs  ✓");

    println!("\n=== All tests passed ===");
}
