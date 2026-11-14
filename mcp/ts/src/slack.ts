import { execFile } from "node:child_process";
import { CommunicationError } from "./identity.ts";
import { mkdtemp, writeFile, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
export type SlackRunner = (args: string[]) => Promise<any>;
export function slackRunner(
  binary = process.env.SLACKCLI_PATH || "slackcli",
): SlackRunner {
  return (args) =>
    new Promise((resolve, reject) => {
      execFile(
        binary,
        args,
        {
          shell: false,
          timeout: 45000,
          maxBuffer: 8000000,
          windowsHide: true,
          env: {
            ...process.env,
            SLACKCLI_LOG_LEVEL: "off",
            SLACKCLI_NO_UPDATE_NOTIFIER: "1",
          },
        },
        (err, stdout) => {
          if (err)
            return reject(
              new CommunicationError(
                "PROVIDER_ERROR",
                "Slack CLI request failed; write outcome may be unknown",
              ),
            );
          try {
            resolve(stdout.trim() ? JSON.parse(stdout) : {});
          } catch {
            reject(
              new CommunicationError(
                "PROVIDER_ERROR",
                "Slack CLI returned invalid JSON",
              ),
            );
          }
        },
      );
    });
}
export class Slack {
  private run: SlackRunner;
  constructor(run: SlackRunner = slackRunner()) {
    this.run = run;
  }
  async accounts() {
    const r = await this.run(["auth", "list", "--json"]);
    return (r.workspaces || []).map((w: any) => ({
      account_id: "slack:" + w.profile,
      provider: "slack",
      native: w.profile,
      name: w.workspace_name,
      capabilities: [
        "chat.list",
        "chat.read",
        "chat.search",
        "chat.send",
        "chat.edit",
        "chat.delete",
        "chat.react",
        "chat.attachment",
      ],
    }));
  }
  async call(profile: string, action: string, a: any) {
    if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,99}$/.test(profile))
      throw new CommunicationError(
        "INVALID_ACCOUNT",
        "Invalid workspace profile",
      );
    if (action === "send" && a.files?.length) {
      const dir = await mkdtemp(join(tmpdir(), "mcp-slack-send-"));
      try {
        const args = [
          "messages",
          "send",
          "--workspace=" + profile,
          "--json",
          "--recipient-id=" + a.channel,
          "--message=" + a.text,
        ];
        if (!/^[CDGU][A-Z0-9]+$/.test(a.channel || ""))
          throw new CommunicationError(
            "INVALID_RESOURCE",
            "Invalid Slack conversation",
          );
        if (a.ts) {
          if (!/^[0-9]+[.][0-9]+$/.test(a.ts))
            throw new CommunicationError(
              "INVALID_RESOURCE",
              "Invalid Slack timestamp",
            );
          args.push("--thread-ts=" + a.ts);
        }
        let total = 0;
        for (const [i, f] of a.files.entries()) {
          if (!/^[A-Za-z0-9+/]*={0,2}$/.test(f.data))
            throw new CommunicationError(
              "INVALID_INPUT",
              "Invalid base64 file",
            );
          const data = Buffer.from(f.data, "base64");
          total += data.length;
          if (total > 5000000)
            throw new CommunicationError("LIMIT_EXCEEDED", "Files exceed 5 MB");
          const name = String(f.name)
            .replace(/[^A-Za-z0-9._-]/g, "_")
            .slice(0, 100);
          const path = join(dir, String(i) + "-" + name);
          await writeFile(path, data, { mode: 0o600 });
          args.push("--file=" + path);
        }
        return await this.run(args);
      } finally {
        await rm(dir, { recursive: true, force: true });
      }
    }
    if (action === "attachment") {
      if (!/^F[A-Z0-9]+$/.test(a.file_id || ""))
        throw new CommunicationError("INVALID_RESOURCE", "Invalid Slack file");
      const info = await this.run([
        "files",
        "info",
        a.file_id,
        "--workspace=" + profile,
        "--json",
      ]);
      const size = info.file?.size ?? info.size;
      if (typeof size !== "number" || size > 5000000)
        throw new CommunicationError(
          "LIMIT_EXCEEDED",
          "Attachment size unavailable or exceeds 5 MB",
        );
      const dir = await mkdtemp(join(tmpdir(), "mcp-slack-"));
      try {
        const output = join(dir, "attachment");
        await new Promise<void>((resolve, reject) =>
          execFile(
            process.env.SLACKCLI_PATH || "slackcli",
            [
              "files",
              "download",
              a.file_id,
              "--workspace=" + profile,
              "--output=" + output,
              "--yes",
            ],
            {
              shell: false,
              timeout: 45000,
              maxBuffer: 64000,
              windowsHide: true,
              env: {
                ...process.env,
                SLACKCLI_LOG_LEVEL: "off",
                SLACKCLI_NO_UPDATE_NOTIFIER: "1",
              },
            },
            (err) =>
              err
                ? reject(
                    new CommunicationError(
                      "PROVIDER_ERROR",
                      "Slack attachment download failed",
                    ),
                  )
                : resolve(),
          ),
        );
        if ((await stat(output)).size > 5000000)
          throw new CommunicationError(
            "LIMIT_EXCEEDED",
            "Attachment exceeds 5 MB",
          );
        return {
          data: (await readFile(output)).toString("base64"),
          mime_type:
            info.file?.mimetype || info.mimetype || "application/octet-stream",
        };
      } finally {
        await rm(dir, { recursive: true, force: true });
      }
    }
    let args: string[];
    const channel = () => {
      if (!/^[CDGU][A-Z0-9]+$/.test(a.channel || ""))
        throw new CommunicationError(
          "INVALID_RESOURCE",
          "Invalid Slack conversation",
        );
      return a.channel;
    };
    const ts = () => {
      if (!/^[0-9]+[.][0-9]+$/.test(a.ts || ""))
        throw new CommunicationError(
          "INVALID_RESOURCE",
          "Invalid Slack timestamp",
        );
      return a.ts;
    };
    switch (action) {
      case "get":
        args = ["conversations", "get", channel(), ts()];
        break;
      case "whoami":
        args = ["auth", "whoami"];
        break;
      case "list":
        args = ["conversations", "list", "--limit=" + a.limit];
        if (a.cursor) args.push("--cursor=" + a.cursor);
        break;
      case "read":
        args = ["conversations", "read", channel(), "--limit=" + a.limit];
        if (a.ts) args.push("--thread-ts=" + ts());
        break;
      case "search":
        args = ["search", "messages", "--limit=" + a.limit, "--", a.query];
        break;
      case "send":
        args = [
          "messages",
          "send",
          "--recipient-id=" + channel(),
          "--message=" + a.text,
        ];
        if (a.ts) args.push("--thread-ts=" + ts());
        break;
      case "edit":
        args = [
          "messages",
          "edit",
          "--channel-id=" + channel(),
          "--timestamp=" + ts(),
          "--message=" + a.text,
        ];
        break;
      case "delete":
        args = [
          "messages",
          "delete",
          "--yes",
          "--channel-id=" + channel(),
          "--timestamp=" + ts(),
        ];
        break;
      case "react":
        args = [
          "messages",
          a.remove ? "unreact" : "react",
          "--channel-id=" + channel(),
          "--timestamp=" + ts(),
          "--emoji=" + a.emoji,
        ];
        break;
      default:
        throw new CommunicationError(
          "UNSUPPORTED_CAPABILITY",
          "Slack operation unsupported",
        );
    }
    if (action === "edit" || action === "delete") {
      const identity = await this.call(profile, "whoami", {});
      const original = await this.call(profile, "get", a);
      if (!identity.user_id || original.message?.user !== identity.user_id)
        throw new CommunicationError(
          "OWNERSHIP_REQUIRED",
          "Only own Slack messages can be changed",
        );
    }
    return this.run([
      args[0]!,
      args[1]!,
      "--workspace=" + profile,
      "--json",
      ...args.slice(2),
    ]);
  }
}
