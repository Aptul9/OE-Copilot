const D = require("/app/node_modules/better-sqlite3");
const db = new D("/root/data/app.db", { readonly: true });
const cols = db.prepare("pragma table_info(teams_accounts)").all().map((c) => c.name);
console.log(cols.join(","));
const want = cols.filter((c) => /slot|name|email|upn|user|tenant|calls|light|relay|label/i.test(c));
for (const r of db.prepare(`select ${want.join(",")} from teams_accounts order by slot`).all()) console.log(JSON.stringify(r));
