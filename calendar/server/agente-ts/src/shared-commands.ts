import fs from "node:fs";
export type SharedCommand = {
  id: string;
  account: string;
  operation: "remove" | "chat";
  on?: boolean;
  error?: string;
};
export function readCommands(file: string): SharedCommand[] {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8")) as SharedCommand[];
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw e;
  }
}
export function saveCommands(file: string, commands: SharedCommand[]) {
  fs.writeFileSync(file + ".tmp", JSON.stringify(commands));
  fs.renameSync(file + ".tmp", file);
}
