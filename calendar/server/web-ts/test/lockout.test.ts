import { describe, expect, it } from "vitest";
import { openDb } from "@/lib/db";
import { clearFailures, lockedFor, LOCKOUT_MINUTES, MAX_FAILED_LOGINS, migrateLockout, registerFailure } from "@/lib/lockout";

function fresh() {
  const db = openDb(":memory:");
  migrateLockout(db);
  return db;
}
const T0 = 1_800_000_000;

describe("blocco per account", () => {
  it("scatta al quinto errore e dura un quarto d'ora", () => {
    const db = fresh();
    for (let i = 1; i < MAX_FAILED_LOGINS; i++) {
      registerFailure(db, "a@x.it", T0 + i);
      expect(lockedFor(db, "a@x.it", T0 + i)).toBe(0);
    }
    registerFailure(db, "a@x.it", T0 + 5);
    expect(lockedFor(db, "a@x.it", T0 + 5)).toBe(LOCKOUT_MINUTES * 60);
    expect(lockedFor(db, "a@x.it", T0 + 5 + LOCKOUT_MINUTES * 60)).toBe(0);
  });

  it("conta per account, non per indirizzo", () => {
    const db = fresh();
    for (let i = 0; i < MAX_FAILED_LOGINS; i++) registerFailure(db, "a@x.it", T0);
    expect(lockedFor(db, "a@x.it", T0 + 1)).toBeGreaterThan(0);
    expect(lockedFor(db, "b@x.it", T0 + 1)).toBe(0);
  });

  it("dopo il blocco il conto riparte: un solo errore non blocca di nuovo", () => {
    const db = fresh();
    for (let i = 0; i < MAX_FAILED_LOGINS; i++) registerFailure(db, "a@x.it", T0);
    const dopo = T0 + LOCKOUT_MINUTES * 60 + 1;
    registerFailure(db, "a@x.it", dopo);
    expect(lockedFor(db, "a@x.it", dopo)).toBe(0);
    for (let i = 1; i < MAX_FAILED_LOGINS - 1; i++) registerFailure(db, "a@x.it", dopo + i);
    expect(lockedFor(db, "a@x.it", dopo + 10)).toBe(0);
  });

  it("errori distanti più di un quarto d'ora non si sommano", () => {
    const db = fresh();
    for (let i = 0; i < MAX_FAILED_LOGINS - 1; i++) registerFailure(db, "a@x.it", T0 + i);
    registerFailure(db, "a@x.it", T0 + LOCKOUT_MINUTES * 60 + 60);
    expect(lockedFor(db, "a@x.it", T0 + LOCKOUT_MINUTES * 60 + 61)).toBe(0);
  });

  it("un accesso riuscito azzera il conto", () => {
    const db = fresh();
    for (let i = 0; i < MAX_FAILED_LOGINS - 1; i++) registerFailure(db, "a@x.it", T0);
    clearFailures(db, "a@x.it");
    registerFailure(db, "a@x.it", T0 + 1);
    expect(lockedFor(db, "a@x.it", T0 + 2)).toBe(0);
  });
});
