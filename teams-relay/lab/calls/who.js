// Name, UPN, oid and tenant of each slot's Skype token (claims only, no token printed)
const fs = require("fs");
for (const n of fs.readdirSync("/root/data").filter((d) => /^\d+$/.test(d)).sort((a, b) => a - b)) {
  try {
    const d = JSON.parse(fs.readFileSync(`/root/data/${n}/discovery.json`, "utf8"));
    const t = d.tokens && d.tokens.skypeToken;
    const c = t ? JSON.parse(Buffer.from(t.split(".")[1], "base64url").toString()) : {};
    console.log(n, c.oid || c.skypeid, c.tid, c.upn || c.unique_name || "", c.name || "", d.region || "");
  } catch (e) {
    console.log(n, "no discovery");
  }
}
