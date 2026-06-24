//! `mdvs scaffold skill` — print the bundled mdvs skill file.

use std::io::Write;

use anyhow::{Context, Result, anyhow};

use crate::scaffold::{Platform, SCAFFOLDING};

/// Print the bundled `SKILL.md` to `stdout`. If `platform_name` is `Some`,
/// look up the platform and write an install-path hint to `stderr` (the
/// body on stdout is identical regardless of platform — the skill content
/// is harness-agnostic).
pub fn run<W: Write, E: Write>(
    stdout: &mut W,
    stderr: &mut E,
    platform_name: Option<&str>,
) -> Result<()> {
    let file = SCAFFOLDING
        .get_file("skill/SKILL.md")
        .ok_or_else(|| anyhow!("bundled skill/SKILL.md is missing — this is a build bug"))?;
    let body = file
        .contents_utf8()
        .ok_or_else(|| anyhow!("bundled skill/SKILL.md is not valid UTF-8"))?;

    stdout
        .write_all(body.as_bytes())
        .context("writing skill body to stdout")?;

    if let Some(name) = platform_name {
        let platform = Platform::load(name)?;
        writeln!(
            stderr,
            "Install to: {} (under {})",
            platform.skill.install_path, platform.meta.display_name
        )
        .context("writing install hint")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_emits_bundled_body() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, None).unwrap();
        let body = String::from_utf8(out).unwrap();
        // First line of SKILL.md is the YAML frontmatter delimiter.
        assert!(
            body.starts_with("---\n"),
            "unexpected skill body start: {body:.60}"
        );
        assert!(
            body.contains("name: mdvs"),
            "skill frontmatter should declare name"
        );
        // No --platform → no stderr hint.
        assert!(err.is_empty(), "expected no stderr hint without --platform");
    }

    #[test]
    fn skill_emits_install_hint_to_stderr_when_platform_given() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, Some("claude-code")).unwrap();
        let body = String::from_utf8(out).unwrap();
        let hint = String::from_utf8(err).unwrap();
        // Body unchanged by --platform.
        assert!(body.starts_with("---\n"));
        // Hint mentions the platform's install path.
        assert!(
            hint.contains(".claude/skills/mdvs/SKILL.md"),
            "hint: {hint}"
        );
        assert!(
            hint.contains("Claude Code"),
            "hint should name the display: {hint}"
        );
    }

    #[test]
    fn skill_errors_on_unknown_platform() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let res = run(&mut out, &mut err, Some("does-not-exist"));
        assert!(res.is_err());
        // The body is still printed before the platform load errors —
        // that's acceptable; the redirect target would still have a valid
        // skill file, and the user sees the error explaining the hint
        // couldn't be emitted.
    }

    #[test]
    fn skill_body_includes_pivot_era_content() {
        // Sanity check that we're emitting the rewritten SKILL.md (Step 2
        // content), not some stale version: the new SKILL.md describes
        // the schema-evolution loop explicitly.
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, None).unwrap();
        let body = String::from_utf8(out).unwrap();
        assert!(
            body.contains("schema-evolution loop"),
            "should mention the loop"
        );
        assert!(
            body.contains("mdvs scaffold"),
            "should reference new commands"
        );
    }
}
