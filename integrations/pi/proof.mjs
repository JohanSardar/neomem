import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { join } from "node:path";
import { performance } from "node:perf_hooks";
import extension from "./index.js";

const watchdog = setTimeout(() => { process.exit(1); }, 12000);
watchdog.unref();
const home = process.argv[2];
process.env.NEOMEM_HOME = home;
await mkdir(join(home, "run"), { recursive: true });
await writeFile(join(home, "config.toml"), "[recall.agents]\npi = true\n");
let connections = 0;
let slow = false;
const captures = [];
let captured;
const both = new Promise(resolve => { captured = resolve; });
const server = createServer(socket => {
  connections++;
  let buffer = Buffer.alloc(0);
  let sending = Promise.resolve();
  socket.on("error", () => {});
  socket.on("data", data => {
    buffer = Buffer.concat([buffer, data]);
    while (buffer.length >= 4 && buffer.length >= buffer.readUInt32BE() + 4) {
      const length = buffer.readUInt32BE();
      const envelope = JSON.parse(buffer.subarray(4, length + 4).toString());
      buffer = buffer.subarray(length + 4);
      assert.equal(envelope.protocol, 1);
      const { request } = envelope;
      if (request.command === "capture") {
        captures.push(request.arguments);
        if (captures.length === 2) captured();
      } else if (slow) continue;
      const response = request.command === "capture" ? { reply: "captured" } : {
        reply: "recalled", body: [
          { id: "fct_01M55FJ500000000000000000D", content: request.arguments.message === "near-limit" ? "x".repeat(9960) : "🧠".repeat(12000) },
          { id: "fct_01M5D6RA00000000000000000B", content: "second memory" },
        ],
      };
      const body = Buffer.from(JSON.stringify(response));
      const frame = Buffer.alloc(body.length + 4);
      frame.writeUInt32BE(body.length);
      body.copy(frame, 4);
      sending = sending.then(() => new Promise(resolve => {
        socket.write(frame.subarray(0, 2));
        setImmediate(() => { socket.write(frame.subarray(2)); resolve(); });
      }));
    }
  });
});
await new Promise(resolve => server.listen(join(home, "run", "neomemd.sock"), resolve));
const handlers = new Map();
extension({ on: (event, handler) => handlers.set(event, handler) });
const ctx = { cwd: home, sessionManager: { getSessionId: () => "wire-proof", getSessionFile: () => join(home, "session.jsonl") } };
const prompt = { prompt: "again" };
const recalled = await handlers.get("before_agent_start")(prompt, ctx);
assert.equal([...recalled.message.content].length, 10000);
assert.ok(recalled.message.content.startsWith("fct_"));
assert.equal(handlers.get("turn_end")({}, ctx), undefined);
assert.equal(handlers.get("agent_end")({}, ctx), undefined);
await both;
assert.deepEqual(captures.map(signal => signal.event), ["stop", "end"]);
assert.ok(captures.every(signal => signal.agent === "pi" && signal.session === "wire-proof"));
assert.equal(connections, 1);
const nearLimit = await handlers.get("before_agent_start")({ prompt: "near-limit" }, ctx);
assert.equal(nearLimit.message.content.trimEnd().split("\n").length, 1);
assert.ok(nearLimit.message.content.endsWith("\n"));
slow = true;
const started = performance.now();
assert.equal(await handlers.get("before_agent_start")(prompt, ctx), undefined);
assert.ok(performance.now() - started < 3650);
slow = false;
assert.ok(await handlers.get("before_agent_start")(prompt, ctx));
assert.equal(connections, 2);
await writeFile(join(home, "config.toml"), "[recall.agents]\npi = false\n");
assert.equal(await handlers.get("before_agent_start")(prompt, ctx), undefined);
handlers.get("session_shutdown")({}, ctx);
await new Promise(resolve => server.close(resolve));
clearTimeout(watchdog);
console.log("one socket; fragmented frames; synchronous stop/end capture; Unicode cap; 3.5s fail-open; reconnect; recall off");
