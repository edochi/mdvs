// mdvs hooks bridge for OpenCode.
//
// OpenCode does not yet ship native shell-command PostToolUse hooks
// (tracked at github.com/anomalyco/opencode/issues/12472). This plugin
// fills the gap by subscribing to `tool.execute.after` and invoking
// `mdvs hook handle` for the same set of events the other harnesses
// hook into:
//
//   - Edit / Write / MultiEdit on a markdown file → validate
//   - Bash with grep/rg/find/etc. → search-nudge
//
// mdvs's own `opencode` platform refuses hook scaffolding (no shell
// surface), so we target the `claude-code` platform — its envelope
// happens to be parseable text we can forward into OpenCode's context
// via `client.session.prompt()`.
//
// Prerequisites:
//   - mdvs on PATH (cargo install --path crates/mdvs, or a release binary
//     in /usr/local/bin/)
//   - mdvs.toml present at the project root or any ancestor
//
// The plugin is transitional. When OpenCode adds first-class
// shell-command hooks, mdvs will fill in `opencode/platform.toml`'s
// `[hooks]` table and this plugin won't be needed.

import { execSync } from "node:child_process";

export const MdvsHooks = async ({ client, project }: any) => ({
    "tool.execute.after": async (input: any) => {
        const cwd: string = project?.directory ?? process.cwd();
        const tool: string = String(input?.tool ?? "").toLowerCase();

        // Decide which mdvs hook (if any) applies to this tool call.
        let kind: "validate" | "search-nudge" | null = null;
        let stdinPayload: Record<string, unknown> = {};

        if (tool === "edit" || tool === "write" || tool === "multiedit") {
            const filePath: string | undefined = input?.args?.filePath;
            if (!filePath?.endsWith(".md")) return;
            kind = "validate";
            stdinPayload = { tool_input: { file_path: filePath }, cwd };
        } else if (tool === "bash") {
            const command: string | undefined = input?.args?.command;
            if (!command) return;
            kind = "search-nudge";
            stdinPayload = { tool_input: { command }, cwd };
        } else {
            return;
        }

        // Invoke mdvs. Silent on any error: mdvs not on PATH, no vault
        // reachable from cwd, no violations, etc. — none of those are
        // worth interrupting the agent.
        let output: string;
        try {
            output = execSync(
                `mdvs hook handle --platform claude-code --kind ${kind}`,
                {
                    input: JSON.stringify(stdinPayload),
                    encoding: "utf8",
                    stdio: ["pipe", "pipe", "pipe"],
                },
            ).trim();
        } catch {
            return;
        }
        if (!output) return;

        // Parse the Claude Code envelope and extract the agent-context
        // message (additionalContext). Cursor and Codex envelopes have
        // different field names; we use the claude-code platform's
        // shape because it carries everything we need.
        let envelope: any;
        try {
            envelope = JSON.parse(output);
        } catch {
            return;
        }
        const agentMessage: string | undefined =
            envelope?.hookSpecificOutput?.additionalContext;
        if (!agentMessage) return;

        // OpenCode's plugin API doesn't have an "additional context"
        // injection mechanism — the closest equivalent is starting a
        // new prompt turn. Tag it clearly so the agent recognises this
        // is a hook-generated message, not a fresh user request.
        const sessionId: string | undefined = input?.sessionID;
        if (!sessionId) return;

        try {
            await client.session.prompt({
                path: { id: sessionId },
                body: {
                    parts: [
                        {
                            type: "text",
                            text: `[mdvs hook] ${agentMessage}`,
                        },
                    ],
                },
            });
        } catch {
            // If injection fails, stay silent — the hook is informational.
        }
    },
});
