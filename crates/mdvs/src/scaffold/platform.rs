//! Per-platform configuration loaded from
//! `scaffolding/platforms/<name>/platform.toml`.
//!
//! The [`Platform`] struct is the in-memory shape of a `platform.toml`. It's
//! plain data — no enum (so new platforms come from new toml files, not
//! recompiles) and no `dyn Trait` (mdvs is a binary, not a library; the
//! "extensibility surface" lives in toml files, not in Rust types).
//!
//! Loading goes through [`Platform::load`]; enumerating bundled platforms
//! goes through [`Platform::list`]. Both read from the bundled
//! [`super::SCAFFOLDING`] tree, so there's no disk access at runtime.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

/// One agent harness's configuration. Deserialised from
/// `scaffolding/platforms/<name>/platform.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Platform {
    /// Identification + display metadata.
    pub meta: Meta,
    /// Where `mdvs scaffold skill` suggests installing the bundled
    /// `SKILL.md`, and where `mdvs hook handle` expects to find it.
    pub skill: SkillConfig,
    /// Where `mdvs scaffold snippet` writes the project-rules block, and
    /// which bundled body template to use.
    pub snippet: SnippetConfig,
    /// Hook configuration. `None` for harnesses without a shell-command
    /// hook surface (OpenCode's TypeScript plugin API, Antigravity's
    /// undocumented post-rebrand surface). `mdvs scaffold hook` refuses
    /// for these and points the user at the recipe page.
    pub hooks: Option<HooksConfig>,
}

/// Identification and display metadata for one platform.
#[derive(Debug, Clone, Deserialize)]
pub struct Meta {
    /// Canonical platform name. Must match the directory name under
    /// `scaffolding/platforms/`.
    pub name: String,
    /// Human-readable name for help text + documentation.
    pub display_name: String,
    /// Optional pointer to the harness's hooks / skills documentation.
    /// Surfaced in `mdvs scaffold hook` output so users can verify the
    /// generated config against the upstream reference.
    pub documentation_url: Option<String>,
}

/// Skill install configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillConfig {
    /// Path where the harness expects to find the skill file, relative to
    /// the project / workspace root. Surfaced as a help-text hint by
    /// `mdvs scaffold skill --platform <name>`.
    pub install_path: String,
}

/// Snippet (project-rules) install configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct SnippetConfig {
    /// Target file where the snippet should land (relative to the project
    /// root). Examples: `CLAUDE.md`, `AGENTS.md`, `.cursor/rules/mdvs.mdc`.
    pub target_file: String,
    /// Which bundled body template to use.
    pub body: SnippetBody,
}

/// The available snippet body templates under `scaffolding/snippet/`.
///
/// New body types require both a new variant here and a corresponding
/// `scaffolding/snippet/<key>.<ext>` file — that's why this is a closed
/// enum rather than a free-form string. The set is small and stable.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SnippetBody {
    /// `scaffolding/snippet/agents-md.md` — plain markdown, no frontmatter.
    /// Drops into `AGENTS.md`, `CLAUDE.md`, or any always-on rules file.
    AgentsMd,
    /// `scaffolding/snippet/cursor-rules.mdc` — same body wrapped in
    /// Cursor's `.mdc` frontmatter (`alwaysApply: true`).
    CursorRules,
}

/// Hook configuration for a platform.
///
/// Present only for harnesses with a shell-command hook surface (Claude
/// Code, Codex, Cursor). OpenCode and Antigravity have `Platform::hooks ==
/// None`.
#[derive(Debug, Clone, Deserialize)]
pub struct HooksConfig {
    /// Path to the harness's hooks config file (relative to the project
    /// root). Examples: `.claude/settings.json`, `.codex/hooks.json`,
    /// `.cursor/hooks.json`.
    pub config_path: String,
    /// On-disk format of the hooks config file.
    pub config_format: HookConfigFormat,
    /// Value to use in both the matcher key and the
    /// `hookSpecificOutput.hookEventName` envelope field. Examples:
    /// `"PostToolUse"` (Claude Code, Codex), `"postToolUse"` (Cursor).
    pub event_name: String,
    /// Tool-name matcher pattern for the validate-on-write hook. The
    /// pipe-separated string follows the harness's matcher syntax.
    /// Example: `"Edit|Write|MultiEdit"`.
    pub matcher_validate: String,
    /// Tool-name matcher pattern for the search-nudge hook. Example:
    /// `"Bash"`.
    pub matcher_search: String,
}

/// File format of a harness's hook config file.
///
/// Closed enum because the set is small and any new format requires emit
/// support in `mdvs scaffold hook`.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HookConfigFormat {
    /// JSON, the common case (Claude Code, Codex, Cursor).
    Json,
    /// TOML — placeholder for harnesses that ship a `[hooks]` table
    /// instead of a separate JSON file (Codex also accepts this in
    /// `~/.codex/config.toml`).
    Toml,
}

impl Platform {
    /// Load the platform config for `name` from the bundled
    /// `scaffolding/platforms/<name>/platform.toml`. Returns an error if no
    /// such platform exists or the toml is malformed.
    pub fn load(name: &str) -> Result<Self> {
        let path = format!("platforms/{name}/platform.toml");
        let file = super::SCAFFOLDING.get_file(&path).ok_or_else(|| {
            anyhow!(
                "unknown platform '{name}' — bundled platforms are: {}",
                Self::list().join(", ")
            )
        })?;
        let content = file
            .contents_utf8()
            .ok_or_else(|| anyhow!("platform.toml for '{name}' is not valid UTF-8"))?;
        toml::from_str(content)
            .with_context(|| format!("parsing scaffolding/{path}"))
    }

    /// List the names of all bundled platforms, sorted alphabetically.
    /// Reads from the embedded scaffolding directory; no disk access.
    pub fn list() -> Vec<String> {
        let Some(platforms_dir) = super::SCAFFOLDING.get_dir("platforms") else {
            return Vec::new();
        };
        let mut names: Vec<String> = platforms_dir
            .dirs()
            .filter_map(|d| d.path().file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All five bundled platforms load without error.
    #[test]
    fn bundled_platforms_load() {
        for name in ["claude-code", "codex", "cursor", "opencode", "antigravity"] {
            let p = Platform::load(name).unwrap_or_else(|e| {
                panic!("failed to load platform '{name}': {e:?}");
            });
            assert_eq!(p.meta.name, name, "platform.toml `meta.name` must match dir name");
        }
    }

    /// `Platform::list` returns the five bundled names in alphabetical order.
    #[test]
    fn list_returns_bundled_platforms_sorted() {
        let got = Platform::list();
        let expected = vec![
            "antigravity".to_string(),
            "claude-code".to_string(),
            "codex".to_string(),
            "cursor".to_string(),
            "opencode".to_string(),
        ];
        assert_eq!(got, expected);
    }

    /// Unknown platform name surfaces a helpful error listing the available
    /// platforms.
    #[test]
    fn unknown_platform_error_lists_known() {
        let err = Platform::load("does-not-exist").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown platform 'does-not-exist'"), "{msg}");
        assert!(msg.contains("claude-code"), "available platforms should be listed: {msg}");
    }

    /// Claude Code uses PascalCase event names (per its hooks docs).
    #[test]
    fn claude_code_uses_pascal_case_event() {
        let p = Platform::load("claude-code").unwrap();
        let hooks = p.hooks.as_ref().expect("claude-code has [hooks]");
        assert_eq!(hooks.event_name, "PostToolUse");
        assert_eq!(hooks.config_path, ".claude/settings.json");
        assert_eq!(hooks.config_format, HookConfigFormat::Json);
    }

    /// Cursor uses camelCase event names (per its hooks docs).
    #[test]
    fn cursor_uses_camel_case_event() {
        let p = Platform::load("cursor").unwrap();
        let hooks = p.hooks.as_ref().expect("cursor has [hooks]");
        assert_eq!(hooks.event_name, "postToolUse");
        assert_eq!(hooks.config_path, ".cursor/hooks.json");
    }

    /// Cursor's snippet uses the `.mdc` body (with frontmatter), targeting
    /// `.cursor/rules/`.
    #[test]
    fn cursor_snippet_uses_mdc_body() {
        let p = Platform::load("cursor").unwrap();
        assert_eq!(p.snippet.body, SnippetBody::CursorRules);
        assert_eq!(p.snippet.target_file, ".cursor/rules/mdvs.mdc");
    }

    /// OpenCode has no shell-hook surface, so `hooks` is `None`. `mdvs
    /// scaffold hook --platform opencode` reads this and refuses.
    #[test]
    fn opencode_has_no_hooks_section() {
        let p = Platform::load("opencode").unwrap();
        assert!(p.hooks.is_none(), "opencode should not declare [hooks]");
        assert_eq!(p.skill.install_path, ".opencode/skills/mdvs/SKILL.md");
        assert_eq!(p.snippet.target_file, "AGENTS.md");
    }

    /// Antigravity has no documented hook surface (yet) — `hooks` is `None`.
    /// The skill + snippet install paths are well-documented in Google's
    /// Codelab and survive the rebrand.
    #[test]
    fn antigravity_has_no_hooks_section() {
        let p = Platform::load("antigravity").unwrap();
        assert!(p.hooks.is_none(), "antigravity should not declare [hooks] in v1");
        assert_eq!(p.skill.install_path, ".agents/skills/mdvs/SKILL.md");
        assert_eq!(p.snippet.target_file, "AGENTS.md");
    }

    /// Codex shares Claude Code's PostToolUse capitalization but lives at a
    /// different config path.
    #[test]
    fn codex_shares_pascal_case_with_different_path() {
        let p = Platform::load("codex").unwrap();
        let hooks = p.hooks.as_ref().expect("codex has [hooks]");
        assert_eq!(hooks.event_name, "PostToolUse");
        assert_eq!(hooks.config_path, ".codex/hooks.json");
        assert_eq!(p.skill.install_path, ".agents/skills/mdvs/SKILL.md");
    }

    /// Every bundled platform declares the same matcher patterns for the
    /// hook tools, since they all share the Edit/Write/MultiEdit + Bash
    /// model. This is a consistency regression check — if a future
    /// platform.toml diverges, the test highlights it for review.
    #[test]
    fn hook_matchers_are_consistent_across_platforms() {
        for name in ["claude-code", "codex", "cursor"] {
            let p = Platform::load(name).unwrap();
            let hooks = p.hooks.as_ref().expect("has [hooks]");
            assert_eq!(hooks.matcher_validate, "Edit|Write|MultiEdit", "{name}");
            assert_eq!(hooks.matcher_search, "Bash", "{name}");
        }
    }
}
