// the window talks to the app only through its two commands, today and undo, plus the
// "shown" event the app sends each time the tray brings the window back
const invoke = window.__TAURI__.core.invoke;

const list = document.getElementById("rows");
const status = document.getElementById("status");

function text(tag, value, cls) {
  const el = document.createElement(tag);
  el.textContent = value;
  if (cls) el.className = cls;
  return el;
}

function render(today) {
  document.getElementById("summary").textContent = today.summary;
  document.getElementById("ledger").textContent = today.ledger;
  list.replaceChildren();
  for (const r of today.rows) {
    const li = document.createElement("li");
    li.className = "row";
    const head = document.createElement("p");
    head.className = "head";
    head.append(text("span", r.time, "time"), " ", text("span", r.agent, "agent"), " ",
      text("span", r.kind, "kind"));
    li.append(head, text("p", r.target, "target"));
    if (r.undo_of) li.append(text("p", "This is the undo of #" + r.undo_of + ".", "undo-of"));
    // the state is always written out, so it never depends on colour alone
    li.append(text("p", "State: " + r.state, "state state-" + r.state.replaceAll(" ", "-")));
    if (r.note) li.append(text("p", r.note, "note"));
    if (r.can_undo) {
      // undoing an undo's own record puts the original change back
      const label = r.undo_of ? "Take back this undo" : "Undo";
      const b = text("button", label, "undo");
      b.type = "button";
      b.setAttribute("aria-label", label + " #" + r.id + ", " + r.kind + " of " + r.target);
      b.addEventListener("click", () => undo(r.id, b));
      li.append(b);
    }
    list.append(li);
  }
}

async function load() {
  try {
    render(await invoke("today"));
  } catch (e) {
    status.textContent = "Could not read the ledger: " + e;
  }
}

async function undo(id, button) {
  button.disabled = true;
  try {
    status.textContent = await invoke("undo", { id });
  } catch (e) {
    // a conflict comes back as an error and nothing on disk has changed
    status.textContent = "Not undone. " + e;
  }
  await load();
}

document.getElementById("refresh").addEventListener("click", load);
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) load();
});
// WebView2 on Windows can keep a hidden window "visible", so the app's own event is the
// signal that works on every platform
window.__TAURI__.event.listen("shown", load);
load();
