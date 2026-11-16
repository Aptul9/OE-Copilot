// The light host's side of a calling worker in serve mode: answers the first call that rings, hangs up after TALK s.
const { spawn } = require("child_process");
const t0 = Date.now();
const TALK = Number(process.env.TALK ?? 20) * 1000;
const child = spawn(process.env.BIN, ["--mode=serve"], { stdio: ["ignore", "inherit", "inherit", "ipc"], env: process.env });
let last = "", answered = false, hung = false;
const log = (v) => console.log(JSON.stringify({ h: Date.now() - t0, ...v }));
child.on("message", (m) => {
  if (m.call) {
    const s = `${m.call.ringing}/${m.call.active}/${m.call.ended}/${m.call.answered}`;
    if (s !== last) { last = s; log({ call: m.call }); }
    if (m.call.ringing && !answered) { answered = true; setTimeout(() => { log({ send: "answer" }); child.send({ id: 1, op: "answer", since: m.call.since }); }, 1500); }
    if (m.call.active && !hung) { hung = true; setTimeout(() => { log({ send: "hangup" }); child.send({ id: 2, op: "hangup" }); }, TALK); }
    return;
  }
  if (m.refreshToken) child.send({ tokenUpdated: true });
  log({ msg: m });
});
child.on("exit", (c) => { log({ exit: c }); process.exit(0); });
setTimeout(() => child.kill("SIGTERM"), Number(process.env.LIFE ?? 90) * 1000);
