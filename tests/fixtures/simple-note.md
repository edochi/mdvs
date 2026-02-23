---
title: "Simple Note"
tags: [rust, testing]
date: 2025-06-12
---

# Simple Note

This is a straightforward note with basic frontmatter. It has a single heading and a couple of paragraphs.

Rust's ownership model prevents data races at compile time. The borrow checker ensures that references are always valid, eliminating use-after-free bugs without a garbage collector.

## Key Concepts

- Ownership: each value has exactly one owner
- Borrowing: references allow temporary access without taking ownership
- Lifetimes: the compiler tracks how long references are valid
