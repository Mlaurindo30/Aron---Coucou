import aronSettingsSchema from "../settings/aron-settings-schema.json" with { type: "json" };

export const ARON_SETTINGS_GROUPS = [
  "Providers",
  "Hermes Routing",
  "Voice/Wake",
  "Vision",
  "Privacy/History",
  "Appearance",
] as const;

export type AronSettingsGroup = (typeof ARON_SETTINGS_GROUPS)[number];
export type HermesProfileRole = "hermes_profile" | "hermes_squad";
export type AronProvider = "jev" | "gemini";

export interface AronSettingsPrefs {
  voice: string;
  engine: "gemini-live";
  theme: "azul" | "ciano" | "verde" | "violeta" | "ambar" | "vermelho";
  orb_style: "hud" | "fluxo" | "hibrido";
  volume: number;
  auto_pause: boolean;
  auto_pause_seconds: number;
  wake_enabled: boolean;
  wake_name: string;
  wake_aliases: string[];
  wake_phrases: string[];
  wake_max_words: number;
  wake_fuzzy: number;
  decision_engine: "laya" | "jev";
  wake_threshold_laya_multilingual: number | null;
  wake_threshold_jev: number | null;
  local_engine: "laya" | "nimble";
  stt_model: "tiny" | "base" | "small";
  stt_device: "auto" | "cpu" | "gpu";
  camera_device: string;
  camera_label: string;
  history_enabled: boolean;
  history_retention_days: number;
  history_store_replies: boolean;
  hermes_profiles: Partial<Record<HermesProfileRole, string>>;
}

export interface AronSettingsSchemaField {
  title: string;
  description: string;
  type: string | string[];
  default: unknown;
  enum?: string[];
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  items?: { type: string };
  properties?: Record<string, { title: string; type: string; minLength?: number; maxLength?: number }>;
  additionalProperties?: boolean;
  "x-control"?: string;
  "x-uiGroup": string;
  "x-editable": boolean;
}

export type AronSettingsKey = keyof AronSettingsPrefs;
type AronSettingsEntry = [AronSettingsKey, AronSettingsSchemaField];

interface AronSettingsSchema {
  properties: Record<AronSettingsKey, AronSettingsSchemaField>;
}

export const ARON_SETTINGS_SCHEMA: AronSettingsSchema = aronSettingsSchema;

export interface AronSecretMetadata {
  id: string;
  label: string;
  last4: string;
  source: string;
}

export interface AronSecretStatus {
  provider: AronProvider;
  configured: boolean;
  entries: AronSecretMetadata[];
}

export interface AronSettingsSnapshot {
  prefs: AronSettingsPrefs;
  secrets: AronSecretStatus[];
  revision: string;
}

export interface AronSettingsPatch {
  prefs: Partial<AronSettingsPrefs>;
  revision: string;
}

export function isSecretWritableSource(source: string): boolean {
  return source === "aron" || source === "store";
}

export function aronSettingsFieldsByGroup(): Record<
  AronSettingsGroup,
  AronSettingsEntry[]
> {
  const groups: Record<AronSettingsGroup, AronSettingsEntry[]> = {
    Providers: [],
    "Hermes Routing": [],
    "Voice/Wake": [],
    Vision: [],
    "Privacy/History": [],
    Appearance: [],
  };
  for (const key of Object.keys(ARON_SETTINGS_SCHEMA.properties) as AronSettingsKey[]) {
    const field = ARON_SETTINGS_SCHEMA.properties[key];
    const group = ARON_SETTINGS_GROUPS.find((candidate) => candidate === field["x-uiGroup"]);
    if (!group) throw new Error(`Unsupported ARON settings group for ${key}`);
    groups[group].push([key, field]);
  }
  return groups;
}

export function safeSecretMetadata(status: AronSecretStatus): AronSecretStatus {
  return {
    provider: status.provider,
    configured: status.configured,
    entries: status.entries.map(({ id, label, last4, source }) => ({
      id,
      label,
      last4,
      source: typeof source === "string" ? source : "unknown",
    })),
  };
}
