mdvs *args:
    ./target/release/mdvs {{args}}

book:
    mdbook serve book/ --open

lint-ast:
    ast-grep scan

# Keep in sync with .pre-commit-config.yaml.
prettier := "npx --yes prettier@3.6.2"

# Hard-wrap markdown at 80 cols (defaults to every tracked .md)
fmt-md *args:
    {{ prettier }} --write {{ if args == "" { "'**/*.md'" } else { args } }}

# Report which markdown files prettier would rewrite, without touching them
check-md *args:
    {{ prettier }} --check {{ if args == "" { "'**/*.md'" } else { args } }}
