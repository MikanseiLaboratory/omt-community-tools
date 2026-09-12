import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

const DOCS_URL = "https://github.com/MikanseiLaboratory/omt-community-tools#readme";

type ToolCard = {
  id: string;
  title: string;
  description: string;
  binary: string;
  enabled: boolean;
  available: boolean;
  version: string;
};

type Labels = {
  title: string;
  subtitle: string;
  settings: string;
  docs: string;
  save: string;
  language: string;
  theme: string;
  version: string;
  launch: string;
  launching: string;
  back: string;
  themeLight: string;
  themeDark: string;
  themeSystem: string;
  unavailable: string;
  simd: string;
  updateCheck: string;
  updateChecking: string;
  updateAvailableTitle: string;
  updateAvailableBody: string;
  updateInstall: string;
  updateLater: string;
  updateNone: string;
  updateInstalling: string;
  updateFailed: string;
};

type LauncherState = {
  language: string;
  theme: string;
  suiteVersion: string;
  simd: string;
  labels: Labels;
  tools: ToolCard[];
  manifest: {
    suite_version: string;
    target: string;
    tools: Array<{ id: string; version: string; binary: string; enabled: boolean }>;
  };
};

type ToolVisual = {
  accent: string;
  icon: string;
};

const TOOL_VISUALS: Record<string, ToolVisual> = {
  "studio-monitor": {
    accent: "#4da3ff",
    icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="5" width="18" height="12" rx="2"/><path d="M8 21h8M12 17v4"/></svg>`,
  },
  "test-patterns": {
    accent: "#ff4fa3",
    icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 5h16v14H4z"/><path d="M8 5v14M12 5v14M16 5v14M4 9h16M4 13h16"/></svg>`,
  },
  "config-manager": {
    accent: "#5ad1a4",
    icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 5h16v14H4z"/><path d="M8 9h8M8 12h8M8 15h5"/></svg>`,
  },
  "discovery-server": {
    accent: "#c58cff",
    icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="12" r="3"/><path d="M12 5v2M12 17v2M5 12h2M17 12h2M7 7l1.5 1.5M15.5 15.5L17 17M7 17l1.5-1.5M15.5 8.5L17 7"/></svg>`,
  },
};

let state: LauncherState | null = null;
let settingsOpen = false;
let updateDialogOpen = false;
let checkingUpdate = false;
let installingUpdate = false;
let pendingUpdate: Update | null = null;
/** toolId -> clearTimeout handle while launch feedback is visible */
const launchingTimers = new Map<string, number>();
/** Keep tile feedback after spawn() returns — window paint usually lags. */
const LAUNCH_FEEDBACK_MS = 4500;

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el;
}

function applyTheme(theme: string) {
  const root = document.documentElement;
  if (theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.dataset.theme = theme;
  }
}

function renderLabels(labels: Labels) {
  $("title-text").textContent = labels.title;
  $("docs-label").textContent = labels.docs;
  $("settings-btn").setAttribute("aria-label", labels.settings);
  $("settings-title").textContent = labels.settings;
  $("language-label").textContent = labels.language;
  $("theme-label").textContent = labels.theme;
  $("version-label").textContent = labels.version;
  $("footer-version-label").textContent = labels.version;
  $("simd-label").textContent = labels.simd;
  $("save-settings").textContent = labels.save;
  $("check-update").textContent = labels.updateCheck;
  $("update-title").textContent = labels.updateAvailableTitle;
  $("update-later").textContent = labels.updateLater;
  $("update-confirm").textContent = labels.updateInstall;

  const themeSelect = $("theme-select") as HTMLSelectElement;
  themeSelect.options[0].text = labels.themeSystem;
  themeSelect.options[1].text = labels.themeLight;
  themeSelect.options[2].text = labels.themeDark;
}

function toolVisual(id: string): ToolVisual {
  return (
    TOOL_VISUALS[id] ?? {
      accent: "#888888",
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="5" y="5" width="14" height="14" rx="3"/></svg>`,
    }
  );
}

function setTileLaunching(tile: HTMLButtonElement, launching: boolean, label: string) {
  tile.classList.toggle("is-launching", launching);
  tile.disabled = launching || tile.classList.contains("is-disabled");
  const status = tile.querySelector(".tool-launch-status");
  if (status) {
    status.textContent = launching ? label : "";
  }
  tile.setAttribute("aria-busy", launching ? "true" : "false");
}

function clearLaunchFeedback(toolId: string) {
  const timer = launchingTimers.get(toolId);
  if (timer !== undefined) {
    window.clearTimeout(timer);
    launchingTimers.delete(toolId);
  }
  const tile = document.querySelector<HTMLButtonElement>(`.tool-tile[data-tool="${toolId}"]`);
  if (tile && state) {
    setTileLaunching(tile, false, state.labels.launching);
  }
}

function renderTools(tools: ToolCard[], labels: Labels) {
  const grid = $("tool-grid");
  grid.innerHTML = "";

  for (const tool of tools) {
    const canLaunch = tool.enabled && tool.available;
    const visual = toolVisual(tool.id);
    const wasLaunching = launchingTimers.has(tool.id);
    const tile = document.createElement("button");
    tile.type = "button";
    tile.className = `tool-tile${canLaunch ? "" : " is-disabled"}${wasLaunching ? " is-launching" : ""}`;
    tile.style.setProperty("--accent-color", visual.accent);
    tile.dataset.tool = tool.id;
    tile.disabled = !canLaunch || wasLaunching;
    tile.setAttribute("aria-busy", wasLaunching ? "true" : "false");
    tile.setAttribute("aria-label", `${tool.title}. ${tool.description}`);
    tile.innerHTML = `
      <span class="tool-icon" aria-hidden="true">${visual.icon}</span>
      <span class="tool-copy">
        <span class="tool-name">${tool.title}</span>
        <span class="tool-desc">${tool.description}</span>
        <span class="tool-launch-status">${wasLaunching ? labels.launching : ""}</span>
      </span>
      <span class="tool-spinner" aria-hidden="true"></span>
    `;
    if (canLaunch) {
      tile.addEventListener("click", async () => {
        if (launchingTimers.has(tool.id)) return;
        hideToast();
        setTileLaunching(tile, true, labels.launching);
        // Placeholder until spawn returns; blocks double-clicks during invoke.
        launchingTimers.set(tool.id, 0);
        const started = Date.now();
        try {
          await invoke("launch_tool", { toolId: tool.id });
          const remaining = Math.max(0, LAUNCH_FEEDBACK_MS - (Date.now() - started));
          const timer = window.setTimeout(() => {
            launchingTimers.delete(tool.id);
            setTileLaunching(tile, false, labels.launching);
          }, remaining);
          launchingTimers.set(tool.id, timer);
        } catch (err) {
          clearLaunchFeedback(tool.id);
          setTileLaunching(tile, false, labels.launching);
          showToast(String(err));
        }
      });
    }
    grid.appendChild(tile);
  }
}

function renderManifest(current: LauncherState) {
  $("suite-version").textContent = current.suiteVersion;
  $("footer-version").textContent = current.suiteVersion;
  const lines = current.manifest.tools.map(
    (t) => `${String(t.id)}  ${t.version}  (${t.enabled ? "enabled" : "preview"})`,
  );
  $("manifest-tools").textContent = lines.join("\n");
  $("simd-info").textContent = current.simd;
}

function showToast(message: string) {
  const el = $("status");
  el.textContent = message;
  el.classList.toggle("hidden", !message);
}

function hideToast() {
  showToast("");
}

function setSettingsOpen(open: boolean) {
  settingsOpen = open;
  $("settings-panel").classList.toggle("hidden", !open);
  $("settings-backdrop").classList.toggle("hidden", !open);
  $("settings-btn").setAttribute("aria-expanded", open ? "true" : "false");
}

function formatLabel(template: string, vars: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (_, key: string) => vars[key] ?? "");
}

function setUpdateDialogOpen(open: boolean) {
  updateDialogOpen = open;
  $("update-dialog").classList.toggle("hidden", !open);
  $("update-backdrop").classList.toggle("hidden", !open);
  if (!open && !installingUpdate) {
    pendingUpdate = null;
  }
}

function showUpdateDialog(update: Update) {
  if (!state) return;
  pendingUpdate = update;
  const confirmBtn = $("update-confirm") as HTMLButtonElement;
  const laterBtn = $("update-later") as HTMLButtonElement;
  confirmBtn.disabled = false;
  laterBtn.disabled = false;
  $("update-title").textContent = state.labels.updateAvailableTitle;
  $("update-body").textContent = formatLabel(state.labels.updateAvailableBody, {
    version: update.version,
  });
  const notes = (update.body ?? "").trim();
  const notesEl = $("update-notes");
  notesEl.textContent = notes;
  notesEl.classList.toggle("hidden", !notes);
  setSettingsOpen(false);
  setUpdateDialogOpen(true);
}

async function checkForUpdate(origin: "auto" | "manual") {
  if (!state || checkingUpdate || installingUpdate || updateDialogOpen) return;
  checkingUpdate = true;
  const statusEl = $("update-status");
  const checkBtn = $("check-update") as HTMLButtonElement;
  if (origin === "manual") {
    statusEl.textContent = state.labels.updateChecking;
    checkBtn.disabled = true;
  }
  try {
    const update = await check();
    if (!update) {
      if (origin === "manual") {
        statusEl.textContent = state.labels.updateNone;
      }
      return;
    }
    if (origin === "manual") {
      statusEl.textContent = "";
    }
    showUpdateDialog(update);
  } catch (err) {
    if (origin === "manual") {
      statusEl.textContent = `${state.labels.updateFailed} ${String(err)}`;
    }
  } finally {
    checkingUpdate = false;
    checkBtn.disabled = false;
  }
}

async function installPendingUpdate() {
  if (!pendingUpdate || !state || installingUpdate) return;
  installingUpdate = true;
  const confirmBtn = $("update-confirm") as HTMLButtonElement;
  const laterBtn = $("update-later") as HTMLButtonElement;
  confirmBtn.disabled = true;
  laterBtn.disabled = true;
  $("update-body").textContent = state.labels.updateInstalling;
  try {
    await pendingUpdate.downloadAndInstall();
    await relaunch();
  } catch (err) {
    installingUpdate = false;
    confirmBtn.disabled = false;
    laterBtn.disabled = false;
    setUpdateDialogOpen(false);
    showToast(String(err));
  }
}

async function refresh() {
  state = await invoke<LauncherState>("get_launcher_state");
  applyTheme(state.theme);
  renderLabels(state.labels);
  renderTools(state.tools, state.labels);
  renderManifest(state);
  ($("language-select") as HTMLSelectElement).value = state.language;
  ($("theme-select") as HTMLSelectElement).value = state.theme;
  document.documentElement.lang = state.language === "ja" ? "ja" : "en";
}

/** Suppress macOS WebView beep on non-input keypresses. */
function installBeepWorkaround() {
  window.addEventListener(
    "keydown",
    (event) => {
      const target = event.composedPath()[0];
      const isEditable =
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        (target instanceof HTMLElement && target.isContentEditable);
      if (!isEditable) {
        if (event.key === "Escape") {
          if (updateDialogOpen && !installingUpdate) {
            setUpdateDialogOpen(false);
          } else if (settingsOpen) {
            setSettingsOpen(false);
          }
        }
        // Keep Tab for focus movement; suppress WebView beep for other keys.
        if (event.key !== "Tab") {
          event.preventDefault();
        }
      }
    },
    { capture: true },
  );
}

async function syncOsTheme() {
  try {
    const win = getCurrentWindow();
    const theme = await win.theme();
    if (theme && (!state || state.theme === "system")) {
      // CSS already follows prefers-color-scheme when data-theme is unset.
      document.documentElement.style.colorScheme = theme;
    }
    await win.onThemeChanged(({ payload }) => {
      if (!state || state.theme === "system") {
        document.documentElement.style.colorScheme = payload ?? "dark";
      }
    });
  } catch {
    // Running in plain browser / unsupported environment.
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  installBeepWorkaround();

  $("settings-btn").addEventListener("click", () => setSettingsOpen(!settingsOpen));
  $("settings-backdrop").addEventListener("click", () => setSettingsOpen(false));
  $("check-update").addEventListener("click", () => {
    void checkForUpdate("manual");
  });
  $("update-later").addEventListener("click", () => {
    if (!installingUpdate) {
      setUpdateDialogOpen(false);
    }
  });
  $("update-backdrop").addEventListener("click", () => {
    if (!installingUpdate) {
      setUpdateDialogOpen(false);
    }
  });
  $("update-confirm").addEventListener("click", () => {
    void installPendingUpdate();
  });
  $("docs-btn").addEventListener("click", async () => {
    try {
      await openUrl(DOCS_URL);
    } catch (err) {
      showToast(String(err));
    }
  });

  $("save-settings").addEventListener("click", async () => {
    const language = ($("language-select") as HTMLSelectElement).value;
    const theme = ($("theme-select") as HTMLSelectElement).value;
    try {
      state = await invoke<LauncherState>("save_settings", {
        args: { language, theme },
      });
      applyTheme(state.theme);
      renderLabels(state.labels);
      renderTools(state.tools, state.labels);
      renderManifest(state);
      $("settings-status").textContent = "OK";
      setSettingsOpen(false);
    } catch (err) {
      $("settings-status").textContent = String(err);
    }
  });

  try {
    await refresh();
    await syncOsTheme();
    void checkForUpdate("auto");
  } catch (err) {
    showToast(String(err));
  }
});
