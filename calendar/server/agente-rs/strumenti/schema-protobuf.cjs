// Come protobufjs risolve googlechat.proto: ogni tipo, campo (numero, etichetta, tipo risolto) ed enum, in ordine di dichiarazione. Il
// test `risolve_lo_schema_come_protobufjs` di rete/src/protobuf.rs lo confronta con lo schema letto in Rust. Serve npm ci in agente-ts.
//   node calendar/server/agente-rs/strumenti/schema-protobuf.cjs [uscita]   (predefinita rete/tests/dati/protobuf-schema.txt)
const fs = require("fs");
const path = require("path");
const { createRequire } = require("module");
const ts = path.resolve(__dirname, "../../agente-ts");
const protobuf = createRequire(path.join(ts, "package.json"))("protobufjs");
const root = protobuf.parse(fs.readFileSync(path.join(ts, "proto/googlechat.proto"), "utf8"), { keepCase: true }).root;
root.resolveAll();
const out = [];
const scalari = new Set();
const strani = [];
function walk(ns) {
  for (const n of ns.nestedArray) {
    if (n instanceof protobuf.Type) {
      out.push("M " + n.fullName);
      for (const f of n.fieldsArray) {
        const t = f.resolvedType ? f.resolvedType.fullName : f.type;
        if (!f.resolvedType) scalari.add(f.type);
        if (f.delimited || f.required || f.packed || f.map || (!f.repeated && !f.hasPresence)) strani.push(n.fullName + "." + f.name);
        out.push(`F ${f.name} ${f.id} ${f.repeated ? "R" : "O"} ${t}`);
      }
      walk(n);
    } else if (n instanceof protobuf.Enum) {
      if (n._features.enum_type !== "CLOSED") strani.push(n.fullName + " open");
      out.push("E " + n.fullName + " " + Object.keys(n.values).map((k) => k + "=" + n.values[k]).join(","));
    } else strani.push(n.constructor.name + " " + n.fullName);
  }
}
walk(root);
fs.writeFileSync(process.argv[2] || path.resolve(__dirname, "../rete/tests/dati/protobuf-schema.txt"), out.join("\n") + "\n");
console.log(out.length, "righe; scalari:", [...scalari].sort().join(" "), "; strani:", strani.join(" ") || "nessuno");
