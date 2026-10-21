import { connect } from "node:net";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join } from "node:path";

export class Daemon {
  constructor(home) {
    this.home = home;
    this.socket = undefined;
    this.pending = [];
    this.buffer = Buffer.alloc(0);
  }

  call(request, timeout = 3500) {
    return new Promise(resolve => {
      const socket = this.connection();
      const timer = setTimeout(() => this.reset(socket), timeout);
      const entry = { resolve, timer };
      this.pending.push(entry);
      socket.ref();
      const body = Buffer.from(JSON.stringify({ protocol: 1, request }));
      const header = Buffer.alloc(4);
      header.writeUInt32BE(body.length);
      socket.write(Buffer.concat([header, body]));
    });
  }

  capture(signal) {
    const request = { command: "capture", arguments: signal };
    void this.call(request).then(response => {
      if (response?.reply !== "captured") {
        return this.spool(request);
      }
    }).catch(() => {});
  }

  connection() {
    if (this.socket) return this.socket;
    const socket = connect(join(this.home, "run", "neomemd.sock"));
    this.socket = socket;
    socket.on("data", data => this.receive(socket, data));
    socket.on("error", () => this.reset(socket));
    socket.on("close", () => this.reset(socket));
    return socket;
  }

  receive(socket, data) {
    if (socket !== this.socket) return;
    this.buffer = Buffer.concat([this.buffer, data]);
    while (this.buffer.length >= 4) {
      const length = this.buffer.readUInt32BE();
      if (length > 64 * 1024 * 1024) return this.reset(socket);
      if (this.buffer.length < length + 4) break;
      const body = this.buffer.subarray(4, length + 4);
      this.buffer = this.buffer.subarray(length + 4);
      const entry = this.pending.shift();
      if (!entry) return this.reset(socket);
      clearTimeout(entry.timer);
      try {
        entry.resolve(JSON.parse(body.toString()));
      } catch {
        entry.resolve(undefined);
        return this.reset(socket);
      }
    }
    if (this.pending.length === 0) socket.unref();
  }

  reset(socket = this.socket) {
    if (!socket || socket !== this.socket) return;
    this.socket = undefined;
    this.buffer = Buffer.alloc(0);
    socket.destroy();
    for (const entry of this.pending.splice(0)) {
      clearTimeout(entry.timer);
      entry.resolve(undefined);
    }
  }

  close() {
    this.socket?.end();
  }

  async spool(request) {
    const body = JSON.stringify({ protocol: 1, request });
    const stamp = (BigInt(Date.now()) * 1000000n + process.hrtime.bigint() % 1000000n).toString().padStart(20, "0");
    const hash = createHash("sha256").update(body).update(stamp).digest("hex").slice(0, 16);
    const directory = join(this.home, "spool");
    await mkdir(directory, { recursive: true });
    const name = `${stamp}-${hash}.json`;
    const temporary = join(directory, `.${name}.tmp`);
    await writeFile(temporary, body);
    await rename(temporary, join(directory, name));
  }
}
