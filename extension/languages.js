const GF_LANGUAGES = {
  en: "English",
  "zh-TW": "繁體中文",
  "zh-CN": "简体中文",
  ja: "日本語",
  ko: "한국어",
};

function geminiForgeLanguage(tag) {
  const parts = String(tag).toLowerCase().replaceAll("_", "-").split("-");
  if (parts[0] === "zh") {
    if (parts.includes("hant")) return "zh-TW";
    if (parts.includes("hans")) return "zh-CN";
    return parts.some(p => ["tw", "hk", "mo"].includes(p)) ? "zh-TW" : "zh-CN";
  }
  return Object.hasOwn(GF_LANGUAGES, parts[0]) ? parts[0] : "en";
}
