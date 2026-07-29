(() => {
  "use strict";

  const root = new URL(".", window.location.href);
  const status = document.querySelector("#status");
  const frame = document.querySelector("#plan");
  const submit = document.querySelector("#submit");
  const discard = document.querySelector("#discard");

  frame.src = new URL("assets/__rep_document__.html", root);
  frame.addEventListener("load", async () => {
    try {
      const response = await fetch(new URL("api/state", root), {
        headers: { "Accept": "application/json" },
      });
      const state = await response.json();
      const blocked = state.blockedResources || 0;
      status.textContent = blocked
        ? `Ready · ${blocked} unsafe resource${blocked === 1 ? "" : "s"} blocked`
        : "Ready";
    } catch {
      status.textContent = "Plan loaded; status unavailable";
    }
  }, { once: true });

  async function finish(kind) {
    submit.disabled = true;
    discard.disabled = true;
    status.textContent = kind === "finish" ? "Submitting…" : "Discarding…";
    try {
      const response = await fetch(new URL(`api/${kind}`, root), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{}",
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      document.body.classList.add("finished");
      document.querySelector("#completion").hidden = false;
    } catch (error) {
      submit.disabled = false;
      discard.disabled = false;
      status.textContent = `Could not complete review: ${error.message}`;
    }
  }

  submit.addEventListener("click", () => finish("finish"));
  discard.addEventListener("click", () => finish("discard"));
})();
