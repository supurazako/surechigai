const setupPanel = document.querySelector("#setup-panel");
const storyPanel = document.querySelector("#story-panel");
const historyPanel = document.querySelector("#history-panel");
const setupForm = document.querySelector("#setup-form");
const startButton = document.querySelector("#start-button");
const formError = document.querySelector("#form-error");
const statusLabel = document.querySelector("#status-label");
const statusPill = document.querySelector("#status-pill");
const sentenceGrid = document.querySelector("#sentence-grid");
const renderedSentence = document.querySelector("#rendered-sentence");
const receivedCount = document.querySelector("#received-count");
const progressBar = document.querySelector("#progress-bar");
const completion = document.querySelector("#completion");
const exchangeList = document.querySelector("#exchange-list");
const emptyLog = document.querySelector("#empty-log");
const deviceId = document.querySelector("#device-id");
const imagePanel = document.querySelector("#image-panel");
const imageStatusLabel = document.querySelector("#image-status-label");
const imagePreview = document.querySelector("#image-preview");
const generationForm = document.querySelector("#generation-form");
const generateButton = document.querySelector("#generate-button");
const generationMessage = document.querySelector("#generation-message");

const sentenceOrder = ["when", "how", "who", "where", "why", "what"];
let initialized = false;
let requestPending = false;
let generationPending = false;
let prefilledRound = null;

function setFormValues(setup) {
  if (initialized || !setup) return;
  for (const [key, value] of Object.entries(setup)) {
    const input = setupForm.elements.namedItem(key);
    if (input) input.value = value;
  }
  initialized = true;
}

function statusText(state) {
  if (state.phase === "setup") return "設定待ち";
  if (state.phase === "starting") return "起動中";
  if (state.phase === "stopped") return "停止";
  return state.role || "動作中";
}

function renderStatus(state) {
  statusLabel.textContent = statusText(state);
  statusPill.dataset.phase = state.phase;
  if (state.last_error) {
    statusLabel.textContent = `再試行中: ${state.last_error}`;
    statusPill.dataset.phase = "error";
  }
}

function renderSentence(device) {
  const byKey = new Map(device.slots.map((slot) => [slot.key, slot]));
  sentenceGrid.replaceChildren(
    ...sentenceOrder.map((key, index) => {
      const slot = byKey.get(key);
      const item = document.createElement("li");
      item.className = slot.text ? "fragment is-filled" : "fragment";

      const number = document.createElement("span");
      number.className = "fragment-number";
      number.textContent = String(index + 1).padStart(2, "0");

      const body = document.createElement("div");
      const label = document.createElement("span");
      label.className = "fragment-label";
      label.textContent = slot.label;
      const text = document.createElement("strong");
      text.textContent = slot.text || "待っています…";
      const source = document.createElement("small");
      source.textContent = slot.source_name ? `from ${slot.source_name}` : "未受取";
      body.append(label, text, source);
      item.append(number, body);
      return item;
    }),
  );

  const count = 6 - device.missing_count;
  receivedCount.textContent = String(count);
  progressBar.style.width = `${(count / 6) * 100}%`;
  renderedSentence.textContent = device.rendered || "まだ文節がありません";
  completion.classList.toggle("is-hidden", !device.complete);

  if (device.complete && device.round !== prefilledRound) {
    const input = generationForm.elements.namedItem("sentence");
    if (input && !input.value.trim()) input.value = device.rendered;
    prefilledRound = device.round;
  }
}

function renderHistory(device) {
  deviceId.textContent = `${device.name} / ID ${device.node.slice(0, 8)}`;
  emptyLog.classList.toggle("is-hidden", device.exchanges.length > 0);
  exchangeList.replaceChildren(
    ...device.exchanges.map((exchange) => {
      const item = document.createElement("li");
      const peer = document.createElement("strong");
      peer.textContent = exchange.peer_name;
      const details = document.createElement("span");
      details.textContent = `配布 ${exchange.sent} ／ 受取 ${exchange.received}`;
      const sequence = document.createElement("small");
      sequence.textContent = `#${exchange.sequence}`;
      item.append(peer, details, sequence);
      return item;
    }),
  );
}

const imageStatusText = {
  送信中: "広場サーバへ送信中…",
  queued: "画像生成の順番待ち…",
  working: "画像を生成中…",
  done: "画像ができました",
  error: "画像生成に失敗しました",
  timeout: "画像生成がタイムアウトしました",
  送信失敗: "広場サーバへの送信に失敗しました",
};

function renderImage(device) {
  generateButton.disabled =
    generationPending ||
    device.image_generation_busy ||
    !device.image_generation_enabled;
  if (!device.image_generation_enabled && !generationPending) {
    generationMessage.textContent =
      "画像生成を使うにはCLIを --post-url 付きで起動してください。";
  }
  if (!device.image_status) {
    imagePanel.classList.add("is-hidden");
    return;
  }
  imagePanel.classList.remove("is-hidden");
  imageStatusLabel.textContent =
    imageStatusText[device.image_status] || device.image_status;
  if (device.image_url) {
    imagePreview.src = device.image_url;
    imagePreview.classList.remove("is-hidden");
  } else {
    imagePreview.classList.add("is-hidden");
  }
}

function render(state) {
  setFormValues(state.setup);
  renderStatus(state);
  const running = Boolean(state.device);
  setupPanel.classList.toggle("is-hidden", state.phase !== "setup");
  storyPanel.classList.toggle("is-hidden", !running);
  historyPanel.classList.toggle("is-hidden", !running);
  if (running) {
    renderSentence(state.device);
    renderHistory(state.device);
    renderImage(state.device);
  }
}

async function refresh() {
  try {
    const response = await fetch("/api/state", { cache: "no-store" });
    if (!response.ok) throw new Error(`状態取得に失敗しました (${response.status})`);
    render(await response.json());
  } catch (error) {
    statusLabel.textContent = error.message;
    statusPill.dataset.phase = "error";
  }
}

setupForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (requestPending) return;
  requestPending = true;
  startButton.disabled = true;
  formError.textContent = "";
  const values = Object.fromEntries(new FormData(setupForm));
  try {
    const response = await fetch("/api/start", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(values),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "開始できませんでした");
    await refresh();
  } catch (error) {
    formError.textContent = error.message;
    startButton.disabled = false;
    requestPending = false;
  }
});

generationForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (generationPending) return;
  generationPending = true;
  generateButton.disabled = true;
  generationMessage.textContent = "画像生成サーバへ送信しています…";
  const values = Object.fromEntries(new FormData(generationForm));
  try {
    const response = await fetch("/api/generate", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(values),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "画像生成を開始できませんでした");
    generationMessage.textContent = "受け付けました。生成完了までお待ちください。";
  } catch (error) {
    generationMessage.textContent = error.message;
  } finally {
    generationPending = false;
    await refresh();
  }
});

refresh();
setInterval(refresh, 750);
