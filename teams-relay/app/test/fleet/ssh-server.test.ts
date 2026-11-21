// The embedded ssh2 server (the no-admin shell into a relay host). Public-key auth only; an exec request runs the
// command through the same runner cmdapi uses. Driven by a real in-process ssh2 client: an authorized key runs a
// command, an unauthorized key is refused.
import net from "node:net";
import { Client } from "ssh2";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { generateEd25519 } from "@/fleet/ssh/keys";
import { startSshServer } from "@/fleet/ssh/server";

const hostKey = generateEd25519();
const good = generateEd25519();
const bad = generateEd25519();

let server: { port: number; close: () => Promise<void> };

beforeAll(async () => {
  server = await startSshServer({ port: 0, host: "127.0.0.1", hostKey: hostKey.private, authorizedKeys: [good.public] });
});

afterAll(async () => {
  await server.close();
});

function exec(privateKey: string, command: string): Promise<{ code: number | null; stdout: string }> {
  return new Promise((resolve, reject) => {
    const conn = new Client();
    conn
      .on("ready", () => {
        conn.exec(command, (err, stream) => {
          if (err) return reject(err);
          let stdout = "";
          let code: number | null = null;
          stream
            .on("data", (d: Buffer) => (stdout += d))
            .on("exit", (c: number) => (code = c))
            .on("close", () => {
              conn.end();
              resolve({ code, stdout });
            });
        });
      })
      .on("error", reject)
      .connect({ host: "127.0.0.1", port: server.port, username: "fleet", privateKey });
  });
}

describe("ssh server", () => {
  it("runs a command for an authorized key", async () => {
    const r = await exec(good.private, "echo sshexectest");
    expect(r.stdout).toContain("sshexectest");
    expect(r.code).toBe(0);
  });

  it("refuses an unauthorized key", async () => {
    await expect(exec(bad.private, "echo nope")).rejects.toThrow();
  });

  it("forwards a direct-tcpip stream to a target host and port", async () => {
    const target = net.createServer((sock) => {
      sock.once("data", (chunk) => sock.end(Buffer.concat([Buffer.from("reply:"), Buffer.from(chunk)])));
    });
    await new Promise<void>((r) => target.listen(0, "127.0.0.1", r));
    const targetPort = (target.address() as net.AddressInfo).port;
    try {
      const reply = await new Promise<string>((resolve, reject) => {
        const conn = new Client();
        conn
          .on("ready", () => {
            conn.forwardOut("127.0.0.1", 0, "localhost", targetPort, (err, stream) => {
              if (err) {
                conn.end();
                return reject(err);
              }
              const chunks: Buffer[] = [];
              stream.on("data", (d: Buffer) => chunks.push(d));
              stream.on("end", () => {
                conn.end();
                resolve(Buffer.concat(chunks).toString("utf8"));
              });
              stream.end("ping");
            });
          })
          .on("error", reject)
          .connect({ host: "127.0.0.1", port: server.port, username: "fleet", privateKey: good.private });
      });
      expect(reply).toBe("reply:ping");
    } finally {
      await new Promise<void>((r) => target.close(() => r()));
    }
  });

  it("rejects direct-tcpip when the target port refuses the connection", async () => {
    const probe = net.createServer();
    await new Promise<void>((r) => probe.listen(0, "127.0.0.1", r));
    const closedPort = (probe.address() as net.AddressInfo).port;
    await new Promise<void>((r) => probe.close(() => r()));

    await expect(
      new Promise<void>((resolve, reject) => {
        const conn = new Client();
        conn
          .on("ready", () => {
            conn.forwardOut("127.0.0.1", 0, "127.0.0.1", closedPort, (err) => {
              conn.end();
              if (err) reject(err);
              else resolve();
            });
          })
          .on("error", reject)
          .connect({ host: "127.0.0.1", port: server.port, username: "fleet", privateKey: good.private });
      }),
    ).rejects.toThrow();
  });
});
