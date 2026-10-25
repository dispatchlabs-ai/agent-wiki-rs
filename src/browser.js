"use strict";
const csrf = document.querySelector('meta[name="csrf-token"]')?.content || "";
async function call(name, args) {
  const response = await fetch(`/api/tools/${encodeURIComponent(name)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
    body: JSON.stringify(args),
    credentials: "same-origin"
  });
  const result = await response.json();
  if (!response.ok) {
    const error = new Error(`${result.error?.code || response.status}: ${result.error?.message || "Request failed"}`);
    error.status = response.status;
    throw error;
  }
  return result;
}
const editor = document.querySelector("#editor");
if (editor) {
  let pending = null;
  const status = document.querySelector("#status");
  editor.addEventListener("submit", async event => {
    event.preventDefault();
    const button = editor.querySelector('[type="submit"]');
    button.disabled = true;
    try {
      if (!pending) {
        const data = new FormData(editor);
        pending = {
          operation_id: `edit-${crypto.randomUUID()}`,
          updates: [{
            id: data.get("id"),
            expected_revision_id: data.get("revision") || null,
            title: data.get("title"),
            description: data.get("description"),
            topic: data.get("topic") || "Wiki",
            body: data.get("body"),
            summary: data.get("summary")
          }]
        };
        const sources = editor.elements.evidence;
        if (sources && sources.value !== sources.defaultValue) {
          try { pending.updates[0].evidence = JSON.parse(sources.value); }
          catch (error) { pending = null; error.status = 400; throw error; }
        }
      }
      status.textContent = "Saving…";
      await call("wiki.save", pending);
      location.assign(`/wiki/${encodeURIComponent(pending.updates[0].id)}/`);
    } catch (error) {
      if ([400, 413, 415, 422].includes(error.status)) {
        pending = null;
        status.textContent = `${error.message}\nCorrect the draft and save again.`;
      } else {
        status.textContent = `${error.message}\nSave again retries the exact pending edit. Copy any newer draft before reloading to resolve a conflict.`;
      }
    } finally { button.disabled = false; }
  });
  document.querySelector("#preview-button").addEventListener("click", async () => {
    try {
      const result = await call("wiki.preview", { body: editor.elements.body.value });
      const preview = document.querySelector("#preview");
      preview.innerHTML = result.html;
      preview.hidden = false;
      preview.scrollIntoView({ behavior: "smooth" });
    } catch (error) { status.textContent = error.message; }
  });
}
document.querySelector("#grant")?.addEventListener("submit", async event => {
  event.preventDefault();
  const form = event.currentTarget;
  const role = form.elements.role.value;
  const status = document.querySelector("#status");
  const button = form.querySelector("button");
  button.disabled = true;
  try {
    const result = await call("wiki.grant", {
      name: form.elements.name.value,
      role,
      scopes: role === "reader" ? ["read"] : role === "manager" ? ["read", "write", "trace", "manage"] : ["read", "write", "trace"]
    });
    document.querySelector("#new-secret").value = result.token;
    document.querySelector("#secret-label").hidden = false;
    status.textContent = "Credential created. Copy it to private storage before leaving this page.";
  } catch (error) { status.textContent = error.message; }
  finally { button.disabled = false; }
});
document.querySelectorAll(".change-role").forEach(button => {
  button.addEventListener("click", async () => {
    const role = button.closest("tr").querySelector(".access-role").value;
    const scopes = role === "reader" ? ["read"] : role === "manager" ? ["read", "write", "trace", "manage"] : ["read", "write", "trace"];
    try { await call("wiki.changeAccess", { id: button.dataset.id, role, scopes }); location.reload(); }
    catch (error) { alert(error.message); }
  });
});
document.querySelectorAll(".revoke").forEach(button => {
  button.addEventListener("click", async () => {
    if (!confirm("Revoke this credential and all of its browser sessions?")) return;
    try { await call("wiki.revoke", { id: button.dataset.id }); location.reload(); }
    catch (error) { alert(error.message); }
  });
});
if (navigator.modelContext && csrf) {
  fetch("/api/tools", { credentials: "same-origin" })
    .then(response => response.ok ? response.json() : [])
    .then(tools => {
      for (const tool of tools) {
        navigator.modelContext.registerTool({
          ...tool,
          execute: async args => ({ content: [{ type: "text", text: JSON.stringify(await call(tool.name, args)) }] })
        });
      }
    }).catch(() => {});
}
