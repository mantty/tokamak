/** The location plugin's build settings, with their defaults applied. */
export interface Settings {
  "background-always": boolean;
  "background-updates": boolean;
  precision: "precise" | "approximate";
  "when-in-use-description"?: string;
  "always-description"?: string;
  "accuracy-purposes"?: Record<string, string>;
}

/**
 * The file that declares what the settings ask for, the plugin section key naming it, the Swift
 * compilation conditions of an Apple build, and notices.
 */
export interface Declarations {
  key: "plist" | "manifest";
  name: string;
  contents: string;
  notices: string[];
  conditions?: string[];
}

export function declarations(platform: string, settings: Settings): Declarations | { problems: string[] };
