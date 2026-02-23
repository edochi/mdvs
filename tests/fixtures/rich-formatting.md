---
title: "Markdown Formatting Test"
tags: [test, formatting]
date: 2025-01-15
draft: true
---

# Rich Formatting

This note tests that **bold**, *italic*, `inline code`, and ~~strikethrough~~ are stripped properly during plain text extraction.

## Links and Images

Here's a [link to Rust docs](https://doc.rust-lang.org/) and an image reference: ![alt text](image.png)

Wikilinks like [[some-other-note]] are common in Obsidian vaults.

## Code Block

```python
def hello():
    print("Hello, world!")
```

The code above should be stripped or handled gracefully.

## Lists

1. First ordered item
2. Second ordered item
   - Nested unordered
   - Another nested

## Blockquote

> This is a blockquote that might appear in notes.
> It spans multiple lines.

## Table

| Header 1 | Header 2 |
|----------|----------|
| Cell A   | Cell B   |
| Cell C   | Cell D   |
