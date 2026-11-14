import assert from "node:assert/strict";
import { test } from "node:test";
import { hostRequest, parseHostReply } from "../src/oe-host.ts";

test("the host command carries script and request only as base64", () => {
  const c = hostRequest("oe-copilot-agent", 3180, "/api/communication/read", { account: "a@x.example", path: "x'; rm -rf / #" });
  assert.match(c, /^bash -c "\$\(printf %s '[A-Za-z0-9+/=]+' \| base64 -d\)"$/);
  assert.doesNotMatch(c, /rm -rf|API_KEY|node/);
  const script = Buffer.from(/'([A-Za-z0-9+/=]+)'/.exec(c)![1]!, "base64").toString();
  assert.match(script, /docker inspect .* oe-copilot-agent/);
  assert.match(script, /http:\/\/127\.0\.0\.1:3180\$p/);
  assert.doesNotMatch(script, /docker exec/);
  // GET without a body sends no data
  assert.doesNotMatch(Buffer.from(/'([A-Za-z0-9+/=]+)'/.exec(hostRequest("c", 3180, "/api/token-posta"))![1]!, "base64").toString(), /data-binary/);
});

test("host request arguments are checked", () => {
  assert.throws(() => hostRequest("c;whoami", 3180, "/x"));
  assert.throws(() => hostRequest("c", 0, "/x"));
  assert.throws(() => hostRequest("c", 3180, "x"));
});

test("the reply is one line: status and base64 body", () => {
  assert.deepEqual(parseHostReply(`200 ${Buffer.from('{"a":1}').toString("base64")}\n`), { status: 200, body: '{"a":1}' });
  assert.deepEqual(parseHostReply("000 \n"), { status: 0, body: "" });
  assert.equal(parseHostReply("{}"), null);
  assert.equal(parseHostReply("200 e30=\n200 e30=\n"), null);
});
