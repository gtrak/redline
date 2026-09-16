/**
 * A Graft plugin for OpenCode that mimicks the behaviour of the official Claude Code plugin.
 *
 * Author: Andrea Arighi - @salvioner
 */

import type { Plugin } from "@opencode-ai/plugin"
import { randomUUID } from "node:crypto"

export const GraftPlugin: Plugin = async ({ directory, $, client }) => {
  // Nodes already injected this session, keyed by sessionID.
  const injected = new Map<string, Set<string>>()

  // Throttle for the slow `graft check` full-content-hash pass.
  const CHECK_INTERVAL = 5 * 60 * 1000
  let lastCheck = 0

  // Split a graft `pointer` ("path" or "path:Lx-Ly") into file + start line.
  const splitPointer = (pointer: string) => {
    const i = pointer.lastIndexOf(":")
    if (i === -1) return { file: pointer, line: "1" }
    return {
      file: pointer.slice(0, i),
      line: pointer.slice(i + 1).replace(/^L/, "").split("-")[0] || "1",
    }
  }

  // Fire-and-forget drift check: logs JSON to stdout even on exit 1, so
  // nothrow() is required (Bun shell throws on non-zero exit).
  const checkAndToast = async () => {
    try {
      const result = await $`graft check --json`.nothrow().quiet()
      const data = JSON.parse(result.text())
      const context = data?.context
      const graph = data?.graph
      if (!context || context.missing) {
        if (!graph || graph.missing) return
      }
      const count =
        (context?.contentDrift?.length ?? 0) +
        (context?.removed?.length ?? 0) +
        (context?.indexDrift?.length ?? 0) +
        (graph
          ? (graph.added?.length ?? 0) +
            (graph.removed?.length ?? 0) +
            (graph.changed?.length ?? 0) +
            (graph.stale?.length ?? 0)
          : 0)
      if (count === 0) return
      await client.tui.showToast({
        body: {
          variant: "warning",
          message: `⚠ graft: ${count} files out of sync — run \`graft build\``,
        },
      })
    } catch {
      // non-fatal
    }
  }

  return {
    // --- Context injection on each user message ---
    "chat.message": async (input, output) => {
      // prompt = non-synthetic text parts only (keeps the injected repo map /
      // context from being re-fed into graft ask)
      let prompt = ""
      for (const part of output.parts) {
        if (part.type === "text" && !part.synthetic) prompt += part.text + "\n"
      }
      prompt = prompt.trim()
      if (!prompt) return

      try {
        // confirmed: graft ask --json → { hits: [{ kind, title, pointer
        // ("file:Lx-Ly"), snippet, score }] } — verified against a real graph.
        const result = await $`graft ask ${prompt} --json`.quiet()
        const data = JSON.parse(result.text())
        const hits = (data?.hits ?? []).slice(0, 5)
        if (!hits.length) return

        let seen = injected.get(input.sessionID)
        if (!seen) {
          seen = new Set()
          injected.set(input.sessionID, seen)
        }

        const lines: string[] = []
        for (const hit of hits) {
          const id = `${hit.title}@${hit.pointer}`
          if (seen.has(id)) continue
          seen.add(id)
          const { file, line } = splitPointer(hit.pointer ?? "")
          const summary = hit.snippet ? `: ${hit.snippet}` : ""
          lines.push(`- ${hit.title} (${file}:${line})${summary}`)
        }
        if (!lines.length) return

        // Part schema source-confirmed: id must start "prt", plus messageID /
        // sessionID / type / text (opencode session/schema.ts). NOTE: live
        // render/persist check was inconclusive — grep matched conversation
        // text, not an injected part; clean re-test pending (see plan).
        output.parts.push({
          id: `prt_${randomUUID()}`,
          messageID: output.message.id,
          sessionID: input.sessionID,
          type: "text",
          text: `[graft context]\n${lines.join("\n")}`,
          synthetic: true,
        })
      } catch {
        // never block the user's prompt
      }
    },

    // --- Session events (created / idle) ---
    // confirmed live: session.created / session.idle only reach plugins via the
    // `event` hook — a direct "session.idle" key never fires (the old key was
    // dead code). See hook-vs-event run: only EVENT lines printed.
    event: async ({ event }) => {
      if (event.type === "session.created") {
        // Seed the session with a repo map (structural, no LLM).
        try {
          const result = await $`graft map --max-dirs 15`.quiet()
          const repoMap = result.text().trim()
          if (!repoMap) return
          // client.session.prompt with noReply:true seeds context without an
          // assistant turn (SDK docs).
          await client.session.prompt({
            path: { id: event.properties.info.id },
            body: {
              noReply: true,
              parts: [{ type: "text", text: repoMap, synthetic: true }],
            },
          })
        } catch {
          // non-fatal
        }
      }

      if (event.type === "session.idle") {
        // Rebuild the graph (fast — content-hash cache skips unchanged files).
        try {
          await $`graft build --no-reuse=false`.quiet()
        } catch {
          // non-fatal
        }

        // Drift toast: throttled to once per 5 min and fire-and-forget, since
        // `graft check` re-hashes all file contents (120s+ on large repos) and
        // must not stall the idle handler.
        if (Date.now() - lastCheck >= CHECK_INTERVAL) {
          lastCheck = Date.now()
          void checkAndToast()
        }
      }
    },

    // --- Blast-radius warning after an edit (existing) ---
    "tool.execute.after": async (input) => {
      if (input.tool !== "edit" && input.tool !== "write") return

      const filePath = input.args?.filePath as string | undefined
      if (!filePath) return

      try {
        const result = await $`graft callers ${filePath} --json`.quiet()
        const data = JSON.parse(result.text())

        if (data?.callers?.length) {
          console.log(
            `⚠ graft: ${filePath} has ${data.callers.length} dependents — ` +
            data.callers.slice(0, 5).map((c: any) => c.symbol).join(", ")
          )
        }
      } catch {
        // best-effort only — don't block the agent
      }
    },
  }
}

export default GraftPlugin
