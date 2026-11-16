// Calls of the last 15 minutes in every slot's messages.db (calls table), to tell whether one is going on
const D = require("/app/node_modules/better-sqlite3");
const fs = require("fs");
const since = Date.now() - 15 * 60 * 1000;
for (const n of fs.readdirSync("/root/data").filter((d) => /^\d+$/.test(d))) {
  try {
    const db = new D(`/root/data/${n}/messages.db`, { readonly: true });
    const cols = db.prepare("pragma table_info(calls)").all().map((c) => c.name);
    const tcol = cols.find((c) => /since|ts|time|started/i.test(c));
    const rows = tcol ? db.prepare(`select * from calls where ${tcol} > ? order by ${tcol} desc limit 3`).all(since) : [];
    if (rows.length) console.log(n, JSON.stringify(rows).slice(0, 400));
    const st = db.prepare("select v from state where k='call'").get();
    if (st) console.log(n, "state call", String(st.v).slice(0, 200));
  } catch (e) {}
}
console.log("checked");
