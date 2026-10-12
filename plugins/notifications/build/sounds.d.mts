/** A sound file's format, whether Apple plays it in a notification, and its length. */
export interface AppleSound {
  format: string;
  playable: boolean;
  seconds: number;
}

export function appleSound(bytes: Uint8Array): AppleSound | undefined;

export function bundledSounds(
  platform: string,
  directory: string,
): { keys?: Record<string, string[]>; problems?: string[] };
