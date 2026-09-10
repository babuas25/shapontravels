"use strict";
// Session token lives only in this tab's memory; never in persistent browser storage.
let token = "",
  selected = null,
  cursor = null,
  busy = false;
const el = (id) => document.getElementById(id);
const labels = {
  pending: "ফল আসার অপেক্ষায়",
  outcome_unknown: "ফল নিশ্চিত নয়",
  manually_resolved: "সমাধান নথিভুক্ত",
  held: "Hold আছে",
  not_created: "Booking তৈরি হয়নি",
  issued: "Ticket issued",
  cancelled: "Cancelled",
};
const errors = {
  MANUAL_RECONCILIATION_REQUIRED:
    "Supplier reference পাওয়া যায়নি। Supplier portal বা support থেকে যাচাই করুন।",
  SUPPLIER_RECONCILIATION_FAILED:
    "Supplier থেকে নিশ্চিত status পাওয়া যায়নি। Portal বা support থেকে যাচাই করুন।",
  SUPPLIER_READ_FAILED: "Supplier-এর সঙ্গে সংযোগ হয়নি। পরে আবার চেষ্টা করুন।",
  SUPPLIER_TIMEOUT: "Supplier উত্তর দিতে সময় নিচ্ছে। পরে status চেক করুন।",
  SUPPLIER_SERVICING_DISABLED:
    "এই supplier-এর status check বন্ধ আছে। দায়িত্বপ্রাপ্ত ব্যক্তির সঙ্গে যোগাযোগ করুন।",
  STALE_BOOKING_VIEW:
    "এই booking আপডেট হয়েছে। আবার খুলে তথ্য দেখে সিদ্ধান্ত দিন।",
  ALREADY_RESOLVED: "এই booking আগেই resolve করা হয়েছে। আবার খুলে ফল দেখুন।",
  BOOKING_NOT_RESOLVABLE: "Request এখনও চলতে পারে। কিছুক্ষণ পরে আবার দেখুন।",
  SUPPLIER_EVIDENCE_REQUIRED:
    "Supplier confirmation, কারণ, প্রমাণ ও reference পূরণ করুন।",
};
function message(text) {
  el("message").textContent = text;
}
function signedOut() {
  token = "";
  selected = null;
  cursor = null;
  el("workspace").hidden = true;
  el("login").hidden = false;
  el("logout").hidden = true;
  el("bookings").replaceChildren();
  el("detail").replaceChildren();
  el("environment").textContent = "Admin";
}
async function api(path, method = "GET", data) {
  const r = await fetch(path, {
    method,
    headers: {
      "Content-Type": "application/json",
      ...(token ? { Authorization: "Bearer " + token } : {}),
    },
    ...(data ? { body: JSON.stringify(data) } : {}),
  });
  let body = {};
  try {
    body = await r.json();
  } catch {}
  if (!r.ok) {
    if (r.status === 401) {
      signedOut();
      throw Error("লগইন করুন। Session শেষ হয়ে থাকতে পারে বা তথ্য সঠিক নয়।");
    }
    throw Error(
      errors[body.error] ||
        "কাজটি সম্পন্ন হয়নি। তথ্য যাচাই করে আবার চেষ্টা করুন।",
    );
  }
  return body;
}
async function action(fn) {
  if (busy) return;
  busy = true;
  document.querySelectorAll("button").forEach((b) => (b.disabled = true));
  try {
    message("");
    await fn();
  } catch (e) {
    message(e.message);
  } finally {
    busy = false;
    document.querySelectorAll("button").forEach((b) => (b.disabled = false));
  }
}
function node(tag, text, cls) {
  const n = document.createElement(tag);
  if (text !== undefined) n.textContent = text;
  if (cls) n.className = cls;
  return n;
}
async function list(append = false) {
  const resolved = el("view").value === "resolved";
  const body = await api(
    "/admin/bookings?resolved=" +
      resolved +
      (append && cursor ? "&before=" + encodeURIComponent(cursor) : ""),
  );
  el("queueTitle").textContent = resolved
    ? "সমাধান নথিভুক্ত"
    : "যাচাইয়ের অপেক্ষায়";
  el("queueDescription").textContent = resolved
    ? "নথিভুক্ত সিদ্ধান্ত ও তার প্রমাণ দেখতে একটি booking নির্বাচন করুন।"
    : "Supplier থেকে নিশ্চিত ফল না পাওয়া bookingগুলো এখানে দেখুন।";
  if (!append) el("bookings").replaceChildren();
  el("environment").textContent = body.environment;
  cursor = body.nextCursor;
  el("more").hidden = !cursor;
  for (const b of body.items) {
    const button = node(
      "button",
      (b.publicRef || "Reference pending") + " · " + b.clientName + " · " + (b.pnr || "PNR পাওয়া যায়নি"),
      "booking",
    );
    button.append(
      node("span", labels[b.state] + " · " + b.supplier),
      node("span", new Date(b.createdAt).toLocaleString("bn-BD")),
    );
    button.onclick = () => action(() => open(b.id));
    el("bookings").append(button);
  }
  if (!el("bookings").children.length)
    el("bookings").append(node("p", "এই তালিকায় কোনো booking নেই।", "muted"));
}
async function open(id) {
  selected = await api("/admin/bookings/" + id);
  render();
}
function render() {
  const b = selected,
    d = el("detail");
  d.replaceChildren(node("h2", "Booking বিস্তারিত"));
  const facts = node("dl");
  for (const [name, value] of [
    ["Agency", b.clientName],
    ["Order Ref", b.publicRef || "PNR পাওয়ার অপেক্ষায়"],
    ["Booking ID", b.id],
    ["Supplier", b.supplier],
    ["PNR", b.pnr || "পাওয়া যায়নি"],
    ["আমাদের রেকর্ড", labels[b.state] || b.state],
    ["Supplier status", b.supplierStatus || "নিশ্চিত নয়"],
    ["Ticket deadline", b.deadline || "নিশ্চিত নয়"],
    [
      "শেষ status check",
      b.lastCheckedAt
        ? new Date(b.lastCheckedAt).toLocaleString("bn-BD")
        : "এখনও হয়নি",
    ],
  ]) {
    facts.append(node("dt", name), node("dd", value));
  }
  d.append(facts);
  const recheck = node("button", "Status আবার চেক করুন", "secondary");
  recheck.onclick = () =>
    action(async () => {
      try {
        await api("/admin/bookings/" + b.id + "/recheck", "POST", {});
        message(
          "Supplier status পাওয়া গেছে। তথ্য দেখে প্রয়োজন হলে সিদ্ধান্ত দিন।",
        );
      } finally {
        await open(b.id);
      }
    });
  d.append(recheck);
  if (b.resolutionHistory && b.resolutionHistory.length) {
    const history = node("details"),
      summary = node(
        "summary",
        "সিদ্ধান্তের ইতিহাস (" + b.resolutionHistory.length + ")",
      );
    history.append(summary);
    for (const r of b.resolutionHistory) {
      const entry = node("dl");
      for (const [name, value] of [
        ["ফল", labels[r.outcome]],
        ["Admin", r.administratorName],
        ["সময়", new Date(r.createdAt).toLocaleString("bn-BD")],
        ["কারণ", r.reason],
        ["প্রমাণ", r.evidence],
        ["Reference", r.supplierCaseRef],
      ])
        entry.append(node("dt", name), node("dd", value));
      history.append(entry);
    }
    d.append(history);
  }
  if (b.lateOutcome)
    d.append(
      node(
        "p",
        "আগের সিদ্ধান্তের পরে supplier response এসেছে। নতুন তথ্য যাচাই করে আবার সিদ্ধান্ত দিন।",
        "notice",
      ),
    );
  if (b.resolution && b.state === "manually_resolved") {
    d.append(node("h2", "নথিভুক্ত সিদ্ধান্ত"));
    const res = node("dl");
    for (const [k, v] of [
      ["ফল", labels[b.resolution.outcome]],
      ["Admin", b.resolution.administratorName],
      ["কারণ", b.resolution.reason],
      ["প্রমাণ", b.resolution.evidence],
      ["Support reference", b.resolution.supplierCaseRef],
      ["সময়", new Date(b.resolution.createdAt).toLocaleString("bn-BD")],
    ])
      res.append(node("dt", k), node("dd", v));
    d.append(res);
    return;
  }
  if (!b.canResolve) {
    d.append(
      node(
        "p",
        "Request এখনও চলতে পারে। পাঁচ মিনিটের কম পুরোনো pending booking resolve করা যায় না। পরে তালিকা আপডেট করুন।",
        "notice",
      ),
    );
    return;
  }
  d.append(
    node(
      "p",
      "Supplier portal বা support থেকে নিশ্চিত হওয়ার পর সিদ্ধান্ত দিন। এই কাজ নতুন Hold, Issue বা Cancel করে না।",
      "notice",
    ),
  );
  const form = node("form");
  const choose = node("select");
  choose.id = "outcome";
  for (const [v, t] of [
    ["", "ফল নির্বাচন করুন"],
    ["held", "Supplier নিশ্চিত করেছে: Hold আছে"],
    ["not_created", "Supplier নিশ্চিত করেছে: Booking তৈরি হয়নি"],
    ["issued", "Supplier নিশ্চিত করেছে: Ticket issued"],
    ["cancelled", "Supplier নিশ্চিত করেছে: Cancelled"],
  ]) {
    const o = node("option", t);
    o.value = v;
    choose.append(o);
  }
  choose.required = true;
  const l = node("label", "যাচাইয়ের ফল");
  l.htmlFor = "outcome";
  form.append(l, choose);
  for (const [id, label, min, max, tag] of [
    ["reason", "কেন এই সিদ্ধান্ত নিচ্ছেন?", 10, 2000, "textarea"],
    [
      "evidence",
      "কীভাবে যাচাই করেছেন? Supplier-এর নিশ্চিত তথ্য লিখুন।",
      10,
      4000,
      "textarea",
    ],
    [
      "supplierCaseRef",
      "Supplier support case / confirmation reference",
      3,
      200,
      "input",
    ],
  ]) {
    const labelEl = node("label", label);
    labelEl.htmlFor = id;
    const inp = node(tag);
    inp.id = id;
    inp.required = true;
    inp.minLength = min;
    inp.maxLength = max;
    form.append(labelEl, inp);
  }
  form.append(
    node(
      "p",
      "Passport, password বা পূর্ণ passenger details এখানে লিখবেন না।",
      "muted",
    ),
  );
  const check = node("label", undefined, "check"),
    box = node("input");
  box.type = "checkbox";
  box.required = true;
  check.append(
    box,
    node("span", "আমি supplier-এর সঙ্গে যাচাই করেছি এবং উপরের ফল নিশ্চিত।"),
  );
  form.append(check);
  const buttons = node("div", undefined, "actions"),
    save = node("button", "সিদ্ধান্ত নথিভুক্ত করুন");
  buttons.append(save);
  form.append(buttons);
  form.onsubmit = (e) => {
    e.preventDefault();
    action(async () => {
      await api("/admin/bookings/" + b.id + "/resolve", "POST", {
        outcome: choose.value,
        reason: el("reason").value,
        evidence: el("evidence").value,
        supplierCaseRef: el("supplierCaseRef").value,
        expectedUpdatedAt: b.updatedAt,
        confirmedWithSupplier: box.checked,
      });
      await list();
      await open(b.id);
      message("সিদ্ধান্ত ও audit record সংরক্ষিত হয়েছে।");
    });
  };
  d.append(form);
}
el("loginForm").onsubmit = (e) => {
  e.preventDefault();
  action(async () => {
    const body = await api("/admin/login", "POST", {
      username: el("username").value,
      password: el("password").value,
    });
    token = body.access_token;
    el("password").value = "";
    el("login").hidden = true;
    el("workspace").hidden = false;
    el("logout").hidden = false;
    await list();
  });
};
el("refresh").onclick = () => action(() => list());
el("more").onclick = () => action(() => list(true));
el("logout").onclick = () =>
  action(async () => {
    try {
      await api("/admin/logout", "POST", {});
    } finally {
      signedOut();
    }
  });

el("view").onchange = () =>
  action(async () => {
    cursor = null;
    await list();
  });
