// Settings window — the place where anything that writes to disk is confirmed.
// Stage 2 covers the Claude Code hooks and the general preferences; API keys and
// integrations land here too in a later stage.

import "./settings.css";
import { Bridge, onEvent, type HookStatus } from "../core/bridge";
import {
  ARON_SETTINGS_GROUPS,
  aronSettingsFieldsByGroup,
  isSecretWritableSource,
  safeSecretMetadata,
  type AronProvider,
  type AronSecretStatus,
  type AronSettingsKey,
  type AronSettingsPrefs,
  type AronSettingsSchemaField,
  type AronSettingsSnapshot,
  type HermesProfileRole,
} from "../core/aron-settings";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { h, clear } from "../views/dom";
import { cameraOptions, cameraState } from "./aron-camera.js";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";

const root = document.getElementById("settings-root")!;

async function save() {
  await Bridge.saveSettings(settings);
}

// ── Reusable bits ─────────────────────────────────────────────────────────────

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement {
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
  el.addEventListener("click", () => {
    const next = !el.classList.contains("on");
    el.classList.toggle("on", next);
    el.setAttribute("aria-pressed", String(next));
    onChange(next);
  });
  return el;
}

function statusDot(ok: boolean): HTMLElement {
  return h("i", { class: "dot", style: `background:${ok ? "#22c55e" : "#f4505e"}` });
}

function renderDiff(text: string): HTMLElement {
  const box = h("div", { class: "diff" });
  for (const line of text.split("\n")) {
    const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
    box.append(h("div", { class: cls, text: line }));
  }
  return box;
}

// ── Claude Code section ───────────────────────────────────────────────────────

function claudeSection(status: HookStatus): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const section = h(
    "section",
    {},
    h("h2", {}, statusDot(status.installed), h("span", { text: "Claude Code" })),
    body,
  );

  const rebuild = async () => {
    const fresh = await Bridge.hooksStatus();
    if (fresh) Object.assign(status, fresh);
    clear(body);
    draw();
    const head = section.querySelector("h2")!;
    clear(head);
    head.append(statusDot(status.installed), h("span", { text: "Claude Code" }));
  };

  function draw() {
    body.append(
      h("div", {
        class: "hint",
        text: status.installed
          ? "Coucou is hooked into your Claude Code sessions. Tool calls, questions and permission requests show up in the island, and you can answer them there."
          : "Install the hooks to see your Claude Code sessions in the island and approve permissions without leaving what you are doing.",
      }),
      h("div", { class: "row" },
        h("label", { text: "settings.json" }),
        h("span", { class: "path", text: status.settingsPath }),
      ),
      h("div", { class: "row" },
        h("label", { text: "Relay" }),
        h("span", { class: "path", text: status.hookPath }),
        statusDot(status.hookReady),
      ),
    );

    if (!status.hookReady) {
      body.append(h("div", {
        class: "notice warn",
        text: "coucou-hook.exe is not in place yet. Restart Coucou; if it still fails, build it with `cargo build -p coucou-hook`.",
      }));
    }

    const actions = h("div", { class: "row" });
    const install = h("button", {
      class: "primary",
      text: status.installed ? "Reinstall hooks…" : "Install hooks…",
      onclick: () => showPreview(true),
    });
    // Writing hook commands that point at a relay which isn't there would give
    // every Claude Code session a broken hook and nothing to show for it.
    if (!status.hookReady) {
      install.disabled = true;
      install.title = "The relay isn't installed yet.";
    }
    actions.append(install);
    if (status.installed) {
      actions.append(h("button", {
        class: "danger",
        text: "Uninstall hooks…",
        onclick: () => showPreview(false),
      }));
    }
    body.append(actions);
  }

  async function showPreview(install: boolean) {
    let preview;
    try {
      preview = await Bridge.hooksPreview(install);
    } catch (err) {
      // An unreadable or invalid settings.json stops here rather than being
      // treated as empty and written over.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: String(err).replace(/^Error:\s*/, "") }),
        h("div", { class: "row" }, h("button", {
          text: "Back",
          onclick: () => { clear(body); draw(); },
        })),
      );
      return;
    }
    if (!preview) return;
    clear(body);
    body.append(
      h("div", {
        class: "hint",
        text: install
          ? "This is exactly what will change in your settings.json. Your own hooks are left untouched."
          : "This removes Coucou's entries only. Your own hooks are left untouched.",
      }),
      renderDiff(preview.diff),
      h("div", { class: "row" },
        h("span", { class: "path", text: `Backup → ${preview.backup}` }),
      ),
    );
    const confirm = h("button", {
      class: install ? "primary" : "danger",
      text: install ? "Back up and write" : "Back up and remove",
    });
    confirm.addEventListener("click", async () => {
      confirm.disabled = true;
      try {
        const backup = await Bridge.hooksApply(install, preview.fingerprint);
        clear(body);
        body.append(h("div", {
          class: "notice ok",
          text: `Done. Previous settings saved as ${backup}. Open a new Claude Code session to pick the hooks up.`,
        }));
        window.setTimeout(() => void rebuild(), 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: `Could not write: ${String(err)}` }));
      }
    });
    body.append(h("div", { class: "row" }, confirm, h("button", {
      text: "Cancel",
      onclick: () => { clear(body); draw(); },
    })));
  }

  draw();
  return section;
}

// ── Claude API section ────────────────────────────────────────────────────────

const MODELS: [string, string][] = [
  ["claude-opus-5", "Claude Opus 5"],
  ["claude-sonnet-5", "Claude Sonnet 5"],
  ["claude-haiku-4-5", "Claude Haiku 4.5"],
];

function apiSection(hasKey: boolean): HTMLElement {
  const dot = statusDot(hasKey);
  const state = h("span", { class: "hint", text: hasKey ? "Key saved in the Windows Credential Manager." : "No key yet — the chat needs one." });

  const field = h("input", {
    type: "password",
    placeholder: hasKey ? "••••••••••••  (stored)" : "sk-ant-...",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;

  const saveBtn = h("button", { class: "primary", text: "Save key" });
  const clearBtn = h("button", { class: "danger", text: "Remove" });
  const feedback = h("div", {});

  async function refresh() {
    const present = (await Bridge.secretPresent("anthropic-api-key")) ?? false;
    dot.style.background = present ? "#22c55e" : "#f4505e";
    state.textContent = present
      ? "Key saved in the Windows Credential Manager."
      : "No key yet — the chat needs one.";
    field.placeholder = present ? "••••••••••••  (stored)" : "sk-ant-...";
    clearBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet("anthropic-api-key", value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: "Saved. It never touches disk." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not save: ${String(err)}` }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear("anthropic-api-key");
      feedback.append(h("div", { class: "notice ok", text: "Key removed." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not remove: ${String(err)}` }));
    }
  });

  const model = h("select", {}) as HTMLSelectElement;
  for (const [id, label] of MODELS) model.append(h("option", { value: id, text: label }));
  if (!MODELS.some(([id]) => id === settings.model)) {
    model.append(h("option", { value: settings.model, text: settings.model }));
  }
  model.value = settings.model;
  model.addEventListener("change", () => {
    settings.model = model.value;
    void save();
  });

  clearBtn.style.display = hasKey ? "" : "none";

  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: "Claude" })),
    state,
    h("div", { class: "row" }, h("label", { text: "API key" }), field, saveBtn, clearBtn),
    h("div", { class: "row" }, h("label", { text: "Model" }), model),
    feedback,
  );
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. */
  fields: { key: string; label: string; placeholder: string; secret: boolean }[];
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", label: "Secret key", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", label: "Token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "Token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "Instance URL", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "API key", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "API key", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "Integration token", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "API key", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = `Pick up to ${MAX_ACTIVE} pills to show next to Mochi — ${used}/${MAX_ACTIVE} in use. Keys are stored in the Windows Credential Manager, never on disk.`;
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
    sw.addEventListener("click", () => {
      const on = settings.activeIntegrations.includes(def.id);
      if (on) {
        settings.activeIntegrations = settings.activeIntegrations.filter((x) => x !== def.id);
      } else {
        if (settings.activeIntegrations.length >= MAX_ACTIVE) return;
        settings.activeIntegrations = [...settings.activeIntegrations, def.id];
      }
      sw.classList.toggle("on", !on);
      updateNote();
      void save();
    });

    const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px;flex:1 1 auto;min-width:0" });
    for (const field of def.fields) {
      const input = h("input", {
        type: field.secret ? "password" : "text",
        placeholder: present[field.key] ? "••••••••  (stored)" : field.placeholder,
        autocomplete: "off",
        spellcheck: "false",
        style: "flex:1 1 auto;min-width:0",
      }) as HTMLInputElement;
      const saveBtn = h("button", { text: "Save" });
      const dotEl = statusDot(present[field.key] ?? false);
      saveBtn.addEventListener("click", async () => {
        const value = input.value.trim();
        try {
          await Bridge.secretSet(field.key, value);
          present[field.key] = value.length > 0;
          input.value = "";
          input.placeholder = value ? "••••••••  (stored)" : field.placeholder;
          dotEl.style.background = value ? "#22c55e" : "#f4505e";
        } catch {
          dotEl.style.background = "#f5a524";
        }
      });
      rows.append(
        h("div", { class: "row" },
          h("label", { style: "min-width:104px", text: field.label }),
          input, saveBtn, dotEl,
        ),
      );
    }

    list.append(
      h("div", { style: "display:flex;gap:12px;align-items:flex-start" },
        h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px;padding-top:4px" },
          sw,
          h("i", { class: "dot", style: `background:${def.color}` }),
          h("span", { style: "font-size:12.5px", text: def.name }),
        ),
        rows,
      ),
    );
  }

  updateNote();
  return h("section", {}, h("h2", {}, h("span", { text: "Integrations" })), note, list);
}

// ── General section ───────────────────────────────────────────────────────────

function generalSection(): HTMLElement {
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    value: String(settings.soundVolume),
  }) as HTMLInputElement;
  volume.addEventListener("input", () => {
    settings.soundVolume = Number(volume.value);
    void save();
  });

  const autoClose = h("input", {
    type: "number", min: "5", max: "120", step: "1",
    value: String(Math.round(settings.autoCloseInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  autoClose.addEventListener("change", () => {
    settings.autoCloseInterval = Math.max(5, Math.min(120, Number(autoClose.value) || 15));
    autoClose.value = String(settings.autoCloseInterval);
    void save();
  });

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: "Main display" }),
    h("option", { value: "cursor", text: "Display under the cursor" }),
  );
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    settings.screen = screen.value as Settings["screen"];
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "General" })),
    h("div", { class: "row" },
      h("label", { text: "Sound" }),
      toggle(settings.soundEnabled, (v) => { settings.soundEnabled = v; void save(); }),
      volume,
    ),
    h("div", { class: "row" },
      h("label", { text: "Auto-close" }),
      autoClose,
      h("span", { class: "hint", text: "seconds after you leave the island" }),
    ),
    h("div", { class: "row" },
      h("label", { text: "Island lives on" }),
      screen,
    ),
    h("div", { class: "row" },
      h("label", { text: "Launch at startup" }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
  );
}

// ── ARON settings — separate store, loaded and updated only through Tauri ─────

type AronPreferenceValue = AronSettingsPrefs[keyof AronSettingsPrefs];

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function credentialErrorMessage(error: unknown, value: string): string {
  const message = errorMessage(error);
  return value ? message.replaceAll(value, "[redacted]") : message;
}

function showNotice(target: HTMLElement, kind: "ok" | "err" | "warn", text: string) {
  clear(target);
  target.append(h("div", { class: `notice ${kind}`, text }));
}

function aronSettingsSection(initial: AronSettingsSnapshot | null, loadError?: unknown): HTMLElement {
  const section = h("section", { class: "aron-settings" });
  section.append(
    h("h2", {}, h("span", { text: "A.R.O.N. settings" })),
    h("div", {
      class: "hint",
      text: "These preferences and credentials are stored by ARON, separately from Coucou settings.",
    }),
  );

  if (!initial) {
    const feedback = h("div", {});
    showNotice(feedback, "err", `Could not load ARON settings: ${errorMessage(loadError)}`);
    const retry = h("button", { class: "primary", text: "Retry loading" });
    retry.addEventListener("click", async () => {
      retry.disabled = true;
      try {
        section.replaceWith(aronSettingsSection(await Bridge.loadAronSettings()));
      } catch (error) {
        showNotice(feedback, "err", `Could not load ARON settings: ${errorMessage(error)}`);
        retry.disabled = false;
      }
    });
    section.append(feedback, h("div", { class: "row" }, retry));
    return section;
  }

  let snapshot = initial;
  let draft: AronSettingsPrefs = structuredClone(initial.prefs);
  let patch: Partial<AronSettingsPrefs> = {};
  let discoveredCameras: MediaDeviceInfo[] = [];
  let cameraMessage = "Camera access is requested only when you choose Detect cameras.";
  let cameraMessageKind: "hint" | "ok" | "warn" | "err" = "hint";

  const feedback = h("div", {});
  const saveButton = h("button", { class: "primary", text: "Save ARON settings" });
  const providers = h("div", { class: "aron-providers" });
  const fieldsHost = h("div", { class: "aron-groups" });
  saveButton.disabled = true;

  function updatePreference(key: AronSettingsKey, value: AronPreferenceValue) {
    draft = { ...draft, [key]: value } as AronSettingsPrefs;
    const nextPatch = { ...patch };
    if (JSON.stringify(value) === JSON.stringify(snapshot.prefs[key])) {
      delete nextPatch[key];
    } else {
      Object.assign(nextPatch, { [key]: value });
    }
    patch = nextPatch;
    saveButton.disabled = Object.keys(patch).length === 0;
    clear(feedback);
  }

  function renderSecretControl(provider: AronProvider): HTMLElement {
    const source = provider === "jev" ? "Jev" : "Gemini";
    const input = h("input", {
      type: "password",
      placeholder: provider === "jev" ? "Jev API key" : "Gemini API key",
      autocomplete: "new-password",
      spellcheck: "false",
      "aria-label": `${source} API key`,
    }) as HTMLInputElement;
    const labelInput = provider === "gemini"
      ? h("input", {
        type: "text",
        placeholder: "Label for this key",
        autocomplete: "off",
        "aria-label": "Gemini key label",
      }) as HTMLInputElement
      : null;
    const state = h("div", { class: "aron-secret-state" });
    const feedback = h("div", {});
    const card = h("div", { class: "aron-provider" });
    const save = h("button", { class: "primary", text: provider === "jev" ? "Save Jev key" : "Add Gemini key" });

    function renderStatus(rawStatus: AronSecretStatus) {
      const status = safeSecretMetadata(rawStatus);
      clear(state);
      state.append(h("div", {
        class: "hint",
        text: `${source} · ${status.configured ? "Configured" : "Not configured"}`,
      }));
      for (const entry of status.entries) {
        const row = h("div", { class: "aron-secret-entry" },
          h("span", { text: `${entry.source} · ${entry.label} · ending in ${entry.last4}` }),
        );
        if (isSecretWritableSource(entry.source)) {
          const remove = h("button", { class: "danger", text: "Remove" });
          remove.addEventListener("click", async () => {
            remove.disabled = true;
            clear(feedback);
            try {
              renderStatus(await Bridge.aronSecretRemove(provider, entry.id));
              showNotice(feedback, "ok", `${source} credential removed.`);
            } catch (error) {
              remove.disabled = false;
              showNotice(feedback, "err", `Could not remove ${source} credential: ${errorMessage(error)}`);
            }
          });
          row.append(remove);
        }
        state.append(row);
      }
    }

    save.addEventListener("click", async () => {
      const value = input.value;
      const label = provider === "jev" ? "Jev" : labelInput?.value.trim() ?? "";
      if (!value.trim()) {
        showNotice(feedback, "err", `Enter a ${source} API key before saving.`);
        return;
      }
      if (!label) {
        showNotice(feedback, "err", "Enter a label for this Gemini key.");
        return;
      }
      save.disabled = true;
      clear(feedback);
      try {
        renderStatus(await Bridge.aronSecretAdd(provider, value, label));
        input.value = "";
        if (labelInput) labelInput.value = "";
        showNotice(feedback, "ok", `${source} credential saved in ARON's credential store.`);
      } catch (error) {
        showNotice(feedback, "err", `Could not save ${source} credential: ${credentialErrorMessage(error, value)}`);
      } finally {
        save.disabled = false;
      }
    });

    const controls = h("div", { class: "row aron-secret-inputs" }, input);
    if (labelInput) controls.append(labelInput);
    controls.append(save);
    card.append(
      h("h3", { text: source }),
      state,
      controls,
      feedback,
    );
    const current = snapshot.secrets.find((status) => status.provider === provider);
    if (current) renderStatus(current);
    else state.append(h("div", { class: "hint", text: `${source} status is unavailable.` }));
    return card;
  }

  providers.append(renderSecretControl("jev"), renderSecretControl("gemini"));

  function conflictNotice(message: string, button: HTMLButtonElement) {
    showNotice(feedback, "err", message);
    const reload = h("button", { text: "Reload current settings" });
    reload.addEventListener("click", async () => {
      reload.disabled = true;
      try {
        section.replaceWith(aronSettingsSection(await Bridge.loadAronSettings()));
      } catch (error) {
        reload.disabled = false;
        feedback.append(h("div", {
          class: "notice err",
          text: `Could not reload ARON settings: ${errorMessage(error)}`,
        }));
      }
    });
    feedback.append(h("div", { class: "row" }, reload));
    button.disabled = false;
  }

  function renderAronField(
    key: AronSettingsKey,
    field: AronSettingsSchemaField,
    value: AronPreferenceValue,
  ): HTMLElement {
    const label = h("label", { text: field.title, for: `aron-${key}` });
    const description = h("div", { class: "hint aron-field-description", text: field.description });
    const types = Array.isArray(field.type) ? field.type : [field.type];
    const isReadOnly = !field["x-editable"];

    if (isReadOnly) {
      const text = value == null ? "Not calibrated" : `${Math.round(Number(value) * 100)}%`;
      return h("div", { class: "aron-field" },
        h("div", { class: "row aron-field-row" }, label, h("span", { class: "aron-readonly", text })),
        description,
      );
    }

    if (field["x-control"] === "hermes-profiles") {
      const profiles = value as AronSettingsPrefs["hermes_profiles"];
      const controls = h("div", { class: "aron-role-fields" });
      for (const [role, definition] of Object.entries(field.properties ?? {})) {
        const roleKey = role as HermesProfileRole;
        const roleInput = h("input", {
          id: `aron-${key}-${role}`,
          type: "text",
          maxlength: String(definition.maxLength ?? 128),
          placeholder: "Optional profile ID",
          autocomplete: "off",
          value: profiles[roleKey] ?? "",
        }) as HTMLInputElement;
        roleInput.addEventListener("change", () => {
          const nextProfiles = { ...profiles };
          const profile = roleInput.value.trim();
          if (profile) nextProfiles[roleKey] = profile;
          else delete nextProfiles[roleKey];
          updatePreference(key, nextProfiles);
        });
        controls.append(h("div", { class: "aron-role-field" },
          h("label", { for: `aron-${key}-${role}`, text: definition.title }),
          roleInput,
        ));
      }
      return h("div", { class: "aron-field" },
        h("div", { class: "row aron-field-row" }, label, controls),
        description,
      );
    }

    if (key === "camera_device") {
      const options = cameraOptions(discoveredCameras, {
        camera_device: draft.camera_device,
        camera_label: draft.camera_label,
      });
      const active = options.find((option) => option.active);
      if (!active && draft.camera_device) {
        options.unshift({
          deviceId: draft.camera_device,
          label: draft.camera_label || "Previously selected camera (not detected)",
          active: true,
        });
      }
      const select = h("select", { id: `aron-${key}`, "aria-label": field.title });
      select.append(h("option", { value: "", text: "Automatic camera selection" }));
      for (const option of options) {
        select.append(h("option", {
          value: option.deviceId,
          text: option.label,
        }));
      }
      select.value = options.find((option) => option.active)?.deviceId ?? "";
      select.addEventListener("change", () => {
        const selected = options.find((option) => option.deviceId === select.value);
        updatePreference("camera_device", select.value);
        updatePreference("camera_label", selected?.label ?? "");
      });
      const detect = h("button", { text: "Detect cameras" });
      detect.addEventListener("click", async () => {
        if (!navigator.mediaDevices?.getUserMedia) {
          cameraMessage = "Could not detect cameras: camera access is unavailable in this window.";
          cameraMessageKind = "err";
          drawFields();
          return;
        }
        discoveredCameras = [];
        detect.disabled = true;
        try {
          const stream = await navigator.mediaDevices.getUserMedia({ video: true });
          let stopFailed = false;
          let stopFailure: unknown = null;
          for (const track of stream.getTracks()) {
            try {
              track.stop();
            } catch (error) {
              stopFailed = true;
              stopFailure ??= error;
            }
          }
          if (stopFailed) throw stopFailure;
          discoveredCameras = await navigator.mediaDevices.enumerateDevices();
          const result = cameraState(discoveredCameras, false);
          cameraMessage = result.message;
          cameraMessageKind = result.state === "empty" ? "warn" : "ok";
        } catch (error) {
          const name =
            typeof error === "object" && error !== null && "name" in error
              ? String(error.name)
              : "";
          const result = cameraState([], name === "NotAllowedError" || name === "SecurityError");
          if (result.state === "denied" || name === "NotFoundError") {
            cameraMessage = result.message;
            cameraMessageKind = result.state === "denied" ? "err" : "warn";
          } else {
            cameraMessage = `Could not detect cameras: ${errorMessage(error)}`;
            cameraMessageKind = "err";
          }
        } finally {
          drawFields();
        }
      });
      return h("div", { class: "aron-field" },
        h("div", { class: "row aron-field-row" }, label,
          h("div", { class: "aron-camera-controls" }, select, detect),
        ),
        h("div", { class: `notice ${cameraMessageKind}`, text: cameraMessage }),
        description,
      );
    }

    if (field.enum) {
      const select = h("select", { id: `aron-${key}` });
      for (const choice of field.enum) {
        select.append(h("option", { value: choice, text: choice }));
      }
      if (!field.enum.includes(String(value))) {
        select.append(h("option", { value: String(value), text: `${String(value)} (saved)` }));
      }
      select.value = String(value);
      select.addEventListener("change", () => updatePreference(key, select.value));
      return h("div", { class: "aron-field" },
        h("div", { class: "row aron-field-row" }, label, select),
        description,
      );
    }

    if (types.includes("boolean")) {
      return h("div", { class: "aron-field" },
        h("div", { class: "row aron-field-row" }, label,
          toggle(Boolean(value), (next) => updatePreference(key, next)),
        ),
        description,
      );
    }

    if (types.includes("array")) {
      const textarea = h("textarea", {
        id: `aron-${key}`,
        rows: "3",
        placeholder: "One item per line",
      }) as HTMLTextAreaElement;
      textarea.value = (value as string[]).join("\n");
      textarea.addEventListener("change", () => updatePreference(
        key,
        textarea.value.split(/\r?\n/).filter((line) => line.length > 0),
      ));
      return h("div", { class: "aron-field" },
        h("div", { class: "row aron-field-row" }, label, textarea),
        description,
      );
    }

    if (types.includes("integer") || types.includes("number")) {
      const input = h("input", {
        id: `aron-${key}`,
        type: "number",
        min: field.minimum,
        max: field.maximum,
        step: types.includes("integer") ? "1" : "any",
      }) as HTMLInputElement;
      input.value = String(value);
      input.addEventListener("change", () => {
        if (!input.reportValidity()) return;
        const number = input.valueAsNumber;
        if (Number.isFinite(number)) updatePreference(key, number);
      });
      return h("div", { class: "aron-field" },
        h("div", { class: "row aron-field-row" }, label, input),
        description,
      );
    }

    const input = h("input", {
      id: `aron-${key}`,
      type: "text",
      maxlength: field.maxLength,
      autocomplete: "off",
    }) as HTMLInputElement;
    input.value = String(value);
    input.addEventListener("change", () => updatePreference(key, input.value));
    return h("div", { class: "aron-field" },
      h("div", { class: "row aron-field-row" }, label, input),
      description,
    );
  }

  function drawFields() {
    clear(fieldsHost);
    const grouped = aronSettingsFieldsByGroup();
    for (const group of ARON_SETTINGS_GROUPS) {
      const body = h("div", { class: "aron-group-fields" });
      for (const [key, field] of grouped[group]) {
        body.append(renderAronField(key, field, draft[key]));
      }
      if (group === "Providers") body.append(providers);
      fieldsHost.append(h("section", { class: "aron-group" },
        h("h3", { text: group }),
        body,
      ));
    }
  }

  saveButton.addEventListener("click", async () => {
    if (Object.keys(patch).length === 0) return;
    saveButton.disabled = true;
    clear(feedback);
    try {
      snapshot = await Bridge.updateAronSettings({ prefs: patch, revision: snapshot.revision });
      draft = structuredClone(snapshot.prefs);
      patch = {};
      showNotice(feedback, "ok", "ARON settings saved.");
      drawFields();
    } catch (error) {
      const message = errorMessage(error);
      if (/changed since they were loaded|revision|conflict/i.test(message)) {
        conflictNotice(`ARON settings changed elsewhere. ${message}`, saveButton);
      } else {
        showNotice(feedback, "err", `Could not save ARON settings: ${message}`);
        saveButton.disabled = false;
      }
    }
  });

  drawFields();
  section.append(
    h("div", { class: "aron-save-row" }, saveButton),
    feedback,
    fieldsHost,
  );
  return section;
}

// ── Boot ──────────────────────────────────────────────────────────────────────

async function main() {
  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }
  const status = (await Bridge.hooksStatus()) ?? {
    installed: false, settingsPath: "", hookPath: "", hookReady: false,
  };

  const hasKey = (await Bridge.secretPresent("anthropic-api-key")) ?? false;

  const keys = [
    "stripe-api-key", "github-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "calcom-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  let aronSnapshot: AronSettingsSnapshot | null = null;
  let aronLoadError: unknown;
  try {
    aronSnapshot = await Bridge.loadAronSettings();
  } catch (error) {
    aronLoadError = error;
  }

  clear(root);
  root.append(
    h("h1", {}, h("span", { text: "Coucou" }), h("span", { class: "version", text: version })),
    aronSettingsSection(aronSnapshot, aronLoadError),
    claudeSection(status),
    apiSection(hasKey),
    integrationsSection(present),
    generalSection(),
    h("div", {
      class: "hint",
      text: "No telemetry. Network requests only go to the services you configure yourself.",
    }),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
  });
}

void main();
