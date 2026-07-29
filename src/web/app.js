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
const modal = document.querySelector("#modal");
const modalForm = document.querySelector("#modal-form");
const modalTitle = document.querySelector("#modal-title");
const modalContent = document.querySelector("#modal-content");
const modalInputLabel = document.querySelector("#modal-input-label");
const modalInput = document.querySelector("#modal-input");
const modalCancel = document.querySelector("#modal-cancel");
const modalConfirm = document.querySelector("#modal-confirm");
let state = null;
let extracted = null;
let overlay = null;
let modalAction = null;
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
  const parts = ["Ready"];
  if (next.annotationCount) {
    parts.push(
      `${next.annotationCount} annotation${next.annotationCount === 1 ? "" : "s"}`,
    );
  }
  if (next.blockedResources) {
    parts.push(
      `${next.blockedResources} unsafe resource${next.blockedResources === 1 ? "" : "s"} blocked`,
    );
  }
  return parts.join(" · ");
}

function renderState(next, scroll = true) {
  state = next;
  const empty = next.status === "empty";
  frame.classList.toggle("empty", empty);
  mode.hidden = !next.mode;
  mode.textContent = next.mode || "";
  setStatus(readyStatus(next));
  overlay?.paint(
    next.selection || [],
    next.annotations || [],
    scroll,
  );
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
  const repaint = () =>
    overlay?.paint(
      state?.selection || [],
      state?.annotations || [],
      false,
    );
  doc.defaultView.addEventListener("scroll", repaint, { passive: true });
  doc.defaultView.addEventListener("resize", repaint);
  doc.fonts?.addEventListener?.("loadingdone", repaint);
  for (const image of doc.images) image.addEventListener("load", repaint);
}

async function sendCommand(command) {
  if (!state || state.status !== "ready") return false;
  try {
    const next = await api("command", {
      method: "POST",
      body: JSON.stringify({ revision: state.revision, ...command }),
    });
    renderState(next);
    return true;
  } catch (error) {
    if (error.status === 409) {
      renderState(await api("state"), false);
      setStatus("Review state was restored; retry your command");
    } else {
      setStatus(`Command failed: ${error.message}`);
    }
    return false;
  }
}

function queueCommand(command) {
  commandQueue = commandQueue.then(() => sendCommand(command));
  return commandQueue;
}

function closeModal() {
  modalAction = null;
  if (modal.open) modal.close();
}

function openModal({
  title,
  content = "",
  inputLabel = null,
  inputValue = "",
  confirm = "Close",
  action = null,
}) {
  modalTitle.textContent = title;
  modalContent.replaceChildren();
  if (typeof content === "string") modalContent.textContent = content;
  else if (content) modalContent.append(content);
  const hasInput = inputLabel !== null;
  modalInput.hidden = !hasInput;
  modalInputLabel.hidden = !hasInput;
  modalInputLabel.textContent = inputLabel || "";
  modalInput.value = inputValue;
  modalCancel.hidden = !action;
  modalConfirm.textContent = confirm;
  modalAction = action;
  modal.showModal();
  if (hasInput) {
    modalInput.focus();
    modalInput.setSelectionRange(modalInput.value.length, modalInput.value.length);
  } else {
    modalConfirm.focus();
  }
}

function listContent(items, emptyMessage) {
  if (!items.length) {
    const paragraph = document.createElement("p");
    paragraph.textContent = emptyMessage;
    return paragraph;
  }
  const list = document.createElement("ol");
  for (const item of items) {
    const row = document.createElement("li");
    row.textContent = item;
    list.append(row);
  }
  return list;
}

function openAnnotation(kind, title) {
  openModal({
    title,
    inputLabel: title,
    confirm: "Save",
    action: (text) => queueCommand({ type: "annotate", kind, text }),
  });
}

function openSearch() {
  openModal({
    title: "Search",
    inputLabel: "Search query",
    confirm: "Search",
    action: (query) =>
      queueCommand({ type: "search", query, forward: true }),
  });
}

function openEdit() {
  if (!state?.editable) {
    setStatus("No editable change or feedback at this selection");
    return;
  }
  openModal({
    title: `Edit ${state.editable.kind}`,
    inputLabel: "Annotation text",
    inputValue: state.editable.text,
    confirm: "Update",
    action: (text) => queueCommand({ type: "edit", text }),
  });
}

function openHelp() {
  const rows = [
    "j / k or arrows — move",
    "Space / Backspace — cycle selection unit",
    "i / o — finer / coarser selection",
    "/ then n / N — search and move through results",
    "c / f — change / feedback",
    "b / a — insert before / after",
    "x — clear an annotation, then mark for deletion",
    "[ / ] — previous / next annotation",
    "e — edit the applicable change or feedback",
    "I / O — outline / links",
    "r — copy current action output",
    "q / Q — submit with confirmation / discard silently",
  ];
  openModal({
    title: "Keyboard help",
    content: listContent(rows, ""),
  });
}

function openOutline() {
  openModal({
    title: "Document outline",
    content: listContent(
      (state?.outline || []).map(
        (row) => `${"  ".repeat(row.level - 1)}${row.text}`,
      ),
      "This document has no headings.",
    ),
  });
}

function openLinks() {
  openModal({
    title: "Links at selection",
    content: listContent(state?.links || [], "No links at this selection."),
  });
}

function confirmFinish(kind, title, message) {
  openModal({
    title,
    content: message,
    confirm: kind === "finish" ? "Submit" : "Discard",
    action: () => finish(kind),
  });
}

async function copyOutput() {
  try {
    const { output } = await api("output");
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(output);
    } else {
      const temporary = document.createElement("textarea");
      temporary.value = output;
      temporary.style.position = "fixed";
      temporary.style.opacity = "0";
      document.body.append(temporary);
      temporary.select();
      if (!document.execCommand("copy")) throw new Error("copy unavailable");
      temporary.remove();
    }
    setStatus("Copied action output to clipboard");
  } catch (error) {
    setStatus(`Could not copy output: ${error.message}`);
  }
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
  if (event.key === "n") return { type: "jumpSearch", forward: true };
  if (event.key === "N") return { type: "jumpSearch", forward: false };
  if (event.key === "[") return { type: "jumpAnnotation", forward: false };
  if (event.key === "]") return { type: "jumpAnnotation", forward: true };
  if (event.key === "x") return { type: "strike" };
  return null;
}

function onKeydown(event) {
  if (modal.open) {
    if (event.key === "Escape") {
      event.preventDefault();
      closeModal();
    } else if (
      event.key === "Enter" &&
      event.target === modalInput &&
      !event.shiftKey
    ) {
      event.preventDefault();
      modalForm.requestSubmit();
    }
    return;
  }
  const command = keyCommand(event);
  if (command) {
    event.preventDefault();
    queueCommand(command);
    return;
  }
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  const actions = {
    "/": openSearch,
    c: () => openAnnotation("change", "Literal change"),
    f: () => openAnnotation("feedback", "Feedback or intent"),
    b: () => openAnnotation("insertBefore", "Insert before"),
    a: () => openAnnotation("insertAfter", "Insert after"),
    e: openEdit,
    "?": openHelp,
    I: openOutline,
    O: openLinks,
    r: copyOutput,
    q: () =>
      confirmFinish("finish", "Submit review?", "Submit all current annotations?"),
    Q: () => finish("discard"),
  };
  const action = actions[event.key];
  if (action) {
    event.preventDefault();
    action();
  }
}

function installDocumentEvents(doc) {
  doc.addEventListener("keydown", onKeydown, true);
  doc.addEventListener(
    "click",
    (event) => {
      if (!extracted || !state || state.status !== "ready" || modal.open) return;
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
  closeModal();
  submit.disabled = true;
  discard.disabled = true;
  setStatus(kind === "finish" ? "Submitting…" : "Discarding…");
  try {
    await commandQueue;
    await api(kind, { method: "POST", body: "{}" });
    document.body.classList.add("finished");
    document.querySelector("#completion").hidden = false;
  } catch (error) {
    submit.disabled = false;
    discard.disabled = false;
    setStatus(`Could not complete review: ${error.message}`);
  }
}

modalForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!modalAction) {
    closeModal();
    return;
  }
  const value = modalInput.hidden ? undefined : modalInput.value;
  if (!modalInput.hidden && !value.trim()) {
    setStatus("Enter text before saving");
    return;
  }
  modalConfirm.disabled = true;
  const completed = await modalAction(value);
  modalConfirm.disabled = false;
  if (completed !== false) closeModal();
});
modalCancel.addEventListener("click", closeModal);
modal.addEventListener("cancel", (event) => {
  event.preventDefault();
  closeModal();
});
window.addEventListener("keydown", onKeydown, true);
submit.addEventListener("click", () => finish("finish"));
discard.addEventListener("click", () => {
  if (state?.annotationCount) {
    confirmFinish(
      "discard",
      "Discard review?",
      "Discard all annotations without producing action output?",
    );
  } else {
    finish("discard");
  }
});
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
