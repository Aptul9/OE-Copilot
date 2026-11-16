// Identity of a slot as the agent stored it (state rows whose key or value names the account), argv[2] = slot
const D = require("/app/node_modules/better-sqlite3");
const db = new D(`/root/data/${process.argv[2]}/messages.db`, { readonly: true });
const tables = db.prepare("select name from sqlite_master where type='table'").all().map((t) => t.name);
console.log(tables.join(","));
if (tables.includes("state")) {
  for (const r of db.prepare("select * from state").all()) {
    const s = JSON.stringify(r);
    if (/upn|email|mail|identity|account|me\b|name/i.test(s) && !/token|secret|cookie/i.test(s)) console.log(s.slice(0, 300));
  }
}
