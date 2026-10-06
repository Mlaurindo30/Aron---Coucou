import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";

const schemaPath = new URL("../src/settings/aron-settings-schema.json", import.meta.url);
const settingsPath = new URL("../src/core/aron-settings.ts", import.meta.url);
const uiPath = new URL("../src/settings/main.ts", import.meta.url);
const schema = existsSync(schemaPath)
  ? JSON.parse(readFileSync(schemaPath, "utf8"))
  : null;
const settings = existsSync(settingsPath)
  ? await import("../src/core/aron-settings.ts")
  : {};
const camera = existsSync(new URL("../src/settings/aron-camera.js", import.meta.url))
  ? await import("../src/settings/aron-camera.js")
  : {};

const preferenceKeys = [
  "voice",
  "engine",
  "theme",
  "orb_style",
  "volume",
  "auto_pause",
  "auto_pause_seconds",
  "wake_enabled",
  "wake_name",
  "wake_aliases",
  "wake_phrases",
  "wake_max_words",
  "wake_fuzzy",
  "decision_engine",
  "wake_threshold_laya_multilingual",
  "wake_threshold_jev",
  "local_engine",
  "stt_model",
  "stt_device",
  "camera_device",
  "camera_label",
  "history_enabled",
  "history_retention_days",
  "history_store_replies",
  "hermes_profiles",
];
const groups = [
  "Providers",
  "Hermes Routing",
  "Voice/Wake",
  "Vision",
  "Privacy/History",
  "Appearance",
];

test("schema covers every ARON preference with defaults and UI metadata", () => {
  assert.ok(schema, "the shared ARON settings schema must exist");
  const properties = schema.properties;
  assert.deepEqual(Object.keys(properties).sort(), [...preferenceKeys].sort());
  assert.deepEqual(schema.required, []);

  for (const [key, field] of Object.entries(properties)) {
    assert.ok(Object.hasOwn(field, "default"), `${key} needs a default`);
    assert.ok(
      typeof field.type === "string" ||
        (Array.isArray(field.type) && field.type.every((type) => typeof type === "string")),
      `${key} needs a JSON type`,
    );
    assert.ok(groups.includes(field["x-uiGroup"]), `${key} needs a known UI group`);
    assert.equal(typeof field["x-editable"], "boolean", `${key} needs editability metadata`);
    if (field.enum) assert.ok(Array.isArray(field.enum), `${key} enum must be an array`);
    if (field.minimum !== undefined || field.maximum !== undefined) {
      assert.equal(typeof field.minimum, "number", `${key} needs a minimum`);
      assert.equal(typeof field.maximum, "number", `${key} needs a maximum`);
    }
  }

  assert.deepEqual(
    Object.entries(properties)
      .filter(([, field]) => !field["x-editable"])
      .map(([key]) => key)
      .sort(),
    ["wake_threshold_jev", "wake_threshold_laya_multilingual"],
  );
  assert.deepEqual(
    Object.keys(properties.hermes_profiles.properties).sort(),
    ["hermes_profile", "hermes_squad"],
  );
});

test("schema-driven group mapping covers every schema field exactly once", () => {
  assert.ok(schema, "the shared ARON settings schema must exist");
  assert.equal(typeof settings.aronSettingsFieldsByGroup, "function");

  const grouped = settings.aronSettingsFieldsByGroup();
  assert.deepEqual(Object.keys(grouped), groups);
  const mapped = Object.entries(grouped).flatMap(([group, fields]) =>
    fields.map(([key]) => [key, group]),
  );
  assert.deepEqual(
    mapped.map(([key]) => key).sort(),
    Object.keys(schema.properties).sort(),
  );
  for (const [key, group] of mapped) {
    assert.equal(schema.properties[key]["x-uiGroup"], group, `${key} is in its schema group`);
  }

  const ui = readFileSync(uiPath, "utf8");
  assert.match(ui, /aronSettingsFieldsByGroup/);
  assert.match(ui, /renderAronField/);
});

test("secret metadata projection never exposes credential values", () => {
  assert.equal(typeof settings.safeSecretMetadata, "function");
  const rawSecret = "private-credential-value";
  const metadata = settings.safeSecretMetadata({
    provider: "gemini",
    configured: true,
    entries: [{
      id: "gemini#abc123",
      label: "Work account",
      last4: "alue",
      source: "env",
      value: rawSecret,
    }],
    value: rawSecret,
  });

  assert.deepEqual(metadata, {
    provider: "gemini",
    configured: true,
    entries: [{ id: "gemini#abc123", label: "Work account", last4: "alue", source: "env" }],
  });
  assert.equal(JSON.stringify(metadata).includes(rawSecret), false);
  assert.equal(typeof settings.isSecretWritableSource, "function");
  assert.equal(settings.isSecretWritableSource("aron"), true);
  assert.equal(settings.isSecretWritableSource("store"), true);
  assert.equal(settings.isSecretWritableSource("env"), false);
  assert.equal(settings.isSecretWritableSource("unknown"), false);

  const ui = readFileSync(uiPath, "utf8");
  assert.match(ui, /isSecretWritableSource\(entry\.source\)/);
});

test("camera options prefer saved IDs and fall back to an exact saved label", () => {
  assert.equal(typeof camera.cameraOptions, "function");
  const devices = [
    { kind: "videoinput", deviceId: "new-id", label: "Desk camera" },
    { kind: "videoinput", deviceId: "other-id", label: "Room camera" },
    { kind: "audioinput", deviceId: "mic-id", label: "Desk camera" },
  ];

  assert.deepEqual(
    camera.cameraOptions(devices, { camera_device: "new-id", camera_label: "Room camera" }),
    [
      { deviceId: "new-id", label: "Desk camera", active: true },
      { deviceId: "other-id", label: "Room camera", active: false },
    ],
  );
  assert.deepEqual(
    camera.cameraOptions(devices, { camera_device: "removed-id", camera_label: "Room camera" }),
    [
      { deviceId: "new-id", label: "Desk camera", active: false },
      { deviceId: "other-id", label: "Room camera", active: true },
    ],
  );
});

test("camera detection reports no-device and permission-denied states", () => {
  assert.equal(typeof camera.cameraState, "function");
  assert.deepEqual(camera.cameraState([], false), {
    state: "empty",
    message: "No cameras found.",
  });
  assert.deepEqual(camera.cameraState([], true), {
    state: "denied",
    message: "Camera permission was denied. Allow access and try again.",
  });
});
