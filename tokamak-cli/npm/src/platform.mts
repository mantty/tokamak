import { createRequire } from "node:module";
import path from "node:path";

/** Resolves a module request to its file, as `require.resolve` does. */
export type Resolve = (request: string) => string;

const { optionalDependencies } = createRequire(import.meta.url)("../package.json") as {
  optionalDependencies: Record<string, string>;
};

export const PLATFORM_PACKS = Object.keys(optionalDependencies).filter((name) => name.startsWith("@tokamakdev/platform-"));

export function packageFor(platform: string, arch: string): string {
  const name = `@tokamakdev/tok-${platform}-${arch}`;
  if (!Object.hasOwn(optionalDependencies, name)) throw new Error(`tokamak does not publish a CLI for ${platform} ${arch}`);
  return name;
}

export function binaryPath(platform: string, arch: string, resolve: Resolve): string {
  const name = packageFor(platform, arch);
  const file = platform === "win32" ? "bin/tok.exe" : "bin/tok";
  try {
    return resolve(`${name}/${file}`);
  } catch {
    throw new Error(`${name} is not installed; reinstall @tokamakdev/tok with optional dependencies enabled`);
  }
}

// npm installs only the platform packs whose os and cpu match this host.
export function platformPackRoots(resolve: Resolve): string[] {
  return PLATFORM_PACKS.flatMap((name) => {
    try {
      return [path.dirname(resolve(`${name}/package.json`))];
    } catch {
      return [];
    }
  });
}
