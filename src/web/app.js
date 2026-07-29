import {
  SelectionOverlay,
  extractDocument,
  selectionPoint,
} from "./document.js";

const root = new URL(".", window.location.href);
const status = document.querySelector("#status");
const mode = document.querySelector("#mode");
const frame = document.querySelector("#plan");
const interactionLayer = document.querySelector("#interaction-layer");
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
let layoutObserver = null;
let heartbeatTimer = null;
let modalAction = null;
let commandQueue = Promise.resolve();
let planClick = null;
const planClickEvents = [];

const MULTI_CLICK_INTERVAL_MS = 500;
const MULTI_CLICK_DISTANCE_PX = 6;

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
  get clickEvents() {
    return planClickEvents.map((event) => ({ ...event }));
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

function repaintOverlay() {
  overlay?.paint(
    state?.selection || [],
    state?.annotations || [],
    false,
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
  layoutObserver?.disconnect();
  layoutObserver = new ResizeObserver(repaintOverlay);
  layoutObserver.observe(doc.documentElement);
  for (const model of extracted.models) layoutObserver.observe(model.owner);
  clearInterval(heartbeatTimer);
  heartbeatTimer = setInterval(() => {
    api("heartbeat", { method: "POST", body: "{}" }).catch(() => {
      // Foreground commands surface connection failures. A background
      // keepalive should not overwrite the user's current status message.
    });
  }, 60_000);
}

async function sendCommand(command, scroll) {
  if (!state || state.status !== "ready") return false;
  try {
    const next = await api("command", {
      method: "POST",
      body: JSON.stringify({ revision: state.revision, ...command }),
    });
    renderState(next, scroll);
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

function queueCommand(command, { scroll = true } = {}) {
  commandQueue = commandQueue.then(() => sendCommand(command, scroll));
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
      "This document has no selectable outline entries.",
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

function openCopyFallback(output, reason) {
  const content = document.createElement("div");
  const message = document.createElement("p");
  message.textContent =
    "Clipboard access was unavailable. Select the action output below or use Copy.";
  const text = document.createElement("textarea");
  text.className = "copy-output";
  text.readOnly = true;
  text.rows = 12;
  text.value = output;
  content.append(message, text);
  openModal({
    title: "Copy action output",
    content,
    confirm: "Copy",
    action: () => {
      text.focus();
      text.select();
      if (!document.execCommand("copy")) {
        setStatus(`Could not copy output: ${reason}`);
        return false;
      }
      setStatus("Copied action output to clipboard");
      return true;
    },
  });
}

async function copyOutput() {
  let output = "";
  try {
    ({ output } = await api("output"));
    if (!navigator.clipboard?.writeText) throw new Error("Clipboard API unavailable");
    await navigator.clipboard.writeText(output);
    setStatus("Copied action output to clipboard");
  } catch (error) {
    if (output) {
      openCopyFallback(output, error.message);
      setStatus("Clipboard access unavailable; action output is ready to copy");
    } else {
      setStatus(`Could not load action output: ${error.message}`);
    }
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
      modalInput.hidden &&
      ["?", "I", "O"].includes(event.key)
    ) {
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

function onPlanClick(event) {
  const recorded = {
    clickCount: null,
    detail: event.detail,
    node: null,
    status: "ignored",
    unit: null,
    x: event.clientX,
    y: event.clientY,
  };
  planClickEvents.push(recorded);
  if (planClickEvents.length > 10) planClickEvents.shift();
  if (!extracted || !state || state.status !== "ready" || modal.open) return;
  const doc = frame.contentDocument;
  const frameRect = frame.getBoundingClientRect();
  const x = event.clientX - frameRect.left;
  const y = event.clientY - frameRect.top;
  if (x < 0 || y < 0 || x > frameRect.width || y > frameRect.height) return;
  const target = (doc.elementsFromPoint?.(x, y) || [doc.elementFromPoint(x, y)])
    .find(
      (element) =>
        element &&
        element !== overlay?.host &&
        !element.closest?.("[data-rep-overlay]"),
    );
  const point = selectionPoint(
    extracted.models,
    doc,
    x,
    y,
    target,
  );
  if (!point) {
    recorded.status = "unmapped";
    return;
  }
  const now = performance.now();
  const continuesSequence =
    planClick &&
    planClick.node === point.node &&
    now - planClick.time <= MULTI_CLICK_INTERVAL_MS &&
    Math.hypot(event.clientX - planClick.x, event.clientY - planClick.y) <=
      MULTI_CLICK_DISTANCE_PX;
  const inferredCount = continuesSequence ? planClick.count + 1 : 1;
  const nativeCount = Number.isSafeInteger(event.detail)
    ? Math.max(1, event.detail)
    : 1;
  const clickCount = Math.min(3, Math.max(inferredCount, nativeCount));
  planClick = {
    count: clickCount,
    node: point.node,
    time: now,
    x: event.clientX,
    y: event.clientY,
  };
  const unit =
    clickCount >= 3
      ? "paragraph"
      : clickCount === 2
        ? "sentence"
        : "word";
  Object.assign(recorded, {
    clickCount,
    node: point.node,
    status: "selected",
    unit,
  });
  queueCommand({ type: "select", ...point, unit }, { scroll: false });
}

function onPlanWheel(event) {
  const view = frame.contentWindow;
  if (!view) return;
  event.preventDefault();
  const scale =
    event.deltaMode === WheelEvent.DOM_DELTA_LINE
      ? 16
      : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
        ? Math.max(1, view.innerHeight)
        : 1;
  view.scrollBy(event.deltaX * scale, event.deltaY * scale);
  requestAnimationFrame(repaintOverlay);
}

async function finish(kind) {
  closeModal();
  submit.disabled = true;
  discard.disabled = true;
  setStatus(kind === "finish" ? "Submitting…" : "Discarding…");
  try {
    await commandQueue;
    await api(kind, { method: "POST", body: "{}" });
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
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
window.addEventListener("resize", repaintOverlay);
interactionLayer.addEventListener("pointerdown", () => {
  interactionLayer.focus({ preventScroll: true });
});
interactionLayer.addEventListener("click", onPlanClick);
interactionLayer.addEventListener("wheel", onPlanWheel, { passive: false });
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
