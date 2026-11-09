import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { AccountPanel } from "@/shared/accounts/AccountPanel";
import { ConnectForm } from "@/shared/accounts/ConnectForm";
import { describe, expect, it } from "vitest";
import { buildAccountRows, labelFor, shownSignIn, withSignIns } from "@/shared/accounts/model";
import { SignInProgress, type SignIn } from "@/shared/accounts/ConnectForm";
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
      busy: ["a@b|*"],
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
  expect(html).toContain("pointer-coarse:size-11");
  expect(html).toContain("<svg");
});

describe("service switches, calls option and status", () => {
  it("shows each service as an icon switch with its state in words, not color alone", () => {
    const html = renderToStaticMarkup(
      createElement(AccountPanel, {
        rows: [{ account: "a@b", capabilities: ["mail"], status: "Connected" }],
        onToggle: () => {},
      }),
    );
    expect(html.match(/<svg/g)!.length).toBeGreaterThanOrEqual(5);
    expect(html).toContain('title="Mail: on"');
    expect(html).toContain('title="Calendar: off"');
    expect(html).toContain('title="Teams: off"');
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

describe("one line per account", () => {
  const labels = {
    names: { "me@npo.example": "NPO", "me@bandpey.example": "Bandpey" },
    logos: { NPO: "/logos/npo.png" },
    forwarded: { NPO: "Bandpey" },
  };
  it("names, logos and forwarded calendars come from the calendar labels; rows sort by name", () => {
    const rows = buildAccountRows(
      [{ account: "me@bandpey.example", capabilities: ["mail", "calendar"], status: "ok" }],
      [{ email: "me@npo.example", slot: 1, state: "ok" }],
      true,
      labels,
    );
    expect(rows.map((r) => [r.name, r.logo, r.via])).toEqual([
      ["Bandpey", undefined, undefined],
      ["NPO", "/logos/npo.png", "Bandpey"],
    ]);
    const plain = buildAccountRows([{ account: "z@x", capabilities: [], status: "ok" }], []);
    expect(plain[0].name).toBeUndefined();
  });
  it("renders a single row with logo, name, address and the forwarded calendar marked, without a separate entry", () => {
    const rows = buildAccountRows([], [{ email: "me@npo.example", slot: 1, state: "ok" }], true, labels);
    const html = renderToStaticMarkup(createElement(AccountPanel, { rows, onToggle: () => {}, onReconnect: () => {} }));
    expect(html.match(/data-account=/g)).toHaveLength(1);
    expect(html).toContain('src="/logos/npo.png"');
    expect(html).toContain(">NPO<");
    expect(html).toContain("me@npo.example");
    expect(html).toContain("Calendar: arrives forwarded via Bandpey");
    expect(html).toContain('aria-label="Connect services of NPO"');
  });
  it("shows the inline area only under the expanded row", () => {
    const rows = buildAccountRows(
      [
        { account: "a@x", capabilities: ["mail"], status: "ok" },
        { account: "b@x", capabilities: ["mail"], status: "ok" },
      ],
      [],
    );
    const html = renderToStaticMarkup(
      createElement(AccountPanel, { rows, onToggle: () => {}, expanded: "b@x", inline: createElement("i", null, "FORM") }),
    );
    expect(html.indexOf("FORM")).toBeGreaterThan(html.indexOf('data-account="b@x"'));
    expect(html.match(/FORM/g)).toHaveLength(1);
  });
});

describe("changes do not block the panel", () => {
  const rows = buildAccountRows(
    [
      { account: "a@x", capabilities: ["mail", "chat"], status: "ok", calls: true, state: "ok" },
      { account: "b@x", capabilities: ["mail"], status: "ok" },
    ],
    [],
  );
  const render = (busy: string[]) =>
    renderToStaticMarkup(createElement(AccountPanel, { rows, busy, onToggle: () => {}, onCalls: () => {}, onRemove: () => {} }));
  const button = (html: string, label: string) => html.match(new RegExp('<button[^>]*aria-label="' + label + '"[^>]*>'))![0];
  it("a change on one switch locks that switch only, with a spinner; other switches and accounts stay usable", () => {
    const html = render(["a@x|calendar"]);
    expect(button(html, "Calendar of a@x")).toContain('aria-busy="true"');
    expect(button(html, "Calendar of a@x")).toContain('disabled=""');
    for (const other of ["Mail of a@x", "Teams of a@x", "Calls of a@x", "Mail of b@x", "Calendar of b@x", "Remove a@x", "Remove b@x"])
      expect(button(html, other)).not.toContain('disabled=""');
    expect(html).toContain("Add account");
    expect(html.match(/<button[^>]*disabled=""[^>]*>[^<]*Add account/)).toBeNull();
  });
  it("a removal on its way locks its own row only", () => {
    const html = render(["a@x|*"]);
    for (const own of ["Mail of a@x", "Calendar of a@x", "Teams of a@x", "Calls of a@x", "Remove a@x"])
      expect(button(html, own)).toContain('disabled=""');
    for (const other of ["Mail of b@x", "Calendar of b@x", "Remove b@x"])
      expect(button(html, other)).not.toContain('disabled=""');
  });
});

it("an app's own logo control replaces the plain logo on every row", () => {
  const rows = buildAccountRows([{ account: "a@x", capabilities: ["mail"], status: "ok" }], [], true, { names: { "a@x": "Alfa" } });
  const html = renderToStaticMarkup(
    createElement(AccountPanel, { rows, onToggle: () => {}, renderLogo: (r) => createElement("button", { "data-logo-apri": r.name }, "logo") }),
  );
  expect(html).toContain('data-logo-apri="Alfa"');
  expect(html).not.toContain("ring-black/10");
});

describe("a guided sign-in and its Teams slot", () => {
  const job = (o: Partial<SignIn> = {}): SignIn => ({ id: "j", account: "me@bip.example", capabilities: ["mail", "calendar", "chat"], phase: "browser", slot: 10, ...o });
  const slotRows = () => buildAccountRows([], [{ email: "", slot: 10, state: "login" }, { email: "", slot: 11, state: "login" }]);
  it("folds the slot a sign-in runs in into one row for the address being signed in", () => {
    const rows = withSignIns(slotRows(), [job()], 1000);
    expect(rows.map((r) => r.account)).toEqual(["Teams account 11", "me@bip.example"]);
    expect(rows[1]).toMatchObject({ pending: true, status: "Signing in" });
    expect(rows[1].unavailable?.mail).toMatch(/cancel/i);
  });
  it("keeps the slot hidden for a moment after the sign-in stopped, then shows what is left", () => {
    const stopped = job({ phase: "blocked", ended: 1000 });
    expect(withSignIns(slotRows(), [stopped], 1100).map((r) => r.account)).toEqual(["Teams account 11"]);
    expect(withSignIns(slotRows(), [stopped], 1300).map((r) => r.account)).toEqual(["Teams account 10", "Teams account 11"]);
  });
  it("shows no cancelled or finished sign-in", () => {
    expect(shownSignIn(job({ phase: "blocked", error: "Connection cancelled." }))).toBe(false);
    expect(shownSignIn(job({ phase: "complete" }))).toBe(false);
    expect(shownSignIn(job({ phase: "blocked", error: "Sign-in timed out. Resume when ready." }))).toBe(true);
  });
  it("a row being signed in has nothing to remove or edit; its progress has Cancel, a stopped one Dismiss", () => {
    const rows = withSignIns([], [job()], 1000);
    const html = renderToStaticMarkup(createElement(AccountPanel, { rows, onToggle: () => {}, renderLogo: () => createElement("button", { "data-logo-apri": "x" }) }));
    expect(html).not.toContain("Remove me@bip.example");
    expect(html).not.toContain("data-logo-apri");
    const progress = renderToStaticMarkup(createElement(SignInProgress, { signins: [job(), job({ id: "k", account: "old@bip.example", phase: "blocked", error: "Sign-in timed out. Resume when ready." })], onConnect: () => {}, onCancel: () => {} }));
    expect(progress).toContain("Cancel the sign-in of me@bip.example");
    expect(progress).toContain("Dismiss the sign-in of old@bip.example");
  });
  it("one name and logo per address, for every list of accounts", () => {
    const labels = { names: { "me@npo.example": "NPO" }, logos: { NPO: "data:image/png;base64,AA" } };
    expect(labelFor(" ME@NPO.EXAMPLE ", labels)).toEqual({ name: "NPO", logo: "data:image/png;base64,AA" });
    expect(labelFor("other@x", labels)).toEqual({});
    expect(labelFor("me@npo.example", null)).toEqual({});
  });
});
