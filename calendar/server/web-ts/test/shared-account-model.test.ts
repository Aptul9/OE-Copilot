import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { AccountPanel } from "@/shared/accounts/AccountPanel";
import { ConnectForm } from "@/shared/accounts/ConnectForm";
import { describe, expect, it } from "vitest";
import { buildAccountRows } from "@/shared/accounts/model";
it("guided form offers services and browser progress without device-code or credential copy instructions", () => {
  const html = renderToStaticMarkup(
    createElement(ConnectForm, {
      onConnect: () => {},
      signins: [
        {
          id: "j",
          account: "a@b",
          capabilities: ["mail"],
          phase: "browser",
          url: "https://teams.example/api/desktop/3",
        },
      ],
    }),
  );
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
  const html = renderToStaticMarkup(
    createElement(AccountPanel, {
      rows: [{ account: "a@b", capabilities: [], status: "Connected" }],
      onToggle: () => {},
    }),
  );
  expect(html).toContain('aria-label="Remove a@b"');
  expect(html).toContain('title="Remove account"');
  expect(html).toContain("min-w-11");
  expect(html).toContain("<svg");
});

describe("service switches, calls option and status", () => {
  it("shows an icon, a label and an On/Off word for each service, not color alone", () => {
    const html = renderToStaticMarkup(
      createElement(AccountPanel, {
        rows: [{ account: "a@b", capabilities: ["mail"], status: "Connected" }],
        onToggle: () => {},
      }),
    );
    expect(html.match(/<svg/g)!.length).toBeGreaterThanOrEqual(5);
    expect(html).toContain(">On<");
    expect(html.match(/>Off</g)).toHaveLength(2);
  });
  it("offers the calls switch only for a Teams account that reports one, and only with Teams on", () => {
    const render = (row: object, onCalls?: () => void) =>
      renderToStaticMarkup(
        createElement(AccountPanel, {
          rows: [
            {
              account: "a@b",
              capabilities: ["chat"],
              status: "Teams: ok",
              ...row,
            },
          ],
          onToggle: () => {},
          onCalls,
        }),
      );
    const withCalls = render({ calls: true }, () => {});
    expect(withCalls.match(/role="switch"/g)).toHaveLength(4);
    expect(withCalls).toContain('aria-label="Calls of a@b"');
    expect(withCalls).toContain("Rings and is answered in the app.");
    const off = render({ calls: false }, () => {});
    expect(off).toContain("Messages only");
    expect(off).toContain("AI (MCP)");
    expect(render({}, () => {}).match(/role="switch"/g)).toHaveLength(3);
    expect(render({ calls: true }).match(/role="switch"/g)).toHaveLength(3);
    expect(render({ calls: true, capabilities: [] }, () => {})).not.toContain(
      "Calls of",
    );
  });
  it("carries the calls state of the Teams account into the row", () => {
    const [shared] = buildAccountRows(
      [
        {
          account: "a@b",
          capabilities: ["chat"],
          status: "ok",
          state: "ok",
          calls: false,
        },
      ],
      [],
    );
    expect(shared.calls).toBe(false);
    const [mailbox] = buildAccountRows(
      [{ account: "a@b", capabilities: ["mail"], status: "ok", calls: null }],
      [],
    );
    expect(mailbox.calls).toBeUndefined();
    const [browser] = buildAccountRows(
      [],
      [{ email: "c@d", slot: 2, state: "ok", calls: true }],
    );
    expect(browser.calls).toBe(true);
    const [stale] = buildAccountRows(
      [{ account: "a@b", capabilities: ["chat"], status: "ok", calls: true }],
      [],
      false,
    );
    expect(stale.calls).toBeUndefined();
  });
});
