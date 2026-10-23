import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { parse } from "smol-toml";
import { Daemon } from "./socket.js";

const TOOL_DEADLINE_MS = 10000;
const text = { type: "string" };
const id = { type: "string", description: "a fct_, ent_ or int_ ID" };
const TOOLS = [
  { command: "search", description: "Search long-term memory: the latest version of each fact, ranked. Write the query the way the fact would be written. Cite the fct_ IDs you use.",
    properties: { text, k: { type: "integer" }, as_of: { type: "integer", description: "epoch milliseconds" }, include_invalid: { type: "boolean" } }, required: ["text"] },
  { command: "entity", description: "Read a person, project or thing from memory: aliases, facts and links.", properties: { name: text }, required: ["name"] },
  { command: "resolve", description: "Find the entity ID for a name or alias.", properties: { name: text }, required: ["name"] },
  { command: "show", description: "Read a memory in full with the records it came from.", properties: { id }, required: ["id"] },
  { command: "history", description: "Read every version of a fact, oldest first.", properties: { id }, required: ["id"] },
  { command: "raw", description: "Keyword search over the raw history of what was said, by words and dates.", properties: { text, limit: { type: "integer" } }, required: ["text"] },
  { command: "useful", description: "Record that a memory helped, which keeps it from fading.", properties: { id }, required: ["id"] },
];

export default function neomem(pi) {
  const home = process.env.NEOMEM_HOME || join(homedir(), ".neomem");
  const daemon = new Daemon(home);
  const signal = (ctx, event) => ({
    agent: "pi",
    session: ctx.sessionManager.getSessionId(),
    transcript_path: ctx.sessionManager.getSessionFile() ?? null,
    event,
  });
  for (const tool of TOOLS) {
    pi.registerTool({
      name: `neomem_${tool.command}`,
      label: `neomem ${tool.command}`,
      description: tool.description,
      parameters: { type: "object", properties: tool.properties, required: tool.required, additionalProperties: false },
      async execute(_toolCallId, params) {
        const response = await daemon.call({ command: tool.command, arguments: params }, TOOL_DEADLINE_MS);
        const text = response === undefined ? "neomem is not running" : JSON.stringify(response.body ?? response);
        return { content: [{ type: "text", text }], details: {} };
      },
    });
  }
  pi.on("turn_end", (_event, ctx) => { daemon.capture(signal(ctx, "stop")); });
  pi.on("agent_end", (_event, ctx) => { daemon.capture(signal(ctx, "end")); });
  pi.on("session_shutdown", (_event, ctx) => {
    daemon.capture(signal(ctx, "end"));
    daemon.close();
  });
  pi.on("before_agent_start", async (event, ctx) => {
    const deadline = Date.now() + 3500;
    let timer;
    const expired = new Promise(resolve => {
      timer = setTimeout(() => { daemon.reset(); resolve(undefined); }, 3500);
    });
    try {
      return await Promise.race([recall(event, ctx), expired]);
    } catch {
      return undefined;
    } finally {
      clearTimeout(timer);
    }
    async function recall(event, ctx) {
      const config = parse(await readFile(join(home, "config.toml"), "utf8"));
      const project = await realpath(ctx.cwd).catch(() => resolve(ctx.cwd));
      const switches = config.recall;
      if ((switches?.projects?.[project] ?? switches?.agents?.pi ?? switches?.enabled ?? false) !== true) return;
      const remaining = deadline - Date.now();
      if (remaining <= 0) return;
      const response = await daemon.call({ command: "recall", arguments: {
        agent: "pi", session: ctx.sessionManager.getSessionId(),
        message: event.prompt, slice: "", project,
      } }, remaining);
      if (response?.reply !== "recalled" || !Array.isArray(response.body)) return;
      if (!response.body.every(memory => typeof memory.id === "string" && typeof memory.content === "string")) return;
      let content = "";
      for (const memory of response.body) {
        const shown = [...(typeof memory.shown === "string" && memory.shown ? memory.shown : `${memory.id}: ${memory.content}\n`)];
        const room = 10000 - [...content].length;
        if (shown.length <= room) { content += shown.join(""); continue; }
        if (room > [...`${memory.id}: `].length + 1) content += `${shown.slice(0, room - 1).join("")}\n`;
        break;
      }
      if (content) return { message: { customType: "neomem-recall", content, display: false } };
    }
  });
}
