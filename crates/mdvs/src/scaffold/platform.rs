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
use serde::{Deserialize, Deserializer};
use serde_json::Value;

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
///
/// Per-harness JSON shapes (the envelope `mdvs hook handle` emits and the
/// config `mdvs scaffold hook` emits) live in the [`envelope`] and
/// [`config`] templates as data — see [`super::template`] for the
/// substitution rules. New harnesses with novel shapes can be supported
/// by adding a new `platform.toml` file; no Rust changes required.
#[derive(Debug, Clone, Deserialize)]
pub struct HooksConfig {
    /// Path to the harness's hooks config file (relative to the project
    /// root). Examples: `.claude/settings.json`, `.codex/hooks.json`,
    /// `.cursor/hooks.json`.
    pub config_path: String,
    /// On-disk format of the hooks config file.
    pub config_format: HookConfigFormat,
    /// The envelope template `mdvs hook handle` emits. Parsed as JSON at
    /// platform-load time; available placeholders are `<<MSG>>` (agent
    /// context, always populated) and `<<USER_MSG>>` (user channel,
    /// populated for `validate` only — pruned for `search-nudge`).
    #[serde(deserialize_with = "deserialize_template")]
    pub envelope: Value,
    /// The config-snippet template `mdvs scaffold hook` emits. Parsed as
    /// JSON at platform-load time; available placeholders are
    /// `<<COMMAND_VALIDATE>>` and `<<COMMAND_SEARCH>>` (the full `mdvs
    /// hook handle …` commands, built from the platform name).
    #[serde(deserialize_with = "deserialize_template")]
    pub config: Value,
}

/// Custom serde deserializer that reads a TOML table `{ template = "…" }`
/// and parses the inner string as JSON. The template must be valid JSON at
/// load time — a malformed template surfaces here, not at runtime.
fn deserialize_template<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Value, D::Error> {
    #[derive(Deserialize)]
    struct TemplateTable {
        template: String,
    }
    let wrapper = TemplateTable::deserialize(deserializer)?;
    serde_json::from_str(&wrapper.template).map_err(serde::de::Error::custom)
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
        toml::from_str(content).with_context(|| format!("parsing scaffolding/{path}"))
    }

    /// List the names of all bundled platforms, sorted alphabetically.
    /// Reads from the embedded scaffolding directory; no disk access.
    pub fn list() -> Vec<String> {
        let Some(platforms_dir) = super::SCAFFOLDING.get_dir("platforms") else {
            return Vec::new();
        };
        let mut names: Vec<String> = platforms_dir
            .dirs()
            .filter_map(|d| {
                d.path()
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
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
            assert_eq!(
                p.meta.name, name,
                "platform.toml `meta.name` must match dir name"
            );
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
        assert!(
            msg.contains("claude-code"),
            "available platforms should be listed: {msg}"
        );
    }

    /// Claude Code's envelope template carries the PostToolUse PascalCase
    /// event name baked into the JSON shape. The structural detail —
    /// whether event_name lives in a field, deep in a wrapper, or
    /// anywhere else — is platform.toml's concern, not Rust's.
    #[test]
    fn claude_code_envelope_template_uses_post_tool_use() {
        let p = Platform::load("claude-code").unwrap();
        let hooks = p.hooks.as_ref().expect("claude-code has [hooks]");
        assert_eq!(hooks.config_path, ".claude/settings.json");
        assert_eq!(hooks.config_format, HookConfigFormat::Json);
        // Envelope template contains the right event name as a literal.
        let envelope_str = serde_json::to_string(&hooks.envelope).unwrap();
        assert!(
            envelope_str.contains("PostToolUse"),
            "claude-code envelope should bake in PostToolUse literally: {envelope_str}"
        );
    }

    /// Cursor's snippet uses the `.mdc` body (with frontmatter), targeting
    /// `.cursor/rules/`.
    #[test]
    fn cursor_snippet_uses_mdc_body() {
        let p = Platform::load("cursor").unwrap();
        assert_eq!(p.snippet.body, SnippetBody::CursorRules);
        assert_eq!(p.snippet.target_file, ".cursor/rules/mdvs.mdc");
    }

    /// Only Claude Code ships a verified [hooks] config today. The other
    /// four platforms either don't have a shell-hook surface (OpenCode,
    /// Antigravity) or had their previous hook config retracted after live
    /// smoke tests failed to confirm firing (Codex, Cursor).
    #[test]
    fn only_claude_code_ships_hooks() {
        assert!(Platform::load("claude-code").unwrap().hooks.is_some());
        for name in ["codex", "cursor", "opencode", "antigravity"] {
            let p = Platform::load(name).unwrap();
            assert!(p.hooks.is_none(), "{name} should not declare [hooks]");
        }
    }

    /// Claude Code's config template references the
    /// `<<COMMAND_VALIDATE>>` and `<<COMMAND_SEARCH>>` placeholders.
    /// Regression check — `mdvs scaffold hook` won't be able to fill the
    /// commands in if claude-code/platform.toml drops the markers.
    #[test]
    fn config_template_references_command_markers() {
        let p = Platform::load("claude-code").unwrap();
        let hooks = p.hooks.as_ref().expect("claude-code has [hooks]");
        let config_str = serde_json::to_string(&hooks.config).unwrap();
        assert!(
            config_str.contains("<<COMMAND_VALIDATE>>"),
            "claude-code config template missing COMMAND_VALIDATE marker"
        );
        assert!(
            config_str.contains("<<COMMAND_SEARCH>>"),
            "claude-code config template missing COMMAND_SEARCH marker"
        );
    }
}
