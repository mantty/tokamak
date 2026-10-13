/** The location plugin's build settings, with their defaults applied. */
export interface Settings {
  background: boolean;
  "background-updates": boolean;
  precision: "precise" | "approximate";
  "when-in-use-description"?: string;
  "always-description"?: string;
  "accuracy-purposes"?: Record<string, string>;
}

export function declarations(
  platform: string,
  settings: Settings,
): { key?: string; name?: string; contents?: string; notices?: string[]; problems?: string[] };
