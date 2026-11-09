import { expect, it } from "vitest";
import { windowsBrowserArgs } from "@/local/browser-identity";
it("uses Windows identity only on Linux", () => {
  expect(windowsBrowserArgs("linux")[0]).toContain("Windows NT 10.0; Win64; x64");
  expect(windowsBrowserArgs("win32")).toEqual([]);
  expect(windowsBrowserArgs("darwin")).toEqual([]);
});
