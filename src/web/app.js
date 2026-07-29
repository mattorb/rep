import {
  SelectionOverlay,
  extractDocument,
  selectionPoint,
} from "./document.js";

const root = new URL(".", window.location.href);
const status = document.querySelector("#status");
const mode = document.querySelector("#mode");
const frame = document.querySelector("#plan");
const submit = document.querySelector("#submit");
const discard = document.querySelector("#discard");
let state = null;
let extracted = null;
let overlay = null;
let commandQueue = Promise.resolve();

window.__repTest = {
  get manifest() {
    return extracted?.manifest || null;
  },
  get state() {
    return state;
  },
  get overlay() {
    return overlay;
  },
};

function setStatus(message) {
  status.textContent = message;
}

async function api(path, options = {}) {
  const response = await fetch(new URL(`api/${path}`, root), {
    headers: {
      Accept: "application/json",
      ...(options.body ? { "Content-Type": "application/json" } : {}),
    },
    ...options,
  });
  if (!response.ok) {
    const message = await response.text();
    const error = new Error(message || `HTTP ${response.status}`);
    error.status = response.status;
    throw error;
  }
  return response.json();
}

function readyStatus(next) {
  if (next.status === "empty") return "No selectable text in this HTML plan";
  if (next.message) return next.message;
  const blocked = next.blockedResources || 0;
  return blocked
    ? `Ready · ${blocked} unsafe resource${blocked === 1 ? "" : "s"} blocked`
    : "Ready";
}

function renderState(next, scroll = true) {
  state = next;
  const empty = next.status === "empty";
  frame.classList.toggle("empty", empty);
  mode.hidden = !next.mode;
  mode.textContent = next.mode || "";
  setStatus(readyStatus(next));
  overlay?.paint(next.selection || [], scroll);
}

function withTimeout(promise, milliseconds) {
  return Promise.race([
    promise,
    new Promise((resolve) => setTimeout(resolve, milliseconds)),
  ]);
}

async function waitForLayout(doc) {
  setStatus("Waiting for fonts and images…");
  const fonts = doc.fonts?.ready || Promise.resolve();
  const images = Promise.all(
    Array.from(doc.images)
      .filter((image) => !image.complete)
      .map(
        (image) =>
          new Promise((resolve) => {
            image.addEventListener("load", resolve, { once: true });
            image.addEventListener("error", resolve, { once: true });
          }),
      ),
  );
  await withTimeout(Promise.all([fonts, images]), 5000);
}

async function initialize() {
  const doc = frame.contentDocument;
  await waitForLayout(doc);
  setStatus("Extracting selectable text…");
  extracted = extractDocument(doc);
  overlay = new SelectionOverlay(doc, extracted.models);
  const next = await api("manifest", {
    method: "POST",
    body: JSON.stringify(extracted.manifest),
  });
  renderState(next, false);
  installDocumentEvents(doc);
  const repaint = () => overlay?.paint(state?.selection || [], false);
  doc.defaultView.addEventListener("scroll", repaint, { passive: true });
  doc.defaultView.addEventListener("resize", repaint);
  doc.fonts?.addEventListener?.("loadingdone", repaint);
  for (const image of doc.images) image.addEventListener("load", repaint);
}

async function sendCommand(command) {
  if (!state || state.status !== "ready") return;
  try {
    const next = await api("command", {
      method: "POST",
      body: JSON.stringify({ revision: state.revision, ...command }),
    });
    renderState(next);
  } catch (error) {
    if (error.status === 409) {
      renderState(await api("state"), false);
      setStatus("Review state was restored; retry your command");
    } else {
      setStatus(`Command failed: ${error.message}`);
    }
  }
}

function queueCommand(command) {
  commandQueue = commandQueue.then(() => sendCommand(command));
}

function keyCommand(event) {
  if (event.metaKey || event.ctrlKey || event.altKey) return null;
  if (
    event.key === "j" ||
    event.key === "ArrowDown" ||
    event.key === "ArrowRight"
  ) {
    return { type: "move", forward: true };
  }
  if (
    event.key === "k" ||
    event.key === "ArrowUp" ||
    event.key === "ArrowLeft"
  ) {
    return { type: "move", forward: false };
  }
  if (event.key === " ") return { type: "cycle", forward: true };
  if (event.key === "Backspace") return { type: "cycle", forward: false };
  if (event.key === "i") return { type: "adjust", finer: true };
  if (event.key === "o") return { type: "adjust", finer: false };
  return null;
}

function onKeydown(event) {
  const command = keyCommand(event);
  if (!command) return;
  event.preventDefault();
  queueCommand(command);
}

function installDocumentEvents(doc) {
  doc.addEventListener("keydown", onKeydown, true);
  doc.addEventListener(
    "click",
    (event) => {
      if (!extracted || !state || state.status !== "ready") return;
      const point = selectionPoint(
        extracted.models,
        doc,
        event.clientX,
        event.clientY,
        event.target,
      );
      if (!point) return;
      const unit =
        event.detail >= 3
          ? "paragraph"
          : event.detail === 2
            ? "sentence"
            : "word";
      event.preventDefault();
      queueCommand({ type: "select", ...point, unit });
    },
    true,
  );
}

async function finish(kind) {
  submit.disabled = true;
  discard.disabled = true;
  setStatus(kind === "finish" ? "Submitting…" : "Discarding…");
  try {
    await api(kind, { method: "POST", body: "{}" });
    document.body.classList.add("finished");
    document.querySelector("#completion").hidden = false;
  } catch (error) {
    submit.disabled = false;
    discard.disabled = false;
    setStatus(`Could not complete review: ${error.message}`);
  }
}

window.addEventListener("keydown", onKeydown, true);
submit.addEventListener("click", () => finish("finish"));
discard.addEventListener("click", () => finish("discard"));
frame.addEventListener(
  "load",
  () => {
    initialize().catch((error) => {
      setStatus(`Could not initialize review: ${error.message}`);
    });
  },
  { once: true },
);
frame.src = new URL("assets/__rep_document__.html", root);
