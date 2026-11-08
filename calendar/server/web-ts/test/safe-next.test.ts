import { describe, expect, it } from "vitest";
import { loginUrl, safeNext } from "@/lib/safe-next";

describe("safeNext", () => {
  it("lascia passare i percorsi di questo sito", () => {
    expect(safeNext("/")).toBe("/");
    expect(safeNext("/app.js?x=1#a")).toBe("/app.js?x=1#a");
  });
  it.each(["//evil.example", "/\\evil.example", "/\t/evil.example", "/\n/evil.example", "https://evil.example/", "javascript:alert(1)", "evil", "", null, undefined])(
    "rimanda a / per %j",
    (v) => expect(safeNext(v as string | null | undefined)).toBe("/"),
  );
  it("costruisce l'indirizzo di accesso", () => {
    expect(loginUrl("/a b?c=1")).toBe("/login?next=%2Fa%20b%3Fc%3D1");
  });
});
