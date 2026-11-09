import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "@/agent/context";
import { isRecentlyRead, markReadViaApi, saveChatViaApi, servedByApi } from "@/agent/jobs/conversation";
import { log } from "@/agent/log";
import { currentSlot, dropSlot, inSlot, perSlot, slotPrefix } from "@/agent/slot-scope";

const chatsvc = vi.hoisted(() => ({
  getTeamsAuth: vi.fn(async () => ({ token: "t", region: "emea", oid: "o", upn: "", name: "" })),
  resolveConversationId: vi.fn(async () => "19:conv@thread.v2"),
  fetchChatMessages: vi.fn(async () => []),
  markConversationRead: vi.fn(async () => undefined),
}));
vi.mock("@/agent/teams/chatsvc", async (original) => ({ ...(await original<typeof import("@/agent/teams/chatsvc")>()), ...chatsvc }));

afterEach(() => vi.restoreAllMocks());

const agent = () => ({ tp: { page: { context: () => ({}) } }, store: { chats: () => [], messages: () => [], markChatRead: () => undefined, saveChatMessages: () => undefined, readByCache: () => new Map() }, media: {} }) as unknown as Agent;

describe("slot scope", () => {
  it("gives one instance per slot, and slot 0 outside any scope", () => {
    const get = perSlot(() => ({}));
    const outside = get();
    expect(get()).toBe(outside);
    const two = inSlot(2, get);
    expect(inSlot(2, get)).toBe(two);
    expect(inSlot(3, get)).not.toBe(two);
    expect(two).not.toBe(outside);
    expect(get()).toBe(outside);
  });

  it("keeps slots apart when their work interleaves", async () => {
    const get = perSlot(() => new Map<string, number>());
    const work = (slot: number) =>
      inSlot(slot, async () => {
        for (let i = 0; i < 4; i++) {
          get().set(`${slot}-${i}`, i);
          await sleep(slot);
          await Promise.resolve();
        }
        expect(currentSlot()).toBe(slot);
        return [...get().keys()];
      });
    const [a, b] = await Promise.all([work(2), work(3)]);
    expect(a).toEqual(["2-0", "2-1", "2-2", "2-3"]);
    expect(b).toEqual(["3-0", "3-1", "3-2", "3-3"]);
  });

  it("prefixes the slot only inside a scope, also after an await", async () => {
    expect(slotPrefix()).toBe("");
    expect(currentSlot()).toBe(0);
    await inSlot(4, async () => {
      expect(slotPrefix()).toBe("[4] ");
      await sleep(5);
      expect(slotPrefix()).toBe("[4] ");
    });
  });

  it("prints the log lines with the slot in a scope, as they are outside", () => {
    const out = vi.spyOn(console, "log").mockImplementation(() => undefined);
    const err = vi.spyOn(console, "error").mockImplementation(() => undefined);
    inSlot(5, () => {
      log.info("light", "x");
      log.warn("light", "y");
    });
    log.info("light", "x");
    log.warn("light", "y");
    expect(out.mock.calls).toEqual([["[5] light: x"], ["light: x"]]);
    expect(err.mock.calls).toEqual([["[5] light: y"], ["light: y"]]);
  });

  it("does not show the chats one slot saved through the API to another", async () => {
    await inSlot(2, async () => {
      expect(await saveChatViaApi(agent(), "chat")).toBe(true);
      expect(servedByApi("chat")).toBe(true);
    });
    inSlot(3, () => expect(servedByApi("chat")).toBe(false));
    inSlot(2, () => expect(servedByApi("chat")).toBe(true));
  });

  it("forgets the state of a dropped slot: the next account of the slot starts clean", async () => {
    await inSlot(6, async () => {
      expect(await saveChatViaApi(agent(), "chat")).toBe(true);
    });
    inSlot(7, () => expect(servedByApi("chat")).toBe(false));
    await inSlot(7, () => saveChatViaApi(agent(), "chat"));
    dropSlot(6);
    inSlot(6, () => expect(servedByApi("chat")).toBe(false));
    inSlot(7, () => expect(servedByApi("chat")).toBe(true));
  });

  it("does not show the chats one slot read to another", async () => {
    await inSlot(2, async () => {
      await markReadViaApi(agent(), "chat");
      expect(isRecentlyRead("chat", "")).toBe(true);
    });
    inSlot(3, () => expect(isRecentlyRead("chat", "")).toBe(false));
  });
});
