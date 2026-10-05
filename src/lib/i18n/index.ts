import { derived, writable, get } from "svelte/store";
import en, { type TranslationKey } from "./en";
import zhCN from "./zh-CN";

export type { TranslationKey } from "./en";
export type Locale = "en" | "zh-CN";
export type LanguageSetting = "auto" | Locale;
export type TranslationParams = Record<string, string | number>;

const dictionaries: Record<Locale, Record<TranslationKey, string>> = {
  en,
  "zh-CN": zhCN,
};

/** Language options; labels are endonyms and must not be translated. */
export const locales: Array<{ value: Locale; label: string }> = [
  { value: "en", label: "English" },
  { value: "zh-CN", label: "简体中文" },
];

/** Resolves the system language to the closest supported locale. */
function detectLocale(): Locale {
  if (typeof navigator === "undefined") return "en";
  return navigator.language.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

/** User language preference; "auto" follows the system language. */
export const languageSetting = writable<LanguageSetting>("auto");

/** Active locale, resolved from the language setting. */
export const locale = derived(languageSetting, (setting) =>
  setting === "auto" ? detectLocale() : setting,
);

// Keep <html lang> in sync with the resolved locale.
locale.subscribe((current) => {
  if (typeof document !== "undefined") {
    document.documentElement.lang = current;
  }
});

function lookup(
  dict: Record<TranslationKey, string>,
  key: string,
): string | undefined {
  return Object.prototype.hasOwnProperty.call(dict, key)
    ? dict[key as TranslationKey]
    : undefined;
}

function interpolate(template: string, params?: TranslationParams): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name) =>
    Object.prototype.hasOwnProperty.call(params, name)
      ? String(params[name])
      : match,
  );
}

/**
 * Translator derived from the current locale.
 * Falls back to English when a key is missing in the active dictionary,
 * and to the key itself when it is missing everywhere.
 */
export const t = derived(locale, (current) => {
  const dict = dictionaries[current] ?? dictionaries.en;
  return (key: string, params?: TranslationParams): string => {
    const template = lookup(dict, key) ?? lookup(en, key);
    return template === undefined ? key : interpolate(template, params);
  };
});

/**
 * Translator for process statuses reported by the backend.
 * Looks up `status.<lowercased status>` and falls back to the raw
 * backend string when no translation exists.
 */
export const statusLabel = derived(locale, (current) => {
  const dict = dictionaries[current] ?? dictionaries.en;
  return (status: string): string => {
    const key = `status.${status.toLowerCase()}`;
    return lookup(dict, key) ?? lookup(en, key) ?? status;
  };
});

export function setLanguage(next: LanguageSetting) {
  languageSetting.set(next);
}

/** Restores the language setting persisted in AppConfig; call after settingsStore.init(). */
export function initLocale(language: string | undefined) {
  setLanguage(
    language === "auto" || locales.some((l) => l.value === language)
      ? (language as LanguageSetting)
      : "auto",
  );
}

export function getLocale(): Locale {
  return get(locale);
}
