// Chats of a slot whose name matches argv[3] (columns first), to find the name a 'call' command takes as arg1
const D = require("/app/node_modules/better-sqlite3");
const db = new D(`/root/data/${process.argv[2]}/messages.db`, { readonly: true });
console.log(db.prepare("pragma table_info(chats)").all().map((c) => c.name).join(","));
const re = new RegExp(process.argv[3] || ".", "i");
for (const r of db.prepare("select * from chats").all()) {
  const s = JSON.stringify(r);
  if (re.test(s)) console.log(s.slice(0, 300));
}
console.log(db.prepare("pragma table_info(commands)").all().map((c) => c.name).join(","));
for (const r of db.prepare("select * from commands order by rowid desc limit 5").all()) console.log(JSON.stringify(r).slice(0, 200));
