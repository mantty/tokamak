"use strict";

const path = require("node:path");
const { optionalDependencies } = require("./package.json");


const PLATFORM_PACKS = Object.keys(optionalDependencies).filter((name) => name.startsWith("@tokamakdev/platform-"));

function packageFor(platform, arch) {
  const name = `@tokamakdev/tok-${platform}-${arch}`;
  if (!Object.hasOwn(optionalDependencies, name)) throw new Error(`tokamak does not publish a CLI for ${platform} ${arch}`);
  return name;
}

function binaryPath(platform, arch, resolve) {
  const name = packageFor(platform, arch);
  const file = platform === "win32" ? "bin/tok.exe" : "bin/tok";
  try {
    return resolve(`${name}/${file}`);
  } catch {
    throw new Error(`${name} is not installed; reinstall @tokamakdev/tok with optional dependencies enabled`);
  }
}

// npm installs only the platform packs whose os and cpu match this host.
function platformPackRoots(resolve) {
  return PLATFORM_PACKS.flatMap((name) => {
    try {
      return [path.dirname(resolve(`${name}/package.json`))];
    } catch {
      return [];
    }
  });
}

module.exports = { PLATFORM_PACKS, packageFor, binaryPath, platformPackRoots };
