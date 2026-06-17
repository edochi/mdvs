mdvs *args:
    ./target/release/mdvs {{args}}

book:
    mdbook serve book/ --open

lint-ast:
    ast-grep scan
