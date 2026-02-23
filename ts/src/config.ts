import { readFile } from "node:fs/promises";
import YAML from "yaml";
import type { VfsConfig } from "./types.js";

/**
 * Replace ${VAR} and ${VAR:-default} with process.env values.
 */
function interpolateEnv(text: string): string {
  return text.replace(/\$\{([^}]+)\}/g, (_match, expr: string) => {
    const sepIdx = expr.indexOf(":-");
    if (sepIdx !== -1) {
      const varName = expr.slice(0, sepIdx);
      const fallback = expr.slice(sepIdx + 2);
      return process.env[varName] ?? fallback;
    }
    return process.env[expr] ?? "";
  });
}

export async function loadConfig(configPath: string): Promise<VfsConfig> {
  const raw = await readFile(configPath, "utf-8");
  const interpolated = interpolateEnv(raw);
  const parsed = YAML.parse(interpolated);

  return {
    backends: parsed.backends ?? {},
    mounts: parsed.mounts ?? [],
    defaults: parsed.defaults,
  };
}
