import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { parse } from "smol-toml";
import { Daemon } from "./socket.js";

export default function neomem(pi) {
  const home = process.env.NEOMEM_HOME || join(homedir(), ".neomem");
  const daemon = new Daemon(home);
  const signal = (ctx, event) => ({
    agent: "pi",
    session: ctx.sessionManager.getSessionId(),
    transcript_path: ctx.sessionManager.getSessionFile() ?? null,
    event,
  });
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
      let charactersLeft = 10000;
      for (const memory of response.body) {
        const prefix = `${memory.id}: `;
        const budget = charactersLeft - [...prefix].length - 1;
        if (budget <= 0) break;
        const body = [...memory.content].slice(0, budget);
        content += `${prefix}${body.join("")}\n`;
        charactersLeft -= [...prefix].length + body.length + 1;
      }
      if (content) return { message: { customType: "neomem-recall", content, display: false } };
    }
  });
}
