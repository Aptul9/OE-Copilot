// One request to the OE Copilot agent, run over SSH on its host. The agent image is a static binary without shell or node, so nothing runs
// inside the container: bash and curl on the host call the port published on its loopback (127.0.0.1:3180 in the compose file), with the
// agent API_KEY read by `docker inspect` into a 0700 temporary file. The key never leaves the host and never appears in a command line.

const b64 = (text: string) => Buffer.from(text).toString("base64");

export const NAME = /^[A-Za-z0-9_][\w.-]*$/;

/** The remote command: script and request travel as base64, so the remote shell never interprets request data. */
export function hostRequest(container: string, port: number, path: string, body?: unknown): string {
  if (!NAME.test(container) || !Number.isInteger(port) || port < 1 || port > 65535 || !path.startsWith("/"))
    throw new Error("Invalid OE host request");
  const curl = `curl -sg --noproxy '*' --max-time 30 -H @"$d/h" -o "$d/o" -w '%{http_code}'`;
  const script = [
    "set -eu",
    `k=$(docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' ${container} | sed -n 's/^API_KEY=//p' | head -n 1)`,
    '[ -n "$k" ] || exit 2',
    "d=$(mktemp -d); chmod 700 \"$d\"; trap 'rm -rf \"$d\"' EXIT",
    "printf 'Authorization: Bearer %s\\nContent-Type: application/json\\n' \"$k\" > \"$d/h\"",
    `p=$(printf %s '${b64(path)}' | base64 -d); : > "$d/o"`,
    body === undefined
      ? `c=$(${curl} "http://127.0.0.1:${port}$p") || c=000`
      : `c=$(printf %s '${b64(JSON.stringify(body))}' | base64 -d | ${curl} --data-binary @- "http://127.0.0.1:${port}$p") || c=000`,
    "printf '%s %s\\n' \"${c:-000}\" \"$(base64 -w0 < \"$d/o\")\"",
  ].join("\n");
  return `bash -c "$(printf %s '${b64(script)}' | base64 -d)"`;
}

/** The answer of hostRequest: HTTP status and body; null when the output is not that one line. */
export function parseHostReply(stdout: string): { status: number; body: string } | null {
  const m = /^(\d{3}) ([A-Za-z0-9+/=]*)\n?$/.exec(stdout);
  return m ? { status: Number(m[1]), body: Buffer.from(m[2]!, "base64").toString("utf8") } : null;
}
