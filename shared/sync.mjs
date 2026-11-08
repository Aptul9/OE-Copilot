import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
for (const app of ["calendar/server/web-ts", "teams-relay/app"]) {
  const dest = path.join(root, app, "src/shared/accounts");
  fs.mkdirSync(dest, { recursive: true });
  for (const file of fs.readdirSync(path.join(root, "shared/accounts"))) {
    const content =
      "// Generated from shared/accounts/" +
      file +
      ". DO NOT EDIT.\n" +
      fs.readFileSync(path.join(root, "shared/accounts", file), "utf8");
    const target = path.join(dest, file);
    if (process.argv.includes("--check")) {
      if (fs.readFileSync(target, "utf8") !== content)
        throw new Error("Shared source out of sync: " + target);
    } else fs.writeFileSync(target, content);
  }
}
