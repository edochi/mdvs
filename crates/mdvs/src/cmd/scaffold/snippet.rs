//! `mdvs scaffold snippet` — print the project-rules snippet for the agent.

use std::io::Write;

use anyhow::{Context, Result, anyhow};

use crate::scaffold::{Platform, SCAFFOLDING, SnippetBody};

/// Print the project-rules snippet body to `stdout`. If `platform_name` is
/// `Some`, the platform's preferred body (per `snippet.body` in
/// `platform.toml`) is used and an install hint is written to `stderr`.
///
/// Without `--platform`, defaults to the universal AGENTS.md-flavoured
/// body. That body works in any `AGENTS.md` / `CLAUDE.md` setup.
pub fn run<W: Write, E: Write>(
    stdout: &mut W,
    stderr: &mut E,
    platform_name: Option<&str>,
) -> Result<()> {
    let (body_key, install_hint) = match platform_name {
        Some(name) => {
            let platform = Platform::load(name)?;
            let body_key = body_file(platform.snippet.body);
            let hint = format!(
                "Install to: {} (under {})",
                platform.snippet.target_file, platform.meta.display_name
            );
            (body_key, Some(hint))
        }
        None => (body_file(SnippetBody::AgentsMd), None),
    };

    let file = SCAFFOLDING
        .get_file(body_key)
        .ok_or_else(|| anyhow!("bundled {body_key} is missing — this is a build bug"))?;
    let body = file
        .contents_utf8()
        .ok_or_else(|| anyhow!("bundled {body_key} is not valid UTF-8"))?;

    stdout
        .write_all(body.as_bytes())
        .context("writing snippet body to stdout")?;

    if let Some(hint) = install_hint {
        writeln!(stderr, "{hint}").context("writing install hint")?;
    }

    Ok(())
}

/// Maps a [`SnippetBody`] variant to its bundled path under
/// `scaffolding/snippet/`. Closed-set match enforced by the enum.
fn body_file(body: SnippetBody) -> &'static str {
    match body {
        SnippetBody::AgentsMd => "snippet/agents-md.md",
        SnippetBody::CursorRules => "snippet/cursor-rules.mdc",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_default_emits_agents_md_body() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, None).unwrap();
        let body = String::from_utf8(out).unwrap();
        assert!(
            body.contains("mdvs knowledge base"),
            "snippet should mention the KB heading"
        );
        // Universal body has no Cursor frontmatter wrapping.
        assert!(
            !body.starts_with("---\n"),
            "AGENTS.md body shouldn't have YAML frontmatter"
        );
        assert!(err.is_empty(), "no stderr hint without --platform");
    }

    #[test]
    fn snippet_claude_code_emits_agents_md_body_with_install_hint() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, Some("claude-code")).unwrap();
        let body = String::from_utf8(out).unwrap();
        let hint = String::from_utf8(err).unwrap();
        assert!(body.contains("mdvs knowledge base"));
        assert!(
            !body.starts_with("---\n"),
            "claude-code uses agents-md body, no frontmatter"
        );
        assert!(
            hint.contains("CLAUDE.md"),
            "hint should target CLAUDE.md: {hint}"
        );
    }

    #[test]
    fn snippet_cursor_emits_mdc_body_with_frontmatter() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, Some("cursor")).unwrap();
        let body = String::from_utf8(out).unwrap();
        let hint = String::from_utf8(err).unwrap();
        // .mdc has Cursor frontmatter (alwaysApply: true).
        assert!(
            body.starts_with("---\n"),
            ".mdc body should start with frontmatter"
        );
        assert!(
            body.contains("alwaysApply: true"),
            ".mdc body should set alwaysApply"
        );
        // Hint mentions the .cursor/rules/ target.
        assert!(hint.contains(".cursor/rules/mdvs.mdc"), "hint: {hint}");
    }

    #[test]
    fn snippet_opencode_uses_agents_md_with_agents_md_target() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, Some("opencode")).unwrap();
        let body = String::from_utf8(out).unwrap();
        let hint = String::from_utf8(err).unwrap();
        assert!(!body.starts_with("---\n"));
        assert!(
            hint.contains("AGENTS.md"),
            "hint should target AGENTS.md: {hint}"
        );
    }

    #[test]
    fn snippet_errors_on_unknown_platform() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let res = run(&mut out, &mut err, Some("does-not-exist"));
        assert!(res.is_err());
    }
}
