// Messages between the supervisor and the light host (node light.cjs --host) on the IPC channel of fork
export type ToHost = { op: "start"; slot: number; env: Record<string, string> } | { op: "stop"; slot: number };
export type FromHost = { slot: number; event: "started" } | { slot: number; event: "exited"; how: string } | { slot: number; event: "stopped" };
export const HOST_FLAG = "--host";
