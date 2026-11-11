// I vettori del codec protobuf in Rust (rete/tests/protobuf.rs), registrati da protobufjs con l'encodeChat e il decodeChat esatti di
// agente-ts/src/gchat.ts. Stesso seme, stessi vettori. Serve npm ci in agente-ts. Il numero facoltativo moltiplica le mutazioni casuali:
// una prova più lunga in una cartella a parte, il file del repository resta quello predefinito.
//   node calendar/server/agente-rs/strumenti/vettori-protobuf.cjs [uscita] [giri]   (predefinita rete/tests/dati/protobuf.json)
const fs = require("fs");
const path = require("path");
const { createRequire } = require("module");
const ts = path.resolve(__dirname, "../../agente-ts");
const protobuf = createRequire(path.join(ts, "package.json"))("protobufjs");
const root = protobuf.parse(fs.readFileSync(path.join(ts, "proto/googlechat.proto"), "utf8"), { keepCase: true }).root;

function encodeChat(type, value) {
  const schema = root.lookupType(type);
  for (const key of Object.keys(value)) if (!schema.fields[key]) throw new Error(`${type}: unknown field ${key}`);
  const message = schema.fromObject(value);
  const error = schema.verify(message);
  if (error) throw new Error(`${type}: ${error}`);
  return schema.encode(message).finish();
}
function decodeChat(type, bytes) {
  const schema = root.lookupType(type);
  return schema.toObject(schema.decode(bytes), { longs: String, enums: String, bytes: String });
}

// mulberry32: same vectors on every run
let seme = 0x0e5eed;
function rnd() {
  seme = (seme + 0x6d2b79f5) | 0;
  let t = Math.imul(seme ^ (seme >>> 15), 1 | seme);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const intero = (n) => Math.floor(rnd() * n);
const scegli = (a) => a[intero(a.length)];

const vettori = [];
const codificati = [];
function codifica(tipo, valore, nota) {
  const v = valore === undefined ? undefined : JSON.parse(JSON.stringify(valore));
  const voce = { nota, tipo, codifica: v };
  try {
    voce.hex = Buffer.from(encodeChat(tipo, v)).toString("hex");
    codificati.push([tipo, voce.hex]);
  } catch (e) {
    voce.errore = String(e && e.message);
  }
  vettori.push(voce);
  return voce.hex;
}
function decodifica(tipo, hex, nota) {
  const voce = { nota, tipo, decodifica: hex };
  try {
    voce.json = JSON.stringify(decodeChat(tipo, Buffer.from(hex, "hex")));
  } catch (e) {
    voce.errore = String(e && e.message);
  }
  vettori.push(voce);
}
function grezzo(f) {
  const w = protobuf.Writer.create();
  f(w);
  return Buffer.from(w.finish()).toString("hex");
}
const tag = (id, filo) => (id << 3) | filo;

const tipi = [];
(function tutti(ns) {
  for (const n of ns.nestedArray) if (n instanceof protobuf.Type) { tipi.push(n); tutti(n); }
})(root);
const nome = (t) => t.fullName.slice(1);
for (const t of tipi) t.fieldsArray.forEach((f) => f.resolve());
function trova(pred) {
  for (const t of tipi) for (const f of t.fieldsArray) if (pred(f)) return [nome(t), f];
  return null;
}
const scalare = (tipo, ripetuto) => trova((f) => !f.resolvedType && f.type === tipo && f.repeated === ripetuto);
const diEnum = (ripetuto) => trova((f) => f.resolvedType instanceof protobuf.Enum && f.repeated === ripetuto);
const diMessaggio = (ripetuto) => trova((f) => f.resolvedType instanceof protobuf.Type && f.repeated === ripetuto);

// ---- the requests gchat.ts builds -------------------------------------------------------------------------------------------
const HEADER = { client_type: "WEB", client_version: "2440378181258", client_feature_capabilities: { spam_room_invites_level: "FULLY_SUPPORTED" } };
const space = { space_id: { space_id: "AAAAabc123" } };
const dm = { dm_id: { dm_id: "dm-xyz_9" } };
codifica("GroupId", { space_id: { space_id: "abc" } }, "gchat.test.ts");
const rh = codifica("RequestHeader", { client_type: "WEB", client_version: "9007199254740993" }, "gchat.test.ts");
decodifica("RequestHeader", rh, "gchat.test.ts: round trip of an int64 above 2^53");
codifica("GetSelfUserStatusRequest", { request_header: HEADER }, "request");
codifica("GetMembersRequest", { member_ids: [{ user_id: { id: "111" } }, { user_id: { id: "222" } }], request_header: HEADER }, "request");
codifica("PaginatedWorldRequest", { fetch_from_user_spaces: true, fetch_options: ["EXCLUDE_GROUP_LITE"], world_section_requests: [{ page_size: 200 }], request_header: HEADER }, "request");
for (const g of [space, dm]) {
  codifica("ListTopicsRequest", { group_id: g, page_size_for_topics: 25, page_size_for_unread_replies: 100, page_size_for_read_replies: 20, request_header: HEADER }, "request");
  codifica("GetGroupRequest", { group_id: g, fetch_options: ["MEMBERS", "MEMBER_IDS_ONLY"], request_header: HEADER }, "request");
  const upload = { type: "UPLOAD_METADATA", upload_metadata: { attachment_token: "tok-1", content_name: "a.pdf", content_type: "application/pdf" } };
  codifica("CreateTopicRequest", { local_id: "oe%0011223344556677", text_body: "Caffè ☕ ciao", annotations: [upload], message_info: { accept_format_annotations: true }, group_id: g, history_v2: true, request_header: HEADER }, "request");
  const parent = { topic_id: { group_id: g, topic_id: "topic-1" } };
  codifica("CreateMessageRequest", { local_id: "oe%aa", text_body: "", annotations: [], message_info: { accept_format_annotations: true }, parent_id: parent, request_header: HEADER }, "request");
  codifica("MarkGroupReadstateRequest", { id: g, last_read_time: "1790000000000000", request_header: HEADER }, "request");
  const mid = { parent_id: parent, message_id: "msg-1" };
  codifica("DeleteMessageRequest", { message_id: mid, request_header: HEADER }, "request");
  codifica("EditMessageRequest", { message_id: mid, text_body: "nuovo", message_info: { accept_format_annotations: true }, request_header: HEADER }, "request");
  for (const type of ["ADD", "REMOVE"]) codifica("UpdateReactionRequest", { message_id: mid, emoji: { unicode: "👍" }, type, request_header: HEADER }, "request");
}
codifica("MarkGroupReadstateRequest", { id: space, last_read_time: "-1", request_header: HEADER }, "negative int64");
codifica("MarkGroupReadstateRequest", { id: space, last_read_time: "18446744073709551615" }, "int64 wraps");

// ---- random values for every type: encode, then decode what was encoded ---------------------------------------------------------
const TESTI = ["", "ciao", "Caffè ☕", "a\nb\t\"c\"", "😀 emoji", "日本語テキスト", "\u0000nul", "x".repeat(70), "<b>&amp;</b>"];
function valoreCasuale(f, p) {
  const t = f.resolvedType;
  if (t instanceof protobuf.Type) return p >= 3 ? undefined : casuale(t, p + 1);
  if (t instanceof protobuf.Enum) {
    const nomi = Object.keys(t.values);
    return rnd() < 0.75 ? scegli(nomi) : t.values[scegli(nomi)];
  }
  switch (f.type) {
    case "string": return scegli(TESTI);
    case "bool": return rnd() < 0.5;
    case "int32": return scegli([0, 1, -1, 127, 128, 300, 2147483647, -2147483648, intero(100000), -intero(1000)]);
    case "int64": return scegli(["0", "1", "-1", "1790000000000000", "9007199254740993", "-9223372036854775808", "9223372036854775807", String(intero(1e9)), intero(1e6)]);
    case "double": return scegli([0, 1.5, -2.25, 1e21, 5e-324, 1.7976931348623157e308, 123456789.123, 0.1, 0.30000000000000004, -1e-7]);
    case "bytes": return Buffer.from(Array.from({ length: intero(12) }, () => intero(256))).toString("base64");
  }
  throw new Error("tipo " + f.type);
}
function casuale(t, p = 0) {
  const o = {};
  const presenza = p === 0 ? 0.6 : p === 1 ? 0.4 : 0.25;
  for (const f of t.fieldsArray) {
    if (rnd() > presenza) continue;
    if (f.repeated) {
      const v = Array.from({ length: intero(3) }, () => valoreCasuale(f, p)).filter((x) => x !== undefined);
      if (v.length || rnd() < 0.3) o[f.name] = v;
    } else {
      const v = valoreCasuale(f, p);
      if (v !== undefined) o[f.name] = v;
    }
  }
  return o;
}
const RISPOSTE = ["GetSelfUserStatusResponse", "GetMembersResponse", "PaginatedWorldResponse", "ListTopicsResponse", "GetGroupResponse", "CreateTopicResponse", "CreateMessageResponse", "UploadMetadata", "MarkGroupReadstateResponse", "EditMessageResponse", "DeleteMessageResponse", "UpdateReactionResponse"];
for (const tipo of RISPOSTE) {
  for (let i = 0; i < 6; ++i) {
    const hex = codifica(tipo, casuale(root.lookupType(tipo)), "random response");
    if (hex !== undefined) decodifica(tipo, hex, "random response round trip");
  }
}
for (const t of tipi) {
  const hex = codifica(nome(t), casuale(t, 1), "random value of every type");
  if (hex !== undefined && hex.length) decodifica(nome(t), hex, "round trip of every type");
}

// ---- fromObject with JavaScript coercions -----------------------------------------------------------------------------------
const STRANI = {
  string: [5, 1.5, true, false, [1, [2, null]], {}, "x", ""],
  int32: ["12", "0x1f", "1e3", "abc", " 7 ", 2.9, -2.9, 4294967297, -4294967297, 2147483648, true, [7], [], {}, 1e21],
  int64: ["0", " 12", "+5", "1e3", "abc", "12abc345678901", "", "1-2", "--5", "-", "NaN", "Infinity", "-Infinity", "0x10", "99999999999999999999999", "18446744073709551615", "-9223372036854775809", 1.5, -2.7, 1e20, -1e20, 9007199254740993, true, [], [5], { low: 1, high: 2 }, { low: -1, high: -1 }, { low: "3" }, {}],
  bool: [0, 1, "", "false", [], {}, 2],
  double: ["1.5", "abc", true, [], [2], [1, 2], "Infinity", "-1e400", {}, " 3 "],
  bytes: ["QQ", "QQ==", "QUJD", "Q", "Q===", "QQ=", "-_-_", "a b", "", "ü", "QUJDRA", [1, 2, 300, -1, "7", null, {}], [], { length: 2, 0: 65 }, { length: "2" }, { length: -1 }, { length: 2.7, 1: 300 }, 5, true, {}],
};
for (const [tipoScalare, strani] of Object.entries(STRANI)) {
  for (const ripetuto of [false, true]) {
    const dove = scalare(tipoScalare, ripetuto);
    if (!dove) continue;
    const [m, f] = dove;
    for (const v of strani) {
      codifica(m, { [f.name]: ripetuto ? [v] : v }, `coercion ${tipoScalare}${ripetuto ? "[]" : ""}`);
      if (ripetuto) codifica(m, { [f.name]: v }, `coercion ${tipoScalare}[] not an array`);
    }
    codifica(m, { [f.name]: ripetuto ? [null] : null }, `null ${tipoScalare}`);
  }
}
for (const ripetuto of [false, true]) {
  const e = diEnum(ripetuto);
  const nomi = Object.keys(e[1].resolvedType.values);
  for (const v of [nomi[0], nomi[nomi.length - 1], e[1].resolvedType.values[nomi[1] ?? nomi[0]], String(e[1].resolvedType.values[nomi[0]]), 99, -1, "NOPE", null, true, 1.5, [], {}]) {
    codifica(e[0], { [e[1].name]: ripetuto ? [v, nomi[0], 99] : v }, `enum${ripetuto ? "[]" : ""}`);
  }
  if (ripetuto) for (const v of [nomi[0], 0, "", 7, {}]) codifica(e[0], { [e[1].name]: v }, "enum[] not an array");
  const m = diMessaggio(ripetuto);
  for (const v of ["x", 5, true, false, 0, "", [], [{}], {}, null]) {
    codifica(m[0], { [m[1].name]: ripetuto ? [v] : v }, `message${ripetuto ? "[]" : ""}`);
    if (ripetuto) codifica(m[0], { [m[1].name]: v }, "message[] not an array");
  }
}
// a message that has a field named length, given as an array
const conLength = trova((f) => f.resolvedType instanceof protobuf.Type && !f.repeated && f.resolvedType.fields.length);
if (conLength) codifica(conLength[0], { [conLength[1].name]: [1, 2, 3] }, "array as a message with a length field");
codifica("UserId", JSON.parse('{"constructor":1,"__proto__":2,"toString":3,"id":"a"}'), "inherited names are not unknown fields");
codifica("UserId", { id: "a", nope: 1 }, "unknown top-level field");
codifica("User", { user_id: { id: "a", nope: 1 } }, "unknown nested field is ignored");
for (const v of [[], [1], "", "ab", 5, true, null]) codifica("UserId", v, "top-level value that is not an object");
codifica("NoSuchType", {}, "unknown type");
codifica(".GroupId", { space_id: { space_id: "abc" } }, "absolute type name");
codifica("", {}, "empty type name");
codifica(".", {}, "root as type name");
// nested types by short and relative names, and nested enums
for (const t of tipi.filter((t) => t.parent !== root).slice(0, 25)) {
  codifica(t.name, {}, "nested type by short name");
  codifica(t.parent.name + "." + t.name, {}, "nested type by relative name");
}
for (const e of root.nestedArray.filter((n) => n instanceof protobuf.Enum).slice(0, 3)) codifica(e.name, {}, "enum is not a type");

// ---- hand-made wire input -------------------------------------------------------------------------------------------------------
const tutti5 = grezzo((w) => {
  w.uint32(tag(15, 0)).uint32(150);
  w.uint32(tag(15, 1)).fixed64(7);
  w.uint32(tag(15, 2)).string("abc");
  w.uint32(tag(15, 3)).uint32(tag(1, 0)).uint32(1).uint32(tag(2, 2)).string("in").uint32(tag(15, 4));
  w.uint32(tag(15, 5)).fixed32(9);
  w.uint32(tag(1, 2)).string("x");
});
decodifica("UserId", tutti5, "unknown fields of every wire type");
for (const filo of [4, 6, 7]) decodifica("UserId", grezzo((w) => w.uint32(tag(15, filo)).uint32(1)), `wire type ${filo}`);
decodifica("UserId", grezzo((w) => w.uint32(tag(15, 3)).uint32(tag(1, 0)).uint32(1).uint32(tag(14, 4))), "group closed by another field");
decodifica("UserId", grezzo((w) => w.uint32(tag(15, 3)).uint32(tag(1, 0)).uint32(1)), "group never closed");
decodifica("UserId", "0000", "field number 0");
decodifica("UserId", "0200", "field number 0, wire 2");
decodifica("UserId", "0a05616263", "string past the end");
decodifica("UserId", "0a", "length missing");
decodifica("UserId", "8080808010", "tag with bits above 32");
decodifica("UserId", "8080808070", "tag with high bits in the fifth byte");
decodifica("UserId", "808080", "tag cut near the end");
decodifica("UserId", "0a8080808008", "length with fifth byte above 7");
decodifica("UserId", "0a8080808007", "length 2^31-ish");
decodifica("UserId", "", "empty buffer");
decodifica("UserId", "10ffffffffffffffffff01", "enum varint of ten bytes (-1, unknown)");
decodifica("UserId", "1001100010ff01", "enum set, unknown value kept out");
decodifica("UserId", "0a0161" + "0a0162", "duplicate string: last wins");
decodifica("UserId", "08011001", "known field with the wrong wire type");
decodifica("User", grezzo((w) => w.uint32(tag(1, 2)).bytes(Buffer.from("0a0161", "hex")).uint32(tag(1, 2)).bytes(Buffer.from("1001", "hex"))), "duplicate message field merges");
decodifica("User", grezzo((w) => w.uint32(tag(1, 0)).uint32(3).uint32(tag(2, 0)).uint32(4)), "message and string fields as varints");
decodifica("User", "0a03" + "0a0161" + "ff", "trailing garbage");
decodifica("User", "0a05" + "0a0161", "nested length past the end");
decodifica("User", "0a02" + "0a0161", "nested length cuts a field");
for (const s of ["ff", "c080", "e282", "eda080", "f09f98", "f09f9880", "f4908080", "61ff62", "e2828261"]) decodifica("UserId", "0a" + (s.length / 2).toString(16).padStart(2, "0") + s, "invalid utf-8 " + s);
decodifica("ListTopicsResponse", Buffer.from("CgUKA2FiYw==", "ascii").toString("hex"), "base64 text read as protobuf");
for (const n of [99, 101, 102]) {
  let h = "";
  for (let i = 0; i < n; ++i) h = "7b" + h + "7c";
  decodifica("UserId", h, `groups nested ${n} deep`);
}
const vuoto = tipi.find((t) => t.fieldsArray.length === 0);
if (vuoto) {
  decodifica(nome(vuoto), "0801" + "1203616263", "message without fields skips everything");
  decodifica(nome(vuoto), "0b0801" + "0c", "group in a message without fields");
  decodifica(nome(vuoto), "0208", "raw tag 2 in a message without fields");
  decodifica(nome(vuoto), "0000", "tag 0 in a message without fields");
  codifica(nome(vuoto), { anything: 1 }, "unknown field of a message without fields");
  codifica(nome(vuoto), {}, "message without fields");
}
// repeated fields packed and unpacked
for (const [cerca, valori] of [
  [() => diEnum(true), (f) => [Object.values(f.resolvedType.values)[0], 99, Object.values(f.resolvedType.values).at(-1), -5]],
  [() => scalare("int32", true), () => [1, -1, 300, 0]],
  [() => scalare("int64", true), () => [1, -1, 300, 0]],
  [() => scalare("bool", true), () => [1, 0, 2, 0]],
  [() => scalare("double", true), () => [1.5, -2]],
]) {
  const dove = cerca();
  if (!dove) continue;
  const [m, f] = dove;
  const v = valori(f);
  const corpo = (w) => { for (const x of v) f.type === "double" ? w.double(x) : f.type === "int64" ? w.int64(x) : f.type === "bool" ? w.uint32(x) : w.int32(x); };
  decodifica(m, grezzo((w) => { w.uint32(tag(f.id, 2)).fork(); corpo(w); w.ldelim(); }), `packed ${f.type}`);
  decodifica(m, grezzo((w) => { for (const x of v) { w.uint32(tag(f.id, f.type === "double" ? 1 : 0)); f.type === "double" ? w.double(x) : f.type === "int64" ? w.int64(x) : w.int32(x); } w.uint32(tag(f.id, 2)).fork(); corpo(w); w.ldelim(); }), `unpacked then packed ${f.type}`);
  decodifica(m, grezzo((w) => w.uint32(tag(f.id, 2)).uint32(9).uint32(1)), `packed ${f.type} past the end`);
  decodifica(m, grezzo((w) => w.uint32(tag(f.id, 2)).uint32(3).uint32(1).uint32(2).uint32(3)), `packed ${f.type} of three bytes`);
  decodifica(m, grezzo((w) => w.uint32(tag(f.id, 2)).uint32(2).uint32(300)), `packed ${f.type} with a two-byte varint`);
  decodifica(m, grezzo((w) => w.uint32(tag(f.id, 2)).uint32(1).uint32(0x80).uint32(1)), `packed ${f.type} with a varint across its end`);
}
for (const [tipoScalare, voci] of [
  ["int32", ["08ffffffffffffffffff01", "08ffffffffff01", "08ffffffff0f", "0880808080808080808001", "08808080808080808080808001", "08ffff"]],
  ["int64", ["08ffffffffffffffffff01", "08ffffffffffffffff7f", "0880808080808080808001", "0880808080808080808080", "08ffffffff", "0801"]],
  ["bool", ["080180", "088000", "0880808080808080808001", "0880808080808080808080", "0802"]],
  ["double", ["09000000000000f87f", "090100000000000000", "09000000000000f0ff", "090000000000000080", "0900000000"]],
]) {
  const dove = scalare(tipoScalare, false);
  if (!dove) continue;
  const [m, f] = dove;
  for (const v of voci) decodifica(m, grezzo((w) => w.uint32(tag(f.id, parseInt(v.slice(0, 2), 16) & 7))) + v.slice(2), `${tipoScalare} on the wire ${v}`);
}

// ---- mutations of everything encoded so far, and the same bytes read as another type -------------------------------------------
const pieni = codificati.filter(([, h]) => h.length >= 4);
const giri = Number(process.argv[3] || 1);
for (let i = 0; i < 500 * giri; ++i) {
  const [tipo, hex] = scegli(pieni);
  const b = Buffer.from(hex, "hex");
  let out;
  switch (intero(5)) {
    case 0: out = b.subarray(0, intero(b.length)); break;
    case 1: out = Buffer.from(b); for (let k = 0; k <= intero(3); ++k) out[intero(out.length)] = intero(256); break;
    case 2: { const p = intero(b.length + 1); out = Buffer.concat([b.subarray(0, p), Buffer.from(Array.from({ length: 1 + intero(3) }, () => intero(256))), b.subarray(p)]); break; }
    case 3: out = Buffer.concat([b, Buffer.from(Array.from({ length: 1 + intero(6) }, () => intero(256)))]); break;
    default: { out = Buffer.from(b); const p = intero(out.length); out[p] = out[p] ^ (1 << intero(8)); }
  }
  decodifica(tipo, out.toString("hex"), "mutation");
}
for (let i = 0; i < 300 * giri; ++i) {
  const [, hex] = scegli(pieni);
  decodifica(nome(scegli(tipi)), hex, "read as another type");
}

const testo = "[\n" + vettori.map((v) => JSON.stringify(v)).join(",\n") + "\n]\n";
fs.writeFileSync(process.argv[2] || path.resolve(__dirname, "../rete/tests/dati/protobuf.json"), testo);
const errori = vettori.filter((v) => v.errore).length;
console.log(vettori.length, "vettori,", errori, "con errore,", testo.length, "byte");
