mdvs *args:
    ./target/release/mdvs {{args}}

book:
    mdbook serve book/ --open

lint-ast:
    ast-grep scan

# Nightly toolchain for rustfmt — rustfmt.toml uses unstable options. Bump deliberately.
rustfmt_toolchain := `cat rustfmt-toolchain`

# Format Rust with the pinned nightly rustfmt
fmt:
    cargo +{{ rustfmt_toolchain }} fmt

# Check Rust formatting without rewriting (what CI runs)
fmt-check:
    cargo +{{ rustfmt_toolchain }} fmt --check

# Keep in sync with .pre-commit-config.yaml.
prettier := "npx --yes prettier@3.6.2"

# Hard-wrap markdown at 80 cols (defaults to every tracked .md)
fmt-md *args:
    {{ prettier }} --write {{ if args == "" { "'**/*.md'" } else { args } }}

# Report which markdown files prettier would rewrite, without touching them
check-md *args:
    {{ prettier }} --check {{ if args == "" { "'**/*.md'" } else { args } }}
