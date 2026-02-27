# mdvs Crate Implementation Plan

## Overview

Single crate, single binary. All prototypes from `scripts/` are validated and ready to become real modules.

Pipeline: **discover** → **schema** → **index** → **search**

```
crates/mdvs/src/
├── main.rs                # CLI (clap): init, build, search, check, update, clean, info
├── lib.rs                 # pub mod declarations
├── discover/              # "what's in my markdown files?"
│   ├── mod.rs
│   ├── scan.rs            # walk files, parse frontmatter → ScannedFiles
│   ├── field_type.rs      # FieldType enum, widen, From<Value>, Into<DataType>
│   └── infer.rs           # DirectoryTree, InferredSchema, GlobMap
├── schema/                # "what's the contract?"
│   ├── mod.rs
│   ├── shared.rs          # FieldDef, FieldTypeSerde, types shared by config + lock
│   ├── config.rs          # MdvsToml — read/write mdvs.toml
│   └── lock.rs            # MdvsLock — read/write mdvs.lock
├── index/                 # "build the searchable thing"
│   ├── mod.rs
│   ├── chunk.rs           # Chunks, extract_plain_text, strip_wikilinks
│   ├── embed.rs           # ModelConfig, Embedder, resolve_revision
│   └── storage.rs         # Parquet read/write, Arrow builders (build_array, build_files_batch, build_chunks_batch)
├── search.rs              # CosineSimilarityUDF, query building
└── cmd/                   # CLI commands (thin wiring, delegates to library modules)
    ├── mod.rs
    ├── init.rs            # scan → infer → write config/lock → build parquet
    ├── build.rs           # validate → write parquet (incremental via content hash)
    ├── search.rs          # load parquet → register UDF → execute SQL → format output
    ├── check.rs           # validate frontmatter against schema
    ├── update.rs          # re-scan → refresh lock
    ├── clean.rs           # rm -rf .mdvs/
    └── info.rs            # show index stats
```

---

## Cargo.toml

```toml
[package]
name = "mdvs"
version = "0.3.0"
edition = "2021"

[dependencies]
# CLI
clap = { version = "4", features = ["derive"] }

# Async (required by DataFusion)
tokio = { version = "1", features = ["full"] }

# Query engine (re-exports arrow 57 + parquet 57)
datafusion = "52"

# Frontmatter
gray_matter = "0.2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Chunking + text
text-splitter = { version = "0.18", features = ["markdown"] }
pulldown-cmark = "0.12"
regex = "1"

# Embedding
model2vec-rs = "0.1.4"

# Inference (tree algorithm)
indextree = "4"

# File walking + glob
walkdir = "2"
globset = "0.4"

# IDs + hashing
uuid = { version = "1", features = ["v4"] }
chrono = "0.4"
```

**No explicit `arrow` or `parquet` deps** — use `datafusion::arrow` and `datafusion::parquet` re-exports to avoid version mismatch.

---

## Module Details

### `discover/field_type.rs`

From: `test_widening.rs` + `test_arrow.rs`

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum FieldType {
    Boolean,
    Integer,
    Float,
    String,
    Array(Box<FieldType>),
    Object(BTreeMap<String, FieldType>),
}

impl From<&Value> for FieldType
    // Recursive: Value::Object → Object, Value::Array → Array(widen elements), etc.

pub fn widen(a: FieldType, b: FieldType) -> FieldType
    // Symmetric widening per the matrix in MEMORY.md

impl Into<DataType> for &FieldType
    // Boolean→Boolean, Integer→Int64, Float→Float64, String→Utf8,
    // Array(T)→List<T>, Object→Struct(...)
    // (Into not From — orphan rule)
```

### `discover/scan.rs`

From: `test_scan.rs`

```rust
#[derive(Debug)]
pub struct ScannedFile {
    pub path: PathBuf,         // relative to scan root
    pub data: Option<Value>,   // parsed YAML frontmatter (None if missing/invalid)
    pub content: String,       // body after ---, trimmed
}

#[derive(Debug)]
pub struct ScannedFiles {
    pub files: Vec<ScannedFile>,
}

impl ScannedFiles {
    pub fn scan(root: &Path, glob: &str, include_bare_files: bool) -> Self
        // walkdir + globset filter, gray_matter extraction
        // only .md/.markdown, sorted by path
}
```

### `discover/infer.rs`

From: `test_inference.rs`

```rust
pub struct FieldTypeInfo {
    pub field_type: FieldType,
    pub files: Vec<PathBuf>,
}

pub struct FieldPaths {
    pub allowed: Vec<String>,
    pub required: Vec<String>,
}

pub struct InferredField {
    pub name: String,
    pub field_type: FieldType,
    pub files: Vec<PathBuf>,
    pub allowed: Vec<String>,
    pub required: Vec<String>,
}

pub struct InferredSchema {
    pub fields: Vec<InferredField>,
}

// --- Tree types (internal) ---

struct NodeData { path: PathBuf, all: HashSet<String>, any: HashSet<String> }
pub struct DirectoryTree { arena: Arena<NodeData>, root: NodeId }

#[derive(Clone, Copy, PartialEq, Eq)]
enum GlobDepth { Shallow, Recursive }

struct GlobMap { entries: HashMap<PathBuf, GlobDepth> }

// --- Public API ---

pub fn infer_field_types(scanned: &ScannedFiles) -> BTreeMap<String, FieldTypeInfo>
    // Flat pass: for each field, widen across all files

impl From<&ScannedFiles> for DirectoryTree
    // Build tree from file paths, populate all/any sets

impl DirectoryTree {
    pub fn merge(&mut self)        // bottom-up merge of field sets
    pub fn infer_paths(&self) -> BTreeMap<String, FieldPaths>
}

impl GlobMap {
    fn new() -> Self
    fn insert_shallow(&mut self, path: PathBuf)
    fn collapse(&mut self, ancestor_path: &Path)
    fn to_globs(&self) -> Vec<String>
}

fn ensure_dir(...) -> NodeId       // free function
fn intersect_all(...) -> HashSet   // free function

impl InferredSchema {
    pub fn infer(scanned: &ScannedFiles) -> Self
        // Orchestrates both passes, joins by field name
    pub fn field(&self, name: &str) -> Option<&InferredField>
}
```

### `schema/shared.rs`

From: `test_toml.rs`

```rust
/// TOML-serializable representation of FieldType.
/// Scalar("string"), Array { array }, Object { object }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FieldTypeSerde {
    Scalar(String),
    Array { array: Box<FieldTypeSerde> },
    Object { object: BTreeMap<String, FieldTypeSerde> },
}

impl From<&FieldType> for FieldTypeSerde
impl TryFrom<&FieldTypeSerde> for FieldType   // Error = String

/// Config for directory scanning — shared by toml and lock.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TomlConfig {
    pub glob: String,
    pub include_bare_files: bool,
}
```

### `schema/config.rs`

From: `test_toml.rs`

```rust
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TomlField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldTypeSerde,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MdvsToml {
    pub config: TomlConfig,
    pub fields: Vec<TomlField>,
}

impl MdvsToml {
    pub fn from_inferred(schema: &InferredSchema, config: &TomlConfig) -> Self
    pub fn read(path: &Path) -> Result<Self>
    pub fn write(&self, path: &Path) -> Result<()>
}
```

### `schema/lock.rs`

From: `test_toml.rs`

```rust
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct LockFile {
    pub path: String,
    pub content_hash: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct LockField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldTypeSerde,
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MdvsLock {
    pub config: TomlConfig,
    pub files: Vec<LockFile>,
    pub fields: Vec<LockField>,
}

impl MdvsLock {
    pub fn from_inferred(schema: &InferredSchema, scanned: &ScannedFiles, config: &TomlConfig) -> Self
    pub fn read(path: &Path) -> Result<Self>
    pub fn write(&self, path: &Path) -> Result<()>
}
```

### `index/chunk.rs`

From: `test_chunk.rs`

```rust
#[derive(Debug, Clone)]
pub struct Chunk {
    pub chunk_index: usize,
    pub start_line: usize,   // 1-based
    pub end_line: usize,     // 1-based
    pub plain_text: String,  // for embedding (not stored in Parquet)
}

pub struct Chunks(Vec<Chunk>);

impl Deref for Chunks {
    type Target = Vec<Chunk>;
}

impl Chunks {
    pub fn new(body: &str, max_chars: usize) -> Self
        // MarkdownSplitter → subslices → byte offset → line numbers → plain text
}

fn chunk_byte_offset(body: &str, chunk: &str) -> usize       // internal
fn byte_offset_to_line(line_starts: &[usize], byte: usize) -> usize  // internal

pub fn extract_plain_text(markdown: &str) -> String
    // pulldown-cmark Event::Text extraction → strip_wikilinks

pub fn strip_wikilinks(text: &str) -> String
    // regex: !?\[\[([^\]|]+)(?:\|([^\]]+))?\]\]
    // [[target]] → target, [[target|display]] → display
    // ![[embed]] → embed, ![[embed|alt]] → alt
```

### `index/embed.rs`

From: `test_embed.rs`

```rust
#[derive(Debug, Clone)]
pub enum ModelConfig {
    Model2Vec {
        model_id: String,
        revision: Option<String>,   // optional in toml, always present in lock
    },
}

pub enum Embedder {
    Model2Vec(StaticModel),
}

impl Embedder {
    pub fn load(config: &ModelConfig) -> Self
    pub fn dimension(&self) -> usize         // probe with encode_single("probe").len()
    pub fn embed(&self, text: &str) -> Vec<f32>
    pub fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>>
}

pub fn resolve_revision(model_id: &str) -> Option<String>
    // Reads ~/.cache/huggingface/hub/models--org--name/snapshots/<sha>/
```

### `index/storage.rs`

From: `test_arrow.rs` + `test_parquet.rs`

```rust
// --- Arrow builders ---

pub fn build_array(values: &[Option<&Value>], ft: &FieldType) -> ArrayRef
    // Recursive: JSON values → typed Arrow arrays

pub fn build_files_batch(
    schema_fields: &[(String, FieldType)],
    files: &[FileRow],
) -> RecordBatch
    // Builds: file_id, filename, data (Struct), content_hash, built_at

pub fn build_chunks_batch(chunks: &[ChunkRow], dimension: i32) -> RecordBatch
    // Builds: chunk_id, file_id, chunk_index, start_line, end_line, embedding

// --- Row types (input to builders) ---

pub struct FileRow {
    pub file_id: String,
    pub filename: String,
    pub frontmatter: Option<Value>,
    pub content_hash: String,
    pub built_at: String,
}

pub struct ChunkRow {
    pub chunk_id: String,
    pub file_id: String,
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub embedding: Vec<f32>,
}

// --- Parquet I/O ---

pub fn write_files_parquet(path: &Path, batch: &RecordBatch) -> Result<()>
    // ArrowWriter, single row group

pub fn write_chunks_parquet(path: &Path, batch: &RecordBatch) -> Result<()>
    // ArrowWriter, single row group

pub fn read_files_parquet(path: &Path) -> Result<RecordBatch>
    // ParquetRecordBatchReaderBuilder

pub fn read_chunks_parquet(path: &Path) -> Result<RecordBatch>
    // ParquetRecordBatchReaderBuilder
```

### `search.rs`

From: `test_search.rs`

```rust
#[derive(Debug)]
pub struct CosineSimilarityUDF {
    signature: Signature,
    query_vector: Vec<f32>,
}

impl CosineSimilarityUDF {
    pub fn new(query_vector: Vec<f32>) -> Self
}

impl PartialEq for CosineSimilarityUDF   // f32 compared via to_bits()
impl Eq for CosineSimilarityUDF
impl Hash for CosineSimilarityUDF         // f32 hashed via to_bits()
impl ScalarUDFImpl for CosineSimilarityUDF
    // invoke_with_args: FixedSizeList<Float32> → Float64 cosine similarity
```

Query building (new, not in prototype — to be designed):

```rust
pub struct SearchQuery {
    pub query_text: String,
    pub where_clause: Option<String>,
    pub limit: usize,
    pub chunks_mode: bool,        // --chunks flag: raw chunk results vs note-level
}

pub async fn execute_search(
    ctx: &SessionContext,
    udf: ScalarUDF,
    query: &SearchQuery,
) -> Result<Vec<SearchResult>>

pub struct SearchResult {
    pub filename: String,
    pub score: f64,
    pub snippet: String,
    // frontmatter fields TBD
}
```

### `cmd/` — CLI Commands

Each command is a thin function that wires the library modules together.

**`cmd/init.rs`** — The big one:
1. `ScannedFiles::scan(root, glob, include_bare_files)`
2. `InferredSchema::infer(&scanned)`
3. `MdvsToml::from_inferred(&schema, &config)` → write `mdvs.toml`
4. `MdvsLock::from_inferred(&schema, &scanned, &config)` → write `mdvs.lock`
5. Print frequency table to stderr
6. `Embedder::load(&model_config)` → download/cache model
7. For each file: `Chunks::new(content, max_chars)` → `Embedder::embed_batch`
8. `build_files_batch` + `build_chunks_batch` → write Parquet to `.mdvs/`

**`cmd/build.rs`** — Incremental rebuild:
1. Read `mdvs.lock` for content hashes
2. `ScannedFiles::scan` for current state
3. Diff: new/modified/deleted files
4. Re-chunk + re-embed only changed files
5. Rewrite Parquet files
6. Update lock with new hashes

**`cmd/search.rs`**:
1. Read `.mdvs/files.parquet` + `.mdvs/chunks.parquet`
2. `Embedder::load` + embed query text
3. `CosineSimilarityUDF::new(query_embedding)`
4. Build SQL (note-level or chunk-level based on `--chunks`)
5. Execute via DataFusion `SessionContext`
6. Format and print results

**`cmd/check.rs`**:
1. Read schema from `mdvs.toml`
2. `ScannedFiles::scan`
3. Validate each file's frontmatter against field defs
4. Report diagnostics

**`cmd/update.rs`**:
1. Re-scan directory
2. Re-infer (or just update lock with current observations)
3. Rewrite `mdvs.lock`

**`cmd/clean.rs`**:
1. `rm -rf .mdvs/`

**`cmd/info.rs`**:
1. Read `mdvs.toml` + `mdvs.lock` + `.mdvs/` Parquet metadata
2. Print: model, file count, chunk count, field list, staleness

### `main.rs`

```rust
#[derive(Parser)]
#[command(name = "mdvs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init { ... },
    Build { ... },
    Search { query: String, ... },
    Check { ... },
    Update { ... },
    Clean,
    Info,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { .. } => cmd::init::run(..),
        Command::Build { .. } => cmd::build::run(..),
        // ...
    }
}
```

---

## Implementation Order

Each step produces a compiling, testable crate. Tests are written alongside each module.

### Step 1: Skeleton
- `Cargo.toml` with all deps
- Empty module stubs (`mod.rs` files with `pub mod` declarations)
- `main.rs` with clap parsing (commands exist but print "not implemented")
- `cargo build` passes

### Step 2: `discover/field_type.rs`
- `FieldType` enum, `From<&Value>`, `widen`, `Into<DataType>`
- Unit tests (from test_widening.rs + test_arrow.rs)

### Step 3: `discover/scan.rs`
- `ScannedFile`, `ScannedFiles::scan`
- Unit tests (from test_scan.rs, uses tempdir)

### Step 4: `discover/infer.rs`
- `DirectoryTree`, `GlobMap`, `InferredSchema`
- Unit tests (from test_inference.rs)

### Step 5: `schema/`
- `shared.rs`: `FieldTypeSerde`, `TomlConfig`
- `config.rs`: `MdvsToml`
- `lock.rs`: `MdvsLock`
- Unit tests (from test_toml.rs)

### Step 6: `index/chunk.rs`
- `Chunks`, `extract_plain_text`, `strip_wikilinks`
- Unit tests (from test_chunk.rs)

### Step 7: `index/embed.rs`
- `ModelConfig`, `Embedder`, `resolve_revision`
- Integration tests (requires model download, from test_embed.rs)

### Step 8: `index/storage.rs`
- Arrow builders + Parquet I/O
- Unit tests (from test_parquet.rs, uses tempdir)

### Step 9: `search.rs`
- `CosineSimilarityUDF`
- Unit tests (from test_search.rs, uses tempdir + DataFusion)

### Step 10: `cmd/` — wire it all together
- `init` first (exercises the full pipeline)
- Then `build`, `search`, `check`, `update`, `clean`, `info`
- Integration tests against real directories

---

## DataFusion Gotchas (reference)

- **Always use `datafusion::arrow` and `datafusion::parquet` re-exports** — never add separate arrow/parquet deps
- **Parquet strings → `Utf8View` / `StringViewArray`** — not `StringArray`
- **`ScalarUDFImpl` requires `DynEq + DynHash`** — manual `PartialEq`/`Eq`/`Hash` impls needed
- **`register_parquet` takes `&str`** path, not `&Path`
- **Struct field access in SQL**: `f.data['field_name']`

---

## .mdvs/ Directory Layout

```
.mdvs/
├── files.parquet      # file_id, filename, data (Struct), content_hash, built_at
└── chunks.parquet     # chunk_id, file_id, chunk_index, start_line, end_line, embedding
```

Config files live at repo root:
```
mdvs.toml              # boundaries (user edits this)
mdvs.lock              # observed state (auto-generated)
```
