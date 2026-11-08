import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { AccountPanel } from "@/shared/accounts/AccountPanel";
import { ConnectForm } from "@/shared/accounts/ConnectForm";
import { describe, expect, it } from "vitest";
import { buildAccountRows } from "@/shared/accounts/model";
it("guided form offers services and browser progress without device-code or credential copy instructions", () => {
  const html = renderToStaticMarkup(createElement(ConnectForm, { onConnect: () => {}, signins: [{ id: "j", account: "a@b", capabilities: ["mail"], phase: "browser", url: "https://teams.example/api/desktop/3" }] }));
  expect(html).toContain('type="email"');
  expect(html).toContain("Open Microsoft sign-in");
  expect(html).toContain("Teams");
  expect(html).not.toContain("device code");
});
describe("shared account identities", () => {
  it("merges case-folded mailbox and browser identity, preserving browser Teams control", () => {
    const rows = buildAccountRows(
      [{ account: " USER@EXAMPLE.COM ", capabilities: ["mail"], status: "ok" }],
      [{ email: "user@example.com", slot: 3, state: "ok" }],
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].capabilities).toEqual(["mail", "chat"]);
    expect(rows[0].unavailable?.chat).toBeUndefined();
  });
  it("deduplicates mailbox capabilities", () => {
    const rows = buildAccountRows(
      [
        { account: "a@b", capabilities: ["mail"], status: "ok" },
        { account: "A@B", capabilities: ["calendar"], status: "ok" },
      ],
      [],
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].capabilities).toEqual(["mail", "calendar"]);
  });
  it("skips empty mailbox identity and keeps unnamed browser slots distinct", () => {
    expect(
      buildAccountRows(
        [{ account: " ", capabilities: [], status: "ok" }],
        [
          { email: "", slot: 1, state: "stopped" },
          { email: "", slot: 2, state: "ok" },
        ],
      ),
    ).toHaveLength(2);
  });
  it("shows stopped browser without active Teams and blocks mail/calendar", () => {
    const [row] = buildAccountRows(
      [],
      [{ email: "a@b", slot: 2, state: "stopped" }],
    );
    expect(row.capabilities).toEqual([]);
    expect(Object.keys(row.unavailable!)).toEqual(["mail", "calendar"]);
  });
});

it("renders three accessible switches with locked browser capability", () => {
  const html = renderToStaticMarkup(
    createElement(AccountPanel, {
      rows: buildAccountRows([], [{ email: "a@b", slot: 1, state: "ok" }]),
      onToggle: () => {},
    }),
  );
  expect(html.match(/role="switch"/g)).toHaveLength(3);
  expect(html.match(/disabled=""/g)).toHaveLength(2);
  expect(html).toContain('aria-checked="true"');
});

it("unknown report preserves configured Teams on, keeps off usable", () => {
  const [row] = buildAccountRows(
    [{ account: "a@b", capabilities: ["chat"], status: "ok" }],
    [],
    false,
  );
  expect(row.capabilities).toEqual(["chat"]);
  expect(row.status).toContain("unknown");
  expect(row.unavailable?.chat).toBeUndefined();
});
it("missing report blocks only enabling Teams", () => {
  const [row] = buildAccountRows(
    [{ account: "a@b", capabilities: ["mail"], status: "ok" }],
    [],
    false,
  );
  expect(row.unavailable?.chat).toContain("unavailable");
  expect(row.unavailable?.mail).toBeUndefined();
});

it("renders common add, remove and loading/error states", () => {
  const html = renderToStaticMarkup(
    createElement(AccountPanel, {
      rows: buildAccountRows(
        [{ account: "a@b", capabilities: ["mail"], status: "ok" }],
        [],
      ),
      onToggle: () => {},
      loading: true,
      error: "Failed",
      pending: "a@b",
    }),
  );
  for (const text of [
    "Add account",
    "Remove account",
    "Loading accounts",
    "Failed",
    "aria-busy",
  ])
    expect(html).toContain(text);
});

it("renders accessible trash target", () => {
const html=renderToStaticMarkup(createElement(AccountPanel,{rows:[{account:"a@b",capabilities:[],status:"Connected"}],onToggle:()=>{}}));
expect(html).toContain('aria-label="Remove a@b"');
expect(html).toContain('title="Remove account"');
expect(html).toContain('min-w-11');
expect(html).toContain('<svg');
});
