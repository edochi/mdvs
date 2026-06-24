//! `mdvs hook handle` — runtime implementation.
//!
//! Reads a hook payload (JSON) from stdin, runs the kind-specific logic,
//! writes a hook-output envelope (JSON) to stdout, exits 0. Hooks are
//! non-blocking by design: violations and tips surface to the agent /
//! user; mdvs never rejects an edit at the harness layer.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::cmd::check;
use crate::cmd::hook::HookKind;
use crate::output::OutputFormat;
use crate::scaffold::{HooksConfig, Platform, template};
use crate::step;

/// Maximum number of lines to send through the user-visible `systemMessage`
/// channel. The agent channel (`additionalContext`) stays uncapped — the
/// agent reads all of it and decides what to surface in its reply.
///
/// Past this point we truncate and append a `...` marker on its own line.
const MAX_USER_LINES: usize = 15;

/// Run the hook handle.
///
/// - Reads the harness payload (Claude Code / Codex / Cursor style stdin
///   JSON) from `stdin`.
/// - Loads the per-platform config (`scaffolding/platforms/<platform>/`).
/// - Dispatches to the right kind: validate or search-nudge.
/// - Writes the platform-shaped JSON envelope to `stdout` (or nothing,
///   when there's nothing to surface — silent path).
/// - Always returns `Ok(())` on the happy path; the caller is expected to
///   exit 0 (non-blocking by design).
///
/// Errors are surfaced for the caller to decide whether to ignore (recipe
/// users may want a malformed payload to be silent rather than noisy).
pub fn run<R: Read, W: Write>(
    stdin: R,
    stdout: &mut W,
    platform_name: &str,
    kind: HookKind,
) -> Result<()> {
    let platform = Platform::load(platform_name).context("loading platform config")?;
    let payload = parse_payload(stdin).context("parsing stdin JSON")?;

    match kind {
        HookKind::Validate => handle_validate(stdout, &platform, &payload),
        HookKind::SearchNudge => handle_search_nudge(stdout, &platform, &payload),
    }
}

// ============================================================================
// Stdin payload shape (lenient — extra fields ignored).
// ============================================================================

/// The subset of harness stdin payload we read. All fields are optional
/// because (a) different harnesses send different shapes and (b) we want
/// the hook to be resilient: a missing field falls back to a sensible
/// default (e.g. silent exit) rather than erroring out.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct HookPayload {
    /// Harness's current working directory at the time of the tool call.
    cwd: Option<String>,
    /// The tool-call input (file edits, bash commands, etc.).
    tool_input: ToolInput,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ToolInput {
    /// Set for Edit / Write / MultiEdit tool calls — the file being changed.
    file_path: Option<String>,
    /// Set for Bash tool calls — the command being run.
    command: Option<String>,
}

fn parse_payload<R: Read>(mut stdin: R) -> Result<HookPayload> {
    let mut buf = String::new();
    stdin.read_to_string(&mut buf).context("reading stdin")?;
    if buf.trim().is_empty() {
        return Ok(HookPayload::default());
    }
    serde_json::from_str::<HookPayload>(&buf).context("decoding hook payload JSON")
}

// ============================================================================
// HookKind::Validate
// ============================================================================

fn handle_validate<W: Write>(
    stdout: &mut W,
    platform: &Platform,
    payload: &HookPayload,
) -> Result<()> {
    // Only fire on .md edits.
    let Some(file_path_str) = payload.tool_input.file_path.as_deref() else {
        return Ok(());
    };
    if !file_path_str.ends_with(".md") {
        return Ok(());
    }

    // Resolve to an absolute path, walking up from there to find mdvs.toml.
    // If a relative path was sent, resolve against the harness's cwd.
    let abs = resolve_path(file_path_str, payload.cwd.as_deref());
    let Some(vault_root) = walk_up_to_mdvs_toml(&abs) else {
        return Ok(());
    };

    // Run validation against the vault. `no_update = true` because hook-
    // triggered validation shouldn't modify `mdvs.toml` (the user doesn't
    // expect an edit to silently write new fields into the schema).
    let result = check::run(
        &vault_root,
        /* no_update */ true,
        /* verbose */ false,
        None,
    );

    // Silent on clean: no violations AND no command-level failure.
    let has_violations = step::has_violations(&result);
    let has_failed = step::has_failed(&result);
    if !has_violations && !has_failed {
        return Ok(());
    }

    // Render twice: markdown for the agent (additionalContext), pretty for
    // the user (systemMessage). Both reuse the same already-collected
    // result; only the formatter differs.
    let agent_body = result
        .render(&OutputFormat::Markdown, /* verbose */ false)
        .context("rendering agent markdown")?;
    let user_body = result
        .render(&OutputFormat::Pretty, /* verbose */ false)
        .context("rendering user pretty")?;

    let agent_msg = append_skill_pointer(&agent_body, &platform.skill.install_path);
    let user_msg = cap_lines(&user_body, MAX_USER_LINES);

    let Some(hooks) = platform.hooks.as_ref() else {
        // Platform has no shell-hook surface (OpenCode, Antigravity). The
        // user shouldn't be invoking this command for such a platform, but
        // if they are, fail loudly so they know.
        anyhow::bail!(
            "platform '{}' has no [hooks] section — `mdvs hook handle` is not supported here. \
             Skill + snippet work via `mdvs scaffold skill|snippet`.",
            platform.meta.name
        );
    };

    let envelope = build_envelope(hooks, &agent_msg, Some(&user_msg));
    writeln!(stdout, "{envelope}").context("writing stdout")?;
    Ok(())
}

// ============================================================================
// HookKind::SearchNudge
// ============================================================================

/// Search-tool prefixes that trigger the nudge. Each entry is matched as a
/// case-sensitive substring of the bash command — same as the shell-script
/// case-statement we extracted from earlier commits.
const SEARCH_TOOL_PATTERNS: &[&str] = &[
    "grep", "rg ", "ripgrep", "find ", "fd ", "fdfind ", "ag ", "ack ", "git grep",
];

fn handle_search_nudge<W: Write>(
    stdout: &mut W,
    platform: &Platform,
    payload: &HookPayload,
) -> Result<()> {
    let Some(hooks) = platform.hooks.as_ref() else {
        anyhow::bail!(
            "platform '{}' has no [hooks] section — `mdvs hook handle` is not supported here.",
            platform.meta.name
        );
    };

    // Walk up from the agent's cwd. If we're not in an mdvs vault, the
    // nudge stays silent.
    let cwd = match payload.cwd.as_deref() {
        Some(c) => PathBuf::from(c),
        None => std::env::current_dir().context("reading current dir")?,
    };
    if walk_up_to_mdvs_toml(&cwd).is_none() {
        return Ok(());
    }

    // Only fire on search-tool commands.
    let Some(command) = payload.tool_input.command.as_deref() else {
        return Ok(());
    };
    if !is_search_command(command) {
        return Ok(());
    }

    let tip = "Tip: `mdvs search` is also available for semantic / hybrid / SQL-filtered search over the KB.";
    let envelope = build_envelope(hooks, tip, None);
    writeln!(stdout, "{envelope}").context("writing stdout")?;
    Ok(())
}

fn is_search_command(command: &str) -> bool {
    SEARCH_TOOL_PATTERNS.iter().any(|pat| command.contains(pat))
}

// ============================================================================
// Helpers — path resolution, walk-up, envelope construction, message shaping
// ============================================================================

fn resolve_path(file_path: &str, cwd: Option<&str>) -> PathBuf {
    let raw = PathBuf::from(file_path);
    if raw.is_absolute() {
        raw
    } else if let Some(cwd) = cwd {
        PathBuf::from(cwd).join(raw)
    } else {
        raw
    }
}

/// Walk up from `start` (which may be a file path or a directory) looking
/// for an `mdvs.toml` in the directory itself or any ancestor.
fn walk_up_to_mdvs_toml(start: &Path) -> Option<PathBuf> {
    // If start is a file, begin from its parent. Stay tolerant of paths
    // that don't exist on disk yet (the agent might have just deleted
    // something or the file may exist but be unreadable due to perms).
    let mut current = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        if current.join("mdvs.toml").is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Cap `body` at `max_lines` lines. If truncation happened, append a
/// `...` marker on its own line so the agent reader sees a clear "more
/// follows" signal. Always trims trailing newlines off `body` first so the
/// truncation marker lands on the right line.
fn cap_lines(body: &str, max_lines: usize) -> String {
    let trimmed = body.trim_end_matches('\n');
    let mut lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() <= max_lines {
        return trimmed.to_string();
    }
    lines.truncate(max_lines);
    let mut out = lines.join("\n");
    out.push_str("\n...");
    out
}

/// Append a pointer to the bundled mdvs skill. The pointer tells the agent
/// (a) where the skill file lives on disk for this platform, and (b) where
/// to find the schema-evolution loop section that covers how to react.
fn append_skill_pointer(body: &str, skill_install_path: &str) -> String {
    let trimmed = body.trim_end_matches('\n');
    format!(
        "{trimmed}\n\n---\n\n_To handle this correctly, load the mdvs skill at \
         `{skill_install_path}` (or run `mdvs scaffold skill` to print it). \
         The schema-evolution loop section covers when to fix the file vs. \
         propose a `mdvs.toml` update._"
    )
}

/// Build the hook-output envelope by substituting `<<MSG>>` and
/// `<<USER_MSG>>` into the platform's envelope template (from
/// `platform.toml`). When `user_msg` is `None`, the template's
/// `<<USER_MSG>>` marker (if any) gets pruned — the user-channel field
/// disappears from the output entirely.
///
/// Per-platform JSON shapes live in `platform.toml`, not here. Different
/// harnesses can have wildly different envelopes (Claude Code's wrapped
/// `hookSpecificOutput`, Cursor's flat `additional_context`, etc.) and
/// this function doesn't care — it just feeds substitution values in.
fn build_envelope(hooks: &HooksConfig, agent_msg: &str, user_msg: Option<&str>) -> String {
    let mut vars: HashMap<&str, Option<String>> = HashMap::new();
    vars.insert("MSG", Some(agent_msg.to_string()));
    vars.insert("USER_MSG", user_msg.map(|s| s.to_string()));
    let envelope = template::substitute(&hooks.envelope, &vars);
    envelope.to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::io::Cursor;
    use tempfile::TempDir;

    /// Build a minimal mdvs vault in `dir`: an `mdvs.toml` with one
    /// categorical `status` field, plus one markdown file.
    ///
    /// The `status_value` parameter lets tests choose between a valid
    /// value (no violations) and an invalid one (one violation surfaces).
    fn write_fixture_vault(dir: &Path, status_value: &str) {
        std::fs::write(
            dir.join("mdvs.toml"),
            r#"
[scan]
glob = "**"
include_bare_files = true
skip_gitignore = false
frontmatter_format = "auto"

[check]
auto_update = false

[[fields.field]]
name = "status"
type = "String"
constraints = { categories = ["active", "archived"] }
"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("note.md"),
            format!("---\nstatus: {status_value}\n---\n# Note\n"),
        )
        .unwrap();
    }

    // --- parse_payload ----------------------------------------------------

    #[test]
    fn parse_payload_empty_stdin_returns_default() {
        let payload = parse_payload(Cursor::new(b"")).unwrap();
        assert!(payload.cwd.is_none());
        assert!(payload.tool_input.file_path.is_none());
        assert!(payload.tool_input.command.is_none());
    }

    #[test]
    fn parse_payload_full_claude_code_shape() {
        let stdin = br#"{
            "session_id": "abc",
            "cwd": "/some/dir",
            "tool_input": { "file_path": "note.md", "command": null }
        }"#;
        let payload = parse_payload(Cursor::new(stdin)).unwrap();
        assert_eq!(payload.cwd.as_deref(), Some("/some/dir"));
        assert_eq!(payload.tool_input.file_path.as_deref(), Some("note.md"));
        assert!(payload.tool_input.command.is_none());
    }

    #[test]
    fn parse_payload_extra_fields_ignored() {
        let stdin = br#"{
            "tool_input": { "file_path": "x.md" },
            "unknown_field": [1, 2, 3]
        }"#;
        let payload = parse_payload(Cursor::new(stdin)).unwrap();
        assert_eq!(payload.tool_input.file_path.as_deref(), Some("x.md"));
    }

    // --- walk_up_to_mdvs_toml --------------------------------------------

    #[test]
    fn walk_up_finds_vault_root_from_file() {
        let dir = TempDir::new().unwrap();
        write_fixture_vault(dir.path(), "active");
        let file = dir.path().join("note.md");
        let found = walk_up_to_mdvs_toml(&file).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn walk_up_finds_vault_root_from_nested_file() {
        let dir = TempDir::new().unwrap();
        write_fixture_vault(dir.path(), "active");
        let sub = dir.path().join("projects").join("alpha");
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("notes.md");
        std::fs::write(&file, "---\nstatus: active\n---\n# Nested\n").unwrap();
        let found = walk_up_to_mdvs_toml(&file).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn walk_up_returns_none_outside_vault() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("note.md");
        std::fs::write(&file, "x").unwrap();
        assert!(walk_up_to_mdvs_toml(&file).is_none());
    }

    // --- cap_lines --------------------------------------------------------

    #[test]
    fn cap_lines_passthrough_under_limit() {
        let input = "a\nb\nc";
        let out = cap_lines(input, 5);
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn cap_lines_truncates_and_appends_marker() {
        let input = "a\nb\nc\nd\ne\nf";
        let out = cap_lines(input, 3);
        assert_eq!(out, "a\nb\nc\n...");
    }

    #[test]
    fn cap_lines_strips_trailing_newlines_before_counting() {
        let input = "a\nb\nc\n\n";
        let out = cap_lines(input, 5);
        assert_eq!(out, "a\nb\nc");
    }

    // --- append_skill_pointer --------------------------------------------

    #[test]
    fn append_skill_pointer_uses_platform_path() {
        let out = append_skill_pointer("Body", ".claude/skills/mdvs/SKILL.md");
        assert!(out.starts_with("Body\n\n---\n\n"));
        assert!(out.contains("`.claude/skills/mdvs/SKILL.md`"));
        assert!(out.contains("schema-evolution loop"));
    }

    // --- is_search_command ----------------------------------------------

    #[test]
    fn is_search_command_recognises_common_tools() {
        for cmd in [
            "grep foo .",
            "rg pattern",
            "find . -name '*.md'",
            "fd extension md",
            "git grep needle",
            "ag pattern",
            "ack -i bar",
        ] {
            assert!(is_search_command(cmd), "should match: {cmd}");
        }
    }

    #[test]
    fn is_search_command_ignores_unrelated() {
        for cmd in ["cargo build", "mdvs search query", "echo hello", "cat file"] {
            assert!(!is_search_command(cmd), "should NOT match: {cmd}");
        }
    }

    // --- build_envelope --------------------------------------------------

    /// With user_msg = Some(...), Claude Code's full Claude-Code-shaped
    /// envelope renders cleanly: PostToolUse + additionalContext +
    /// systemMessage.
    #[test]
    fn build_envelope_claude_code_validate_includes_both_channels() {
        let p = Platform::load("claude-code").unwrap();
        let hooks = p.hooks.as_ref().unwrap();
        let env = build_envelope(hooks, "agent body", Some("user body"));
        let parsed: Value = serde_json::from_str(&env).unwrap();
        assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(
            parsed["hookSpecificOutput"]["additionalContext"],
            "agent body"
        );
        assert_eq!(parsed["systemMessage"], "user body");
    }

    /// With user_msg = None, the `<<USER_MSG>>` marker is pruned and the
    /// resulting envelope omits `systemMessage` entirely. The wrapper
    /// stays because its other field is still populated.
    #[test]
    fn build_envelope_claude_code_search_nudge_prunes_user_message() {
        let p = Platform::load("claude-code").unwrap();
        let hooks = p.hooks.as_ref().unwrap();
        let env = build_envelope(hooks, "tip only", None);
        let parsed: Value = serde_json::from_str(&env).unwrap();
        assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(
            parsed["hookSpecificOutput"]["additionalContext"],
            "tip only"
        );
        assert!(
            parsed.get("systemMessage").is_none(),
            "systemMessage should be pruned"
        );
    }

    // Cursor envelope test removed: mdvs no longer ships a hook config or
    // envelope for cursor — the previous implementation was schema-correct
    // per the docs but not observed firing in a live smoke test.

    // --- run() validate end-to-end --------------------------------------

    #[test]
    fn validate_silent_on_clean_vault() {
        let dir = TempDir::new().unwrap();
        write_fixture_vault(dir.path(), "active");
        let file = dir.path().join("note.md");
        let stdin = format!(r#"{{"tool_input":{{"file_path":"{}"}}}}"#, file.display());
        let mut out = Vec::new();
        run(
            Cursor::new(stdin),
            &mut out,
            "claude-code",
            HookKind::Validate,
        )
        .unwrap();
        assert!(
            out.is_empty(),
            "expected silent exit on clean vault, got: {}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn validate_emits_envelope_with_violations() {
        let dir = TempDir::new().unwrap();
        write_fixture_vault(dir.path(), "bogus");
        let file = dir.path().join("note.md");
        let stdin = format!(r#"{{"tool_input":{{"file_path":"{}"}}}}"#, file.display());
        let mut out = Vec::new();
        run(
            Cursor::new(stdin),
            &mut out,
            "claude-code",
            HookKind::Validate,
        )
        .unwrap();
        assert!(!out.is_empty(), "expected envelope output for violations");

        let env: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(env["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        let context = env["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(
            context.contains("status"),
            "violation should mention the field: {context}"
        );
        assert!(
            context.contains(".claude/skills/mdvs/SKILL.md"),
            "skill pointer should use claude-code's install path: {context}"
        );
        assert!(
            env["systemMessage"].is_string(),
            "systemMessage should be populated"
        );
    }

    #[test]
    fn validate_silent_on_non_md_file() {
        let dir = TempDir::new().unwrap();
        write_fixture_vault(dir.path(), "active");
        let stdin = format!(
            r#"{{"tool_input":{{"file_path":"{}/some.rs"}}}}"#,
            dir.path().display()
        );
        let mut out = Vec::new();
        run(
            Cursor::new(stdin),
            &mut out,
            "claude-code",
            HookKind::Validate,
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn validate_silent_outside_vault() {
        let dir = TempDir::new().unwrap();
        let lone_file = dir.path().join("note.md");
        std::fs::write(&lone_file, "x").unwrap();
        let stdin = format!(
            r#"{{"tool_input":{{"file_path":"{}"}}}}"#,
            lone_file.display()
        );
        let mut out = Vec::new();
        run(
            Cursor::new(stdin),
            &mut out,
            "claude-code",
            HookKind::Validate,
        )
        .unwrap();
        assert!(out.is_empty());
    }

    // Cursor end-to-end envelope test removed: mdvs no longer ships a hook
    // config or envelope for cursor — the previous implementation was
    // schema-correct per the docs but not observed firing in a live smoke
    // test. The `validate_errors_on_platform_without_hooks` test below
    // covers the "no hook config" path more generally.

    #[test]
    fn validate_errors_on_platform_without_hooks() {
        let dir = TempDir::new().unwrap();
        write_fixture_vault(dir.path(), "bogus");
        let file = dir.path().join("note.md");
        let stdin = format!(r#"{{"tool_input":{{"file_path":"{}"}}}}"#, file.display());
        let mut out = Vec::new();
        let err = run(Cursor::new(stdin), &mut out, "opencode", HookKind::Validate)
            .expect_err("opencode has no [hooks] section, run should error");
        let msg = format!("{err}");
        assert!(msg.contains("opencode"), "{msg}");
        assert!(msg.contains("no [hooks] section"), "{msg}");
    }

    // --- run() search-nudge end-to-end -----------------------------------

    #[test]
    fn search_nudge_emits_when_in_vault_and_command_matches() {
        let dir = TempDir::new().unwrap();
        write_fixture_vault(dir.path(), "active");
        let stdin = format!(
            r#"{{"cwd":"{}","tool_input":{{"command":"grep foo ."}}}}"#,
            dir.path().display()
        );
        let mut out = Vec::new();
        run(
            Cursor::new(stdin),
            &mut out,
            "claude-code",
            HookKind::SearchNudge,
        )
        .unwrap();
        assert!(!out.is_empty());
        let env: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(env["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert!(
            env["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains("mdvs search"),
            "tip should mention mdvs search"
        );
        assert!(
            env.get("systemMessage").is_none(),
            "search-nudge should not surface to user UI — only to model"
        );
    }

    #[test]
    fn search_nudge_silent_outside_vault() {
        let dir = TempDir::new().unwrap();
        let stdin = format!(
            r#"{{"cwd":"{}","tool_input":{{"command":"grep foo ."}}}}"#,
            dir.path().display()
        );
        let mut out = Vec::new();
        run(
            Cursor::new(stdin),
            &mut out,
            "claude-code",
            HookKind::SearchNudge,
        )
        .unwrap();
        assert!(out.is_empty(), "no nudge outside a vault");
    }

    #[test]
    fn search_nudge_silent_on_non_search_command() {
        let dir = TempDir::new().unwrap();
        write_fixture_vault(dir.path(), "active");
        let stdin = format!(
            r#"{{"cwd":"{}","tool_input":{{"command":"cargo build"}}}}"#,
            dir.path().display()
        );
        let mut out = Vec::new();
        run(
            Cursor::new(stdin),
            &mut out,
            "claude-code",
            HookKind::SearchNudge,
        )
        .unwrap();
        assert!(out.is_empty());
    }
}
