const D = require("/app/node_modules/better-sqlite3");
const db = new D("/root/data/app.db");
const slot = Number(process.argv[2]);
const on = process.argv[3] === "1" ? 1 : 0;
db.prepare("update teams_accounts set calls=? where slot=?").run(on, slot);
console.log("slot", slot, "calls", db.prepare("select calls from teams_accounts where slot=?").get(slot).calls);
